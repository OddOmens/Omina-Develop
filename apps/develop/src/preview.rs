//! Rendering the photo for the screen, off the UI thread. Developing a big photo takes a
//! moment, so the viewer never waits for it: it asks for the picture it needs and keeps
//! showing the last one finished until the new one arrives.
//!
//! The original is turned and flipped once, then halved again and again into a pyramid of
//! smaller copies. Each render starts from the smallest copy that is still sharp at the
//! current zoom (an even smaller one while a slider is being dragged, so dragging stays
//! smooth), and is done again sharper when the drag ends.

use std::sync::Arc;

use develop_core::analysis::Histogram;
use develop_core::edit::{Edit, View};
use develop_core::{Bitmap, Photo, Rect, Vec2};
use gpui_kit::{AppContext as _, Context, RenderImage, Window};
use rayon::prelude::*;

use crate::workspace::Workspace;

/// The pyramid stops halving once a copy's longer side is this small.
const SMALLEST_SIDE: u32 = 384;
/// The most pixels a render may have while a slider or handle is being dragged.
const INTERACTIVE_PIXELS: f32 = 2_200_000.0;

/// What a pyramid was made from: which original, turned and flipped how.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OrientKey {
    source: usize,
    quarter_turns: u8,
    flip_horizontal: bool,
    flip_vertical: bool,
}

impl OrientKey {
    pub fn of(photo: &Photo) -> Self {
        let crop = &photo.edit.crop;
        Self { source: source_id(photo), quarter_turns: crop.quarter_turns % 4, flip_horizontal: crop.flip_horizontal, flip_vertical: crop.flip_vertical }
    }
}

/// Which original a photo shows: the same for every edit of it, different for every import
/// (even one opened after another was closed; a memory address could be reused).
pub fn source_id(photo: &Photo) -> usize {
    photo.source.id() as usize
}

/// The oriented photo at full size and halved again and again.
pub struct Pyramid {
    pub key: OrientKey,
    /// The full-size image's size.
    pub size: (u32, u32),
    /// Level 0 is full size; each next level is half the one before.
    pub levels: Vec<Arc<Bitmap>>,
}

impl Pyramid {
    fn build(photo_source: &develop_core::Source, crop: &develop_core::crop::Crop, key: OrientKey) -> Pyramid {
        let full = crop.orient(&photo_source.bitmap);
        let size = (full.width, full.height);
        let mut levels = vec![Arc::new(full)];
        loop {
            let last = levels.last().expect("the pyramid has its full-size level");
            if last.width.max(last.height) <= SMALLEST_SIDE {
                break;
            }
            let half = last.resized(last.width.div_ceil(2), last.height.div_ceil(2));
            levels.push(Arc::new(half));
        }
        Pyramid { key, size, levels }
    }

    /// How big level `level` is compared with full size. Halving rounds each side up, so a
    /// level can be a little more of the photo across than down (a 1-pixel-wide strip stops
    /// shrinking across at all); the smaller of the two is used, so a render from the level
    /// never asks for pixels past its edge.
    pub fn scale(&self, level: usize) -> f32 {
        let bitmap = &self.levels[level.min(self.levels.len() - 1)];
        let across = bitmap.width as f32 / self.size.0.max(1) as f32;
        let down = bitmap.height as f32 / self.size.1.max(1) as f32;
        across.min(down)
    }

    /// The level to render from to show `area` (frame pixels) at `wanted` device pixels per
    /// image pixel: the smallest that is still sharp, or smaller still while `interactive`.
    pub fn level_for(&self, wanted: f32, area: Rect, interactive: bool) -> usize {
        let mut level = (0..self.levels.len()).rev().find(|&level| self.scale(level) >= wanted * 0.98).unwrap_or(0);
        if interactive {
            while level + 1 < self.levels.len() && area.width * area.height * self.scale(level).powi(2) > INTERACTIVE_PIXELS {
                level += 1;
            }
        }
        level
    }

    /// The smallest level whose longer side is at least `side` pixels (or the full size).
    pub fn level_at_least(&self, side: u32) -> usize {
        (0..self.levels.len()).rev().find(|&level| self.levels[level].width.max(self.levels[level].height) >= side).unwrap_or(0)
    }

    /// The photo's color as it came (straight sRGB, 0–1) averaged around an image point (pixels).
    pub fn sample(&self, at: Vec2) -> Option<[f32; 3]> {
        // A level around 1500 pixels wide averages away noise without blurring a small target.
        let level = self.level_at_least(1500);
        let bitmap = &self.levels[level];
        let scale = self.scale(level);
        let (x, y) = ((at.x * scale) as i32, (at.y * scale) as i32);
        if x < 0 || y < 0 || x >= bitmap.width as i32 || y >= bitmap.height as i32 {
            return None;
        }
        let mut sum = [0.0f32; 3];
        let mut count = 0.0;
        for dy in -2..=2 {
            for dx in -2..=2 {
                let pixel = bitmap.pixel(x + dx, y + dy);
                if pixel[3] == 0 {
                    continue;
                }
                for channel in 0..3 {
                    sum[channel] += f32::from(pixel[channel]) / f32::from(pixel[3]);
                }
                count += 1.0;
            }
        }
        (count > 0.0).then(|| sum.map(|value| (value / count).min(1.0)))
    }
}

/// Which picture a render is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    /// The edited photo (or, in Crop mode, the whole frame).
    Main,
    /// The photo as it came, for comparing.
    Before,
}

/// Everything a finished render depends on; a new render is needed when any of it changes.
#[derive(Clone, Debug, PartialEq)]
pub struct RenderKey {
    pub orient: OrientKey,
    pub edit: Edit,
    pub view: View,
    pub level: usize,
    /// The color under transparent parts (the straightened frame's corners in Crop mode).
    pub backdrop: u32,
    /// Whether clipped highlights and shadows are painted red and blue (Lightroom's clipping
    /// warning).
    pub clipping: bool,
}

/// A finished render.
pub struct Rendered {
    pub key: RenderKey,
    pub image: Arc<RenderImage>,
    /// The frame area the picture shows.
    pub area: Rect,
}

/// The viewer's renders and the pyramids they come from: one for each open photo, so going
/// back to a photo's tab shows it again without turning and halving it all over.
#[derive(Default)]
pub struct Preview {
    pyramids: Vec<Arc<Pyramid>>,
    building: Option<OrientKey>,
    main: Option<Rendered>,
    before: Option<Rendered>,
    in_flight: [bool; 2],
    /// The finished photo's histogram, from the latest render of it, and which photo it was.
    histogram: Option<(usize, Arc<Histogram>)>,
}

impl Preview {
    pub fn shown(&self, slot: Slot) -> Option<&Rendered> {
        match slot {
            Slot::Main => self.main.as_ref(),
            Slot::Before => self.before.as_ref(),
        }
    }

    /// The pyramid for `photo` as it is turned now, once it is ready.
    pub fn pyramid_of(&self, photo: &Photo) -> Option<Arc<Pyramid>> {
        let key = OrientKey::of(photo);
        self.pyramids.iter().find(|pyramid| pyramid.key == key).cloned()
    }

    /// The histogram of `photo`'s latest render, if there is one yet.
    pub fn histogram_of(&self, photo: &Photo) -> Option<Arc<Histogram>> {
        self.histogram.as_ref().filter(|(source, _)| *source == source_id(photo)).map(|(_, histogram)| histogram.clone())
    }

    /// Lets go of everything kept for the photo with original `source` (when it closes),
    /// returning its pictures so their textures can be freed.
    pub fn release(&mut self, source: usize) -> Vec<Arc<RenderImage>> {
        self.pyramids.retain(|pyramid| pyramid.key.source != source);
        if self.building.is_some_and(|key| key.source == source) {
            self.building = None;
        }
        if self.histogram.as_ref().is_some_and(|(of, _)| *of == source) {
            self.histogram = None;
        }
        let mut released = Vec::new();
        for slot in [&mut self.main, &mut self.before] {
            if slot.as_ref().is_some_and(|rendered| rendered.key.orient.source == source) {
                released.extend(slot.take().map(|rendered| rendered.image));
            }
        }
        released
    }
}

/// The pyramid for the open photo, starting to build it if needed (`None` until it's ready).
pub fn pyramid(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) -> Option<Arc<Pyramid>> {
    let photo = workspace.state.photo()?;
    let key = OrientKey::of(photo);
    if let Some(pyramid) = workspace.ui.preview.pyramid_of(photo) {
        return Some(pyramid);
    }
    if workspace.ui.preview.building != Some(key) {
        workspace.ui.preview.building = Some(key);
        let source = photo.source.clone();
        let crop = photo.edit.crop.clone();
        cx.spawn_in(window, async move |this, cx| {
            let pyramid = cx.background_spawn(async move { Pyramid::build(&source, &crop, key) }).await;
            this.update(cx, |this, cx| {
                // A newer orientation may have been asked for meanwhile; then this one is dropped.
                let preview = &mut this.ui.preview;
                if preview.building == Some(key) {
                    preview.building = None;
                    // One pyramid per photo: one for an earlier turn of it isn't needed now.
                    preview.pyramids.retain(|kept| kept.key.source != key.source);
                    preview.pyramids.push(Arc::new(pyramid));
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }
    None
}

/// Asks for `view` rendered from pyramid level `level` into `slot`, unless that is what it
/// already shows. Only one render per slot runs at a time; when it finishes, the viewer draws
/// again and asks for whatever is newest by then.
pub fn request(workspace: &mut Workspace, slot: Slot, view: View, level: usize, backdrop: u32, window: &mut Window, cx: &mut Context<Workspace>) {
    let clipping = workspace.ui.show_clipping && slot == Slot::Main && view == View::Result;
    let Some(photo) = workspace.state.photo() else { return };
    let Some(pyramid) = workspace.ui.preview.pyramid_of(photo) else { return };
    let level = level.min(pyramid.levels.len() - 1);
    // The Before picture never depends on the Develop settings or masks, so changing them
    // doesn't render it again.
    let edit = match view {
        View::Before => Edit { crop: photo.edit.crop.clone(), ..Edit::default() },
        // While a preset or a fix is hovered, the photo shows what it would do.
        _ => workspace.ui.hover_edit.as_ref().filter(|(source, _)| *source == source_id(photo)).map_or_else(|| photo.edit.clone(), |(_, edit)| edit.clone()),
    };
    let key = RenderKey { orient: pyramid.key, edit, view, level, backdrop, clipping };
    let index = slot as usize;
    if workspace.ui.preview.shown(slot).is_some_and(|shown| shown.key == key) || workspace.ui.preview.in_flight[index] {
        return;
    }
    workspace.ui.preview.in_flight[index] = true;
    let measure = slot == Slot::Main && view == View::Result;
    cx.spawn_in(window, async move |this, cx| {
        let job_key = key.clone();
        let (image, area, histogram) = cx
            .background_spawn(async move {
                let key = job_key;
                let source = &pyramid.levels[key.level];
                let area = key.edit.area(key.view, pyramid.size);
                let mut bitmap = key.edit.render(source, pyramid.size, pyramid.scale(key.level), key.view);
                let histogram = measure.then(|| Arc::new(Histogram::of(&bitmap)));
                if key.clipping {
                    paint_clipping(&mut bitmap);
                }
                (to_render_image(&bitmap, key.backdrop), area, histogram)
            })
            .await;
        this.update_in(cx, |this, window, cx| {
            let preview = &mut this.ui.preview;
            preview.in_flight[index] = false;
            // A render for a photo since closed or turned is thrown away.
            let current = this.state.photo().map(OrientKey::of);
            if current != Some(key.orient) {
                cx.drop_image(image, Some(window));
                cx.notify();
                return;
            }
            if let Some(histogram) = histogram {
                preview.histogram = Some((key.orient.source, histogram));
            }
            let rendered = Rendered { key, image, area };
            let previous = match slot {
                Slot::Main => preview.main.replace(rendered),
                Slot::Before => preview.before.replace(rendered),
            };
            // Let go of the previous picture's texture, or every render would stay in GPU memory.
            if let Some(previous) = previous {
                cx.drop_image(previous.image, Some(window));
            }
            cx.notify();
        })
        .ok();
    })
    .detach();
}

/// Paints pixels clipped to white red and pixels clipped to black blue, as Lightroom's clipping
/// warning does, so it's plain where detail is lost.
pub fn paint_clipping(bitmap: &mut Bitmap) {
    bitmap.data.par_chunks_exact_mut(4).for_each(|pixel| {
        if pixel[3] < 250 {
            return;
        }
        if pixel[..3].iter().any(|channel| *channel >= 253) {
            pixel[..3].copy_from_slice(&[255, 40, 40]);
        } else if pixel[..3].iter().all(|channel| *channel <= 2) {
            pixel[..3].copy_from_slice(&[40, 110, 255]);
        }
    });
}

/// A rendered photo (premultiplied RGBA) as GPUI wants it: BGRA, with transparent parts
/// filled with `backdrop` (0xRRGGBB).
pub fn to_render_image(bitmap: &Bitmap, backdrop: u32) -> Arc<RenderImage> {
    let under = [(backdrop >> 16) & 0xff, (backdrop >> 8) & 0xff, backdrop & 0xff];
    let mut bgra = vec![0u8; bitmap.data.len()];
    bgra.par_chunks_exact_mut(4).zip(bitmap.data.par_chunks_exact(4)).for_each(|(out, pixel)| {
        let keep = 255 - u32::from(pixel[3]);
        let channel = |index: usize| (u32::from(pixel[index]) + develop_core::bitmap::mul_div_255(under[index], keep)).min(255) as u8;
        out[0] = channel(2);
        out[1] = channel(1);
        out[2] = channel(0);
        out[3] = 255;
    });
    let buffer = image::ImageBuffer::<image::Rgba<u8>, _>::from_raw(bitmap.width.max(1), bitmap.height.max(1), bgra).unwrap_or_else(|| image::ImageBuffer::new(1, 1));
    Arc::new(RenderImage::new(vec![image::Frame::new(buffer)]))
}

/// A small render of the finished photo with `edit`, at least `side` pixels on its longer side,
/// made now (for the project file's preview and the preset thumbnails).
pub fn render_small(pyramid: &Pyramid, edit: &Edit, side: u32) -> Bitmap {
    let level = pyramid.level_at_least(side);
    edit.render(&pyramid.levels[level], pyramid.size, pyramid.scale(level), View::Result)
}

/// Builds pyramids for tests elsewhere in the app.
#[cfg(test)]
pub mod tests_support {
    use std::sync::Arc;

    use super::{OrientKey, Pyramid};

    /// The pyramid for `photo` as it is turned now.
    pub fn pyramid(photo: &develop_core::Photo) -> Arc<Pyramid> {
        Arc::new(Pyramid::build(&photo.source, &photo.edit.crop, OrientKey::of(photo)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use develop_core::Rgba;

    #[test]
    fn the_pyramid_halves_down_to_a_small_copy() {
        let photo = crate::state::test_support::photo();
        let big = develop_core::Source::new("Big.jpg", Vec::new(), Bitmap::filled(3000, 2000, Rgba::rgb(0.5, 0.5, 0.5)));
        let mut crop = develop_core::crop::Crop::default();
        crop.turn(true, (3000, 2000));
        let pyramid = Pyramid::build(&big, &crop, OrientKey::of(&photo));
        assert_eq!(pyramid.size, (2000, 3000));
        assert_eq!(pyramid.levels.len(), 4);
        assert_eq!((pyramid.levels[3].width, pyramid.levels[3].height), (250, 375));
        // Zoomed out to a quarter, the quarter-size copy is sharp enough.
        let whole = Rect::new(0.0, 0.0, 2000.0, 3000.0);
        assert_eq!(pyramid.level_for(0.25, whole, false), 2);
        assert_eq!(pyramid.level_for(2.0, whole, false), 0);
        // While dragging, the full size is too much for a whole 6-megapixel photo.
        assert!(pyramid.level_for(1.0, whole, true) >= 1);
        assert_eq!(pyramid.level_at_least(400), 2);
    }

    #[test]
    fn a_narrow_strips_small_copies_fill_their_whole_preview() {
        // Halving a 2 × 1000 strip stops shrinking its width at 1 pixel, so the levels are not
        // the same fraction of the photo across as down.
        let photo = crate::state::test_support::photo();
        let strip = develop_core::Source::new("Strip.png", Vec::new(), Bitmap::filled(2, 1000, Rgba::rgb(0.5, 0.5, 0.5)));
        let pyramid = Pyramid::build(&strip, &develop_core::crop::Crop::default(), OrientKey::of(&photo));
        for level in 0..pyramid.levels.len() {
            let out = Edit::default().render(&pyramid.levels[level], pyramid.size, pyramid.scale(level), View::Result);
            assert_eq!(out.pixel(0, out.height as i32 - 1)[3], 255, "level {level}: the foot of the strip is drawn");
        }
    }

    #[test]
    fn clipped_pixels_are_painted_red_and_blue() {
        let mut bitmap = Bitmap::new(3, 1);
        bitmap.set_pixel(0, 0, [255, 200, 100, 255]);
        bitmap.set_pixel(1, 0, [0, 1, 2, 255]);
        bitmap.set_pixel(2, 0, [120, 120, 120, 255]);
        paint_clipping(&mut bitmap);
        assert_eq!(bitmap.pixel(0, 0), [255, 40, 40, 255]);
        assert_eq!(bitmap.pixel(1, 0), [40, 110, 255, 255]);
        assert_eq!(bitmap.pixel(2, 0), [120, 120, 120, 255]);
    }

    #[test]
    fn colors_are_sampled_straight() {
        let photo = crate::state::test_support::photo();
        let pyramid = Pyramid::build(&photo.source, &photo.edit.crop, OrientKey::of(&photo));
        let color = pyramid.sample(Vec2::new(100.0, 50.0)).unwrap();
        assert!((color[0] - 100.0 / 255.0).abs() < 0.02 && (color[2] - 128.0 / 255.0).abs() < 0.01, "{color:?}");
        assert!(pyramid.sample(Vec2::new(-5.0, 3.0)).is_none());
    }
}

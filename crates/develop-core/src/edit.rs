//! A photo's edit (every Develop setting, the crop and the masks) and the rendering that
//! turns the original into the result. Rendering always starts from the untouched original, so
//! edits never pile up losses.
//!
//! The steps, in order:
//! 1. turn and flip the original (`Crop::orient`), done once by the caller and reused;
//! 2. develop the whole image (everything but vignette and grain);
//! 3. apply the masks;
//! 4. straighten and cut out the crop;
//! 5. add the vignette and grain, centered on the cropped picture.

use serde::{Deserialize, Serialize};

use crate::bitmap::Bitmap;
use crate::crop::Crop;
use crate::develop::{Develop, apply_effects, develop_without_effects};
use crate::geom::Rect;
use crate::masks::{Mask, apply_masks};

/// Everything done to a photo.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Edit {
    pub develop: Develop,
    pub crop: Crop,
    pub masks: Vec<Mask>,
}

/// What part of the edit a render shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum View {
    /// The finished photo: developed, masked, cropped, with effects.
    Result,
    /// The photo as it came, cut to the same crop, for comparing before and after.
    Before,
    /// Developed and masked but not cropped: the whole straightened frame, for Crop mode.
    Uncropped,
}

impl Edit {
    /// Whether the edit leaves the photo as it was.
    pub fn is_neutral(&self) -> bool {
        self.develop.is_neutral() && self.crop.is_neutral() && self.masks.iter().all(|mask| !mask.is_active())
    }

    /// The area of the frame a view shows, for an image of `size` (see `crop` for the spaces).
    pub fn area(&self, view: View, size: (u32, u32)) -> Rect {
        match view {
            View::Result | View::Before => self.crop.kept(size),
            View::Uncropped => self.crop.frame_bounds(size),
        }
    }

    /// Renders `view` from `image`: the original already turned and flipped by this edit's crop
    /// (`Crop::orient`), at `scale` times its full size. `size` is the full-size image's size.
    /// The result is the view's area at `scale`.
    pub fn render(&self, image: &Bitmap, size: (u32, u32), scale: f32, view: View) -> Bitmap {
        let scale = scale.max(1e-4);
        let developed = match view {
            View::Before => image.clone(),
            View::Result | View::Uncropped => {
                let developed = develop_without_effects(image, &self.develop, scale);
                apply_masks(&developed, &self.masks, scale)
            }
        };
        let mut out = self.crop.cut(&developed, size, self.area(view, size), scale);
        if view == View::Result {
            apply_effects(&mut out, &self.develop, scale);
        }
        out
    }

    /// The finished photo at full size, from the original as decoded.
    pub fn render_full(&self, original: &Bitmap) -> Bitmap {
        let image = self.crop.orient(original);
        let size = (image.width, image.height);
        self.render(&image, size, 1.0, View::Result)
    }

    /// The finished photo's full size in pixels.
    pub fn output_size(&self, original: (u32, u32)) -> (u32, u32) {
        let size = self.crop.oriented_size(original.0, original.1);
        let kept = self.crop.kept(size);
        (kept.width.round().max(1.0) as u32, kept.height.round().max(1.0) as u32)
    }

    /// Turns the photo a quarter turn (clockwise or not). The crop and the masks move with it,
    /// so both stay on the same part of the photo. `size` is the image's size before turning.
    pub fn turn(&mut self, clockwise: bool, size: (u32, u32)) {
        self.crop.turn(clockwise, size);
        for mask in &mut self.masks {
            mask.shape = mask.shape.turned(clockwise);
        }
    }

    /// Mirrors the photo left to right (`horizontal`) or top to bottom, with its crop and masks.
    pub fn flip(&mut self, horizontal: bool, size: (u32, u32)) {
        self.crop.flip(horizontal, size);
        for mask in &mut self.masks {
            mask.shape = mask.shape.flipped(horizontal);
        }
    }

    /// The settings that "copy settings" carries from one photo to another: the Develop settings
    /// and masks, but not the crop, which belongs to each photo's own framing.
    pub fn look(&self) -> Edit {
        Edit { develop: self.develop.clone(), crop: Crop::default(), masks: self.masks.clone() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Rgba;
    use crate::geom::Vec2;
    use crate::masks::MaskShape;

    fn photo() -> Bitmap {
        let mut bitmap = Bitmap::new(60, 40);
        for y in 0..40 {
            for x in 0..60 {
                bitmap.set_pixel(x, y, [(x * 4) as u8, (y * 6) as u8, 120, 255]);
            }
        }
        bitmap
    }

    #[test]
    fn a_neutral_edit_gives_back_the_photo() {
        let original = photo();
        assert!(Edit::default().is_neutral());
        assert_eq!(Edit::default().render_full(&original), original);
    }

    #[test]
    fn the_result_is_the_crop_at_the_asked_scale() {
        let original = photo();
        let mut edit = Edit::default();
        edit.crop.rect = Some(Rect::new(10.0, 10.0, 20.0, 10.0));
        edit.develop.exposure = 0.5;
        edit.develop.vignette = -40.0;
        let full = edit.render_full(&original);
        assert_eq!((full.width, full.height), (20, 10));
        assert_eq!(edit.output_size((60, 40)), (20, 10));
        let preview = edit.render(&original.resized(30, 20), (60, 40), 0.5, View::Result);
        assert_eq!((preview.width, preview.height), (10, 5));
        // Before shows the same crop, undeveloped.
        let before = edit.render(&original, (60, 40), 1.0, View::Before);
        assert_eq!(before.pixel(0, 0), original.pixel(10, 10));
        // Crop mode shows the whole frame.
        let whole = edit.render(&original, (60, 40), 1.0, View::Uncropped);
        assert_eq!((whole.width, whole.height), (60, 40));
    }

    #[test]
    fn turning_the_photo_turns_the_result() {
        let original = Bitmap::filled(30, 10, Rgba::rgb(0.5, 0.5, 0.5));
        let mut edit = Edit::default();
        edit.crop.turn(true, (30, 10));
        let out = edit.render_full(&original);
        assert_eq!((out.width, out.height), (10, 30));
    }

    #[test]
    fn masks_stay_on_the_same_part_of_the_photo_when_it_turns_or_flips() {
        // A dark ellipse, tilted, over the left of a 40 × 20 gray photo.
        let original = Bitmap::filled(40, 20, Rgba::rgb(0.5, 0.5, 0.5));
        let mut edit = Edit::default();
        let mut mask = Mask::new("Dark", MaskShape::Radial { center: Vec2::new(0.2, 0.5), radius: Vec2::new(0.15, 0.3), angle: 20.0, feather: 0.0 });
        mask.settings.exposure = -4.0;
        edit.masks.push(mask);
        let dark = |out: &Bitmap, x: i32, y: i32| out.pixel(x, y)[0] < 60;
        let first = edit.render_full(&original);
        assert!(dark(&first, 8, 10) && !dark(&first, 32, 10));
        // Turned clockwise, the photo's point (x, y) is drawn at (20 - y, x) of a 20 × 40 picture.
        let mut turned = edit.clone();
        turned.turn(true, (40, 20));
        let out = turned.render_full(&original);
        assert!(dark(&out, 10, 8) && !dark(&out, 10, 32), "the mask turned with the photo");
        // And every turn and flip lands back where it started after a full round.
        let mut round = edit.clone();
        for _ in 0..4 {
            let size = round.crop.oriented_size(40, 20);
            round.turn(false, size);
        }
        round.flip(true, (40, 20));
        round.flip(true, (40, 20));
        round.flip(false, (40, 20));
        let mirrored = round.render_full(&original);
        assert!(dark(&mirrored, 8, 10), "a vertical flip keeps a mask on the left on the left");
        round.flip(false, (40, 20));
        let MaskShape::Radial { center, radius, angle, .. } = round.masks[0].shape else { panic!("still radial") };
        assert!(center.distance(Vec2::new(0.2, 0.5)) < 1e-5 && radius == Vec2::new(0.15, 0.3) && angle == 20.0, "back where it began");
    }

    #[test]
    fn copied_looks_leave_the_crop_behind() {
        let mut edit = Edit::default();
        edit.develop.contrast = 20.0;
        edit.crop.angle = 3.0;
        edit.masks.push(Mask::new("Sky", MaskShape::linear(Vec2::new(0.5, 0.0), Vec2::new(0.5, 0.5))));
        let look = edit.look();
        assert_eq!(look.develop.contrast, 20.0);
        assert_eq!(look.masks.len(), 1);
        assert!(look.crop.is_neutral());
    }
}

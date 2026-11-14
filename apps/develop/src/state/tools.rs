//! What the tools do with presses and drags on the photo: Crop's handles, moving the crop and
//! straightening by dragging outside it; the Masks tool's new gradients and their handles;
//! and the White Balance Selector's click. The viewer turns the pointer into frame points
//! (see `develop_core::crop` for the spaces) and calls `press`, `drag` and `release`.

use develop_core::Rect;
use develop_core::crop::MIN_SIDE;
use develop_core::geom::Vec2;
use develop_core::masks::MaskShape;

use super::{EditorState, MaskKind};

/// The tools in the top bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    /// Looking: drag to pan, click to zoom.
    View,
    /// Crop & Straighten.
    Crop,
    /// Click a neutral spot to set the white balance.
    WhiteBalance,
    /// Make and move masks.
    Masks,
}

/// The keys held during a drag.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Keys {
    pub shift: bool,
    pub alt: bool,
}

/// Which part of the crop a press took hold of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CropGrip {
    /// A corner: 0 top-left, 1 top-right, 2 bottom-right, 3 bottom-left.
    Corner(usize),
    /// An edge's middle: 0 top, 1 right, 2 bottom, 3 left.
    Edge(usize),
    /// Inside the crop: moves it.
    Inside,
    /// Outside the crop: turns the photo to straighten it.
    Outside,
}

/// Which part of a mask a press took hold of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaskGrip {
    /// Moves the whole mask.
    Move,
    /// A linear gradient's full-strength end.
    Start,
    /// A linear gradient's fade-out end.
    End,
    /// A radial gradient's side: its width (`true`) or height, on either side.
    Radius(bool),
}

/// A drag in progress on the photo.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Interaction {
    /// Cropping: what was grabbed, where (frame point), and the crop and angle at the press.
    Crop { grip: CropGrip, press: Vec2, rect: Rect, angle: f32, automatic: bool },
    /// Drawing a new mask from `press` (image pixels); `made` once it is big enough to add.
    NewMask { press: Vec2, made: Option<usize> },
    /// Changing mask `index` by `grip`, from `press` (image pixels) and its shape then.
    Mask { index: usize, grip: MaskGrip, press: Vec2, shape: MaskShape },
}

/// A point's distance from a segment.
fn distance_to_segment(point: Vec2, a: Vec2, b: Vec2) -> f32 {
    let along = b - a;
    let length_squared = along.dot(along);
    if length_squared < 1e-9 {
        return point.distance(a);
    }
    let t = ((point - a).dot(along) / length_squared).clamp(0.0, 1.0);
    point.distance(a + along * t)
}

/// A radial gradient's four side handles, in image pixels: right, bottom, left, top.
pub fn radial_handles(center: Vec2, radius: Vec2, angle: f32) -> [Vec2; 4] {
    let (sin, cos) = angle.to_radians().sin_cos();
    let along = |x: f32, y: f32| center + Vec2::new(x * cos - y * sin, x * sin + y * cos);
    [along(radius.x, 0.0), along(0.0, radius.y), along(-radius.x, 0.0), along(0.0, -radius.y)]
}

/// The point a mask is grabbed by to move or select it, in image fractions.
pub fn mask_pin(shape: &MaskShape) -> Vec2 {
    match *shape {
        MaskShape::Linear { start, end } => start.lerp(end, 0.5),
        MaskShape::Radial { center, .. } => center,
    }
}

impl EditorState {
    /// A frame point as an image point (pixels).
    pub fn frame_to_image(&self, point: Vec2) -> Option<Vec2> {
        let size = self.image_size()?;
        Some(self.edit_settings()?.crop.frame_to_image(size).apply(point))
    }

    /// An image point (pixels) as a frame point.
    pub fn image_to_frame(&self, point: Vec2) -> Option<Vec2> {
        let size = self.image_size()?;
        Some(self.edit_settings()?.crop.image_to_frame(size).apply(point))
    }

    /// Image pixels as fractions of the image.
    fn to_fraction(&self, point: Vec2) -> Option<Vec2> {
        let (width, height) = self.image_size()?;
        Some(Vec2::new(point.x / width.max(1) as f32, point.y / height.max(1) as f32))
    }

    /// The part of the crop at `point` (frame), within `tolerance` frame pixels of a handle.
    pub fn crop_grip_at(&self, point: Vec2, tolerance: f32) -> Option<CropGrip> {
        let size = self.image_size()?;
        let rect = self.edit_settings()?.crop.kept(size);
        let corners = rect.corners();
        if let Some(index) = corners.iter().position(|corner| corner.distance(point) <= tolerance) {
            return Some(CropGrip::Corner(index));
        }
        let edges = [corners[0].lerp(corners[1], 0.5), corners[1].lerp(corners[2], 0.5), corners[2].lerp(corners[3], 0.5), corners[3].lerp(corners[0], 0.5)];
        if let Some(index) = edges.iter().position(|edge| edge.distance(point) <= tolerance) {
            return Some(CropGrip::Edge(index));
        }
        // Anywhere along an edge drags that edge, as Lightroom's does.
        for index in 0..4 {
            if distance_to_segment(point, corners[index], corners[(index + 1) % 4]) <= tolerance * 0.6 {
                return Some(CropGrip::Edge(index));
            }
        }
        Some(if rect.contains(point) { CropGrip::Inside } else { CropGrip::Outside })
    }

    /// The mask handle at `point` (image pixels): the selected mask's handles first, then any
    /// mask's pin.
    pub fn mask_grip_at(&self, point: Vec2, tolerance: f32) -> Option<(usize, MaskGrip)> {
        let (width, height) = self.image_size()?;
        let size = Vec2::new(width as f32, height as f32);
        let pixels = |fraction: Vec2| Vec2::new(fraction.x * size.x, fraction.y * size.y);
        let masks = &self.edit_settings()?.masks;
        if let Some(index) = self.selected_mask.filter(|index| *index < masks.len()) {
            match masks[index].shape {
                MaskShape::Linear { start, end } => {
                    if pixels(start).distance(point) <= tolerance {
                        return Some((index, MaskGrip::Start));
                    }
                    if pixels(end).distance(point) <= tolerance {
                        return Some((index, MaskGrip::End));
                    }
                }
                MaskShape::Radial { center, radius, angle, .. } => {
                    let handles = radial_handles(pixels(center), pixels(radius), angle);
                    if let Some(side) = handles.iter().position(|handle| handle.distance(point) <= tolerance) {
                        return Some((index, MaskGrip::Radius(side % 2 == 0)));
                    }
                }
            }
        }
        // Pins: the selected mask's first, so overlapping masks pick the one being worked on.
        let order = self.selected_mask.into_iter().chain(0..masks.len());
        for index in order.filter(|index| *index < masks.len()) {
            if pixels(mask_pin(&masks[index].shape)).distance(point) <= tolerance {
                return Some((index, MaskGrip::Move));
            }
        }
        None
    }

    /// A press on the photo at `point` (frame), with `tolerance` frame pixels to grab handles.
    /// Returns whether the tool took it (otherwise the viewer pans).
    pub fn press(&mut self, point: Vec2, tolerance: f32) -> bool {
        self.interaction = None;
        let Some(size) = self.image_size() else { return false };
        match self.tool {
            Tool::View | Tool::WhiteBalance => false,
            Tool::Crop => {
                let Some(grip) = self.crop_grip_at(point, tolerance) else { return false };
                let crop = &self.edit_settings().expect("a photo is open").crop;
                self.interaction = Some(Interaction::Crop { grip, press: point, rect: crop.kept(size), angle: crop.angle, automatic: crop.rect.is_none() });
                true
            }
            Tool::Masks => {
                let Some(at) = self.frame_to_image(point) else { return false };
                if let Some((index, grip)) = self.mask_grip_at(at, tolerance) {
                    let shape = self.edit_settings().expect("a photo is open").masks[index].shape;
                    self.selected_mask = Some(index);
                    self.interaction = Some(Interaction::Mask { index, grip, press: at, shape });
                } else {
                    self.interaction = Some(Interaction::NewMask { press: at, made: None });
                }
                true
            }
        }
    }

    /// The pointer moved to `point` (frame) during a press.
    pub fn drag(&mut self, point: Vec2, keys: Keys, tolerance: f32) {
        let Some(interaction) = self.interaction else { return };
        match interaction {
            Interaction::Crop { grip, press, rect, angle, automatic } => self.drag_crop(grip, press, rect, angle, automatic, point, keys),
            Interaction::NewMask { press, made } => {
                let Some(at) = self.frame_to_image(point) else { return };
                let index = match made {
                    Some(index) => index,
                    // A click (or a tiny wobble) shouldn't leave a mask behind.
                    None if at.distance(press) < tolerance => return,
                    None => {
                        let Some(placeholder) = self.new_mask_shape(press, at, keys) else { return };
                        let Some(index) = self.add_mask(placeholder) else {
                            self.interaction = None;
                            return;
                        };
                        self.interaction = Some(Interaction::NewMask { press, made: Some(index) });
                        index
                    }
                };
                if let Some(shape) = self.new_mask_shape(press, at, keys) {
                    self.edit_mask(index, "New Mask", |mask| mask.shape = shape);
                }
            }
            Interaction::Mask { index, grip, press, shape } => {
                let Some(at) = self.frame_to_image(point) else { return };
                if let Some(shape) = self.dragged_mask_shape(shape, grip, press, at, keys) {
                    self.edit_mask(index, "Move Mask", |mask| mask.shape = shape);
                }
            }
        }
    }

    /// The press ended.
    pub fn release(&mut self) {
        // A click on an empty part of the photo with the Masks tool lets go of the selected mask.
        if let Some(Interaction::NewMask { made: None, .. }) = self.interaction {
            self.selected_mask = None;
        }
        self.interaction = None;
    }

    /// A new mask's shape for a drag from `from` to `to` (image pixels).
    fn new_mask_shape(&self, from: Vec2, to: Vec2, keys: Keys) -> Option<MaskShape> {
        let start = self.to_fraction(from)?;
        let end = self.to_fraction(to)?;
        Some(match self.mask_kind {
            MaskKind::Linear => MaskShape::linear(start, end),
            MaskKind::Radial => {
                let (width, height) = self.image_size()?;
                let mut reach = Vec2::new((to.x - from.x).abs(), (to.y - from.y).abs());
                // Shift draws a circle.
                if keys.shift {
                    let side = reach.x.max(reach.y);
                    reach = Vec2::new(side, side);
                }
                MaskShape::radial(start, Vec2::new(reach.x / width.max(1) as f32, reach.y / height.max(1) as f32))
            }
        })
    }

    /// `shape` changed by dragging `grip` from `press` to `at` (image pixels).
    fn dragged_mask_shape(&self, shape: MaskShape, grip: MaskGrip, press: Vec2, at: Vec2, keys: Keys) -> Option<MaskShape> {
        let (width, height) = self.image_size()?;
        let size = Vec2::new(width as f32, height as f32);
        let moved = self.to_fraction(at)? - self.to_fraction(press)?;
        Some(match (shape, grip) {
            (MaskShape::Linear { start, end }, MaskGrip::Move) => MaskShape::Linear { start: start + moved, end: end + moved },
            (MaskShape::Radial { center, radius, angle, feather }, MaskGrip::Move) => MaskShape::Radial { center: center + moved, radius, angle, feather },
            (MaskShape::Linear { end, .. }, MaskGrip::Start) => MaskShape::Linear { start: self.to_fraction(at)?, end: if keys.alt { end - moved } else { end } },
            (MaskShape::Linear { start, .. }, MaskGrip::End) => MaskShape::Linear { start: if keys.alt { start - moved } else { start }, end: self.to_fraction(at)? },
            (MaskShape::Radial { center, radius, angle, feather }, MaskGrip::Radius(wide)) => {
                // The pointer's offset from the middle, in the ellipse's own turned axes.
                let offset = at - Vec2::new(center.x * size.x, center.y * size.y);
                let (sin, cos) = (-angle).to_radians().sin_cos();
                let turned = Vec2::new(offset.x * cos - offset.y * sin, offset.x * sin + offset.y * cos);
                let mut reach = Vec2::new(radius.x * size.x, radius.y * size.y);
                if wide {
                    reach.x = turned.x.abs();
                } else {
                    reach.y = turned.y.abs();
                }
                // Shift keeps it a circle.
                if keys.shift {
                    let side = if wide { reach.x } else { reach.y };
                    reach = Vec2::new(side, side);
                }
                let radius = Vec2::new((reach.x / size.x).max(0.005), (reach.y / size.y).max(0.005));
                MaskShape::Radial { center, radius, angle, feather }
            }
            (shape, _) => shape,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn drag_crop(&mut self, grip: CropGrip, press: Vec2, rect: Rect, angle: f32, automatic: bool, point: Vec2, keys: Keys) {
        let Some(size) = self.image_size() else { return };
        let Some(crop) = self.edit_settings().map(|edit| edit.crop.clone()) else { return };
        match grip {
            CropGrip::Outside => {
                // Turning around the crop's middle straightens the photo the same way.
                let center = rect.center();
                let turned = |at: Vec2| (at.y - center.y).atan2(at.x - center.x).to_degrees();
                let mut delta = turned(point) - turned(press);
                delta = (delta + 540.0).rem_euclid(360.0) - 180.0;
                let mut degrees = angle + delta;
                // Shift turns in whole degrees.
                if keys.shift {
                    degrees = degrees.round();
                }
                self.edit("Straighten", |photo| {
                    let crop = &mut photo.edit.crop;
                    crop.rect = (!automatic).then_some(rect);
                    crop.set_angle(degrees, size);
                });
            }
            CropGrip::Inside => {
                let offset = point - press;
                let target = Rect::new(rect.x + offset.x, rect.y + offset.y, rect.width, rect.height);
                let fitted = furthest_fitting(rect, target, |candidate| crop.fits(candidate, size));
                self.edit("Crop", |photo| photo.edit.crop.rect = Some(fitted));
            }
            CropGrip::Corner(_) | CropGrip::Edge(_) => {
                let ratio = if keys.shift && self.crop_aspect == 0 { Some(rect.width / rect.height.max(1e-3)) } else { self.held_ratio() };
                let target = resized_crop(rect, grip, point, ratio);
                let fits = |candidate: Rect| crop.fits(candidate, size);
                let mut fitted = furthest_fitting(rect, target, fits);
                if ratio.is_none() {
                    // Free crops then grow each way on its own as far as the photo allows, so a
                    // corner dragged off the photo still reaches both of its edges.
                    fitted = furthest_fitting(fitted, Rect { x: target.x, width: target.width, ..fitted }, fits);
                    fitted = furthest_fitting(fitted, Rect { y: target.y, height: target.height, ..fitted }, fits);
                }
                self.edit("Crop", |photo| photo.edit.crop.rect = Some(fitted));
            }
        }
    }
}

/// The rectangle part of the way from `from` toward `to` that is furthest along and still
/// passes `fits` (`from` itself if nothing further does).
fn furthest_fitting(from: Rect, to: Rect, fits: impl Fn(Rect) -> bool) -> Rect {
    let lerp = |t: f32| Rect::new(from.x + (to.x - from.x) * t, from.y + (to.y - from.y) * t, from.width + (to.width - from.width) * t, from.height + (to.height - from.height) * t);
    if fits(to) {
        return to;
    }
    let (mut low, mut high) = (0.0f32, 1.0f32);
    for _ in 0..24 {
        let middle = (low + high) * 0.5;
        if fits(lerp(middle)) { low = middle } else { high = middle }
    }
    lerp(low)
}

/// `rect` with the corner or edge `grip` dragged to `point`, the opposite side staying put.
/// With `ratio` (width over height) the shape is held.
fn resized_crop(rect: Rect, grip: CropGrip, point: Vec2, ratio: Option<f32>) -> Rect {
    let (left, top, right, bottom) = (rect.x, rect.y, rect.x + rect.width, rect.y + rect.height);
    match grip {
        CropGrip::Corner(index) => {
            // The opposite corner stays where it is.
            let anchor = [Vec2::new(right, bottom), Vec2::new(left, bottom), Vec2::new(left, top), Vec2::new(right, top)][index];
            let direction = [Vec2::new(-1.0, -1.0), Vec2::new(1.0, -1.0), Vec2::new(1.0, 1.0), Vec2::new(-1.0, 1.0)][index];
            let mut width = ((point.x - anchor.x) * direction.x).max(MIN_SIDE);
            let mut height = ((point.y - anchor.y) * direction.y).max(MIN_SIDE);
            if let Some(ratio) = ratio {
                // The side dragged further sets the size; the other follows the ratio.
                if width / height > ratio { height = width / ratio } else { width = height * ratio }
            }
            let x = if direction.x < 0.0 { anchor.x - width } else { anchor.x };
            let y = if direction.y < 0.0 { anchor.y - height } else { anchor.y };
            Rect::new(x, y, width, height)
        }
        CropGrip::Edge(index) => {
            let (mut new_left, mut new_top, mut new_right, mut new_bottom) = (left, top, right, bottom);
            match index {
                0 => new_top = point.y.min(bottom - MIN_SIDE),
                1 => new_right = point.x.max(left + MIN_SIDE),
                2 => new_bottom = point.y.max(top + MIN_SIDE),
                _ => new_left = point.x.min(right - MIN_SIDE),
            }
            let mut out = Rect::new(new_left, new_top, new_right - new_left, new_bottom - new_top);
            if let Some(ratio) = ratio {
                // The other dimension follows, centered on the edge's middle line.
                let center = rect.center();
                if index % 2 == 0 {
                    let width = out.height * ratio;
                    out = Rect::new(center.x - width * 0.5, out.y, width, out.height);
                } else {
                    let height = out.width / ratio;
                    out = Rect::new(out.x, center.y - height * 0.5, out.width, height);
                }
            }
            out
        }
        CropGrip::Inside | CropGrip::Outside => rect,
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::state;
    use super::*;

    #[test]
    fn dragging_a_corner_crops_and_keeps_the_crop_on_the_photo() {
        let mut state = state();
        state.set_tool(Tool::Crop);
        // The photo is 200 × 100; the bottom-right corner starts at (200, 100).
        assert!(state.press(Vec2::new(199.0, 99.0), 6.0));
        assert!(matches!(state.interaction, Some(Interaction::Crop { grip: CropGrip::Corner(2), .. })));
        state.drag(Vec2::new(150.0, 80.0), Keys::default(), 6.0);
        state.release();
        let rect = state.edit_settings().unwrap().crop.rect.unwrap();
        assert_eq!((rect.x, rect.y, rect.width, rect.height), (0.0, 0.0, 150.0, 80.0));
        // Dragging past the photo's edge stops at the edge.
        state.press(Vec2::new(150.0, 80.0), 6.0);
        state.drag(Vec2::new(500.0, 500.0), Keys::default(), 6.0);
        let rect = state.edit_settings().unwrap().crop.rect.unwrap();
        assert!((rect.width - 200.0).abs() < 0.5 && (rect.height - 100.0).abs() < 0.5, "{rect:?}");
    }

    #[test]
    fn a_held_ratio_shapes_the_drag() {
        let mut state = state();
        state.set_tool(Tool::Crop);
        state.set_crop_aspect(2); // 1 × 1
        let rect = state.edit_settings().unwrap().crop.rect.unwrap();
        let corner = Vec2::new(rect.x + rect.width, rect.y + rect.height);
        state.press(corner, 6.0);
        state.drag(corner - Vec2::new(40.0, 10.0), Keys::default(), 6.0);
        let rect = state.edit_settings().unwrap().crop.rect.unwrap();
        assert!((rect.width - rect.height).abs() < 0.5, "{rect:?}");
    }

    #[test]
    fn moving_the_crop_stops_at_the_edge() {
        let mut state = state();
        state.set_tool(Tool::Crop);
        state.edit("Crop", |photo| photo.edit.crop.rect = Some(Rect::new(50.0, 25.0, 100.0, 50.0)));
        assert!(state.press(Vec2::new(100.0, 50.0), 6.0));
        state.drag(Vec2::new(400.0, 50.0), Keys::default(), 6.0);
        let rect = state.edit_settings().unwrap().crop.rect.unwrap();
        assert!((rect.x - 100.0).abs() < 0.5 && rect.width == 100.0, "{rect:?}");
    }

    #[test]
    fn dragging_outside_the_crop_straightens() {
        let mut state = state();
        state.set_tool(Tool::Crop);
        state.edit("Crop", |photo| photo.edit.crop.rect = Some(Rect::new(80.0, 40.0, 40.0, 20.0)));
        // Around the crop's middle (100, 50): from straight right to a little below right.
        assert!(state.press(Vec2::new(190.0, 50.0), 2.0));
        assert!(matches!(state.interaction, Some(Interaction::Crop { grip: CropGrip::Outside, .. })));
        state.drag(Vec2::new(190.0, 50.0 + 90.0 * 5f32.to_radians().tan()), Keys { shift: true, alt: false }, 2.0);
        let crop = &state.edit_settings().unwrap().crop;
        assert_eq!(crop.angle, 5.0);
        assert!(crop.fits(crop.rect.unwrap(), (200, 100)));
    }

    #[test]
    fn drawing_makes_masks_and_their_handles_move() {
        let mut state = state();
        state.use_mask_tool(MaskKind::Radial);
        // A click alone makes nothing.
        state.press(Vec2::new(100.0, 50.0), 4.0);
        state.drag(Vec2::new(101.0, 50.0), Keys::default(), 4.0);
        state.release();
        assert!(state.edit_settings().unwrap().masks.is_empty());
        // A drag makes one, sized by the drag.
        state.press(Vec2::new(100.0, 50.0), 4.0);
        state.drag(Vec2::new(140.0, 70.0), Keys::default(), 4.0);
        state.release();
        let masks = &state.edit_settings().unwrap().masks;
        assert_eq!(masks.len(), 1);
        assert_eq!(state.selected_mask, Some(0));
        let MaskShape::Radial { center, radius, .. } = masks[0].shape else { panic!("radial") };
        assert_eq!(center, Vec2::new(0.5, 0.5));
        assert!((radius.x - 0.2).abs() < 1e-4 && (radius.y - 0.2).abs() < 1e-4, "{radius:?}");
        // Its right handle widens it.
        state.press(Vec2::new(140.0, 50.0), 4.0);
        assert!(matches!(state.interaction, Some(Interaction::Mask { grip: MaskGrip::Radius(true), .. })));
        state.drag(Vec2::new(160.0, 50.0), Keys::default(), 4.0);
        state.release();
        let MaskShape::Radial { radius, .. } = state.edit_settings().unwrap().masks[0].shape else { panic!("radial") };
        assert!((radius.x - 0.3).abs() < 1e-4, "{radius:?}");
        // Its pin moves it.
        state.press(Vec2::new(100.0, 50.0), 4.0);
        state.drag(Vec2::new(120.0, 50.0), Keys::default(), 4.0);
        state.release();
        assert_eq!(mask_pin(&state.edit_settings().unwrap().masks[0].shape), Vec2::new(0.6, 0.5));
        // A click on an empty spot lets go of it.
        state.press(Vec2::new(10.0, 10.0), 4.0);
        state.release();
        assert_eq!(state.selected_mask, None);
    }

    #[test]
    fn masks_stay_on_the_photo_when_it_is_straightened() {
        let mut state = state();
        state.set_angle(10.0);
        // A frame point maps to the image and back to itself.
        let point = Vec2::new(30.0, 70.0);
        let back = state.image_to_frame(state.frame_to_image(point).unwrap()).unwrap();
        assert!(back.distance(point) < 1e-3);
        // A linear gradient drawn in the frame lands where it was drawn on the photo.
        state.use_mask_tool(MaskKind::Linear);
        state.press(Vec2::new(100.0, 20.0), 4.0);
        state.drag(Vec2::new(100.0, 80.0), Keys::default(), 4.0);
        state.release();
        let MaskShape::Linear { start, .. } = state.edit_settings().unwrap().masks[0].shape else { panic!("linear") };
        let drawn = state.image_to_frame(state.image_point(start).unwrap()).unwrap();
        assert!(drawn.distance(Vec2::new(100.0, 20.0)) < 1e-2, "{drawn:?}");
    }
}

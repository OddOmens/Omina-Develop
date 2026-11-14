//! Crop & Straighten, as in Lightroom: quarter turns and flips, a straighten angle, and the
//! crop rectangle. None of it changes the photo's pixels; the rendered result is cut from the
//! photo as these settings say.
//!
//! Three spaces are involved:
//! - the *original*: the photo as decoded;
//! - the *image*: the original turned and flipped (`orient`), `oriented_size` pixels big;
//! - the *frame*: the image turned by the straighten angle about its center. The crop
//!   rectangle is an upright rectangle in the frame, so a straightened horizon stays level.
//!
//! Masks are placed in the image, so they stay on the same part of the photo when the crop or
//! the angle changes.

use serde::{Deserialize, Serialize};

use crate::bitmap::Bitmap;
use crate::geom::{Affine, Rect, Vec2};

/// The steepest straighten angle, either way, in degrees (Lightroom's limit too).
pub const MAX_ANGLE: f32 = 45.0;

/// The smallest a crop may get, in image pixels, so it never vanishes under a drag.
pub const MIN_SIDE: f32 = 16.0;

/// How the photo is turned, flipped, straightened and cropped.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Crop {
    /// Quarter turns clockwise, 0–3.
    pub quarter_turns: u8,
    /// Mirrored left to right, after turning.
    pub flip_horizontal: bool,
    /// Mirrored top to bottom, after turning.
    pub flip_vertical: bool,
    /// The straighten angle in degrees, clockwise.
    pub angle: f32,
    /// The kept area in the frame, in image pixels. `None` keeps all of it (the largest
    /// upright rectangle of the photo's shape that fits once straightened).
    pub rect: Option<Rect>,
}

/// Ratios the crop can be held to, as width over height (of a landscape crop; a portrait one
/// uses the inverse).
pub const ASPECTS: [(&str, Option<f32>); 8] = [
    ("Free", None),
    ("Original", Some(0.0)),
    ("1 × 1", Some(1.0)),
    ("4 × 5", Some(5.0 / 4.0)),
    ("5 × 7", Some(7.0 / 5.0)),
    ("2 × 3", Some(3.0 / 2.0)),
    ("4 × 3", Some(4.0 / 3.0)),
    ("16 × 9", Some(16.0 / 9.0)),
];

impl Crop {
    /// Whether the photo is shown whole and as it was taken.
    pub fn is_neutral(&self) -> bool {
        *self == Crop::default()
    }

    /// Whether the turns swap the photo's width and height.
    fn turned_sideways(&self) -> bool {
        self.quarter_turns % 2 == 1
    }

    /// The image's size (the original turned and flipped) for an original of `width` × `height`.
    pub fn oriented_size(&self, width: u32, height: u32) -> (u32, u32) {
        if self.turned_sideways() { (height, width) } else { (width, height) }
    }

    /// The original turned and flipped as the settings say.
    pub fn orient(&self, original: &Bitmap) -> Bitmap {
        let mut out = match self.quarter_turns % 4 {
            1 => original.rotated_quarter(true),
            2 => original.rotated_quarter(true).rotated_quarter(true),
            3 => original.rotated_quarter(false),
            _ => original.clone(),
        };
        if self.flip_horizontal {
            out = out.flipped(true);
        }
        if self.flip_vertical {
            out = out.flipped(false);
        }
        out
    }

    /// From image points to frame points: a turn by the straighten angle about the image's center.
    pub fn image_to_frame(&self, size: (u32, u32)) -> Affine {
        let center = Vec2::new(size.0 as f32 * 0.5, size.1 as f32 * 0.5);
        Affine::translate(-center).then(Affine::rotate(self.angle)).then(Affine::translate(center))
    }

    /// From frame points back to image points.
    pub fn frame_to_image(&self, size: (u32, u32)) -> Affine {
        let center = Vec2::new(size.0 as f32 * 0.5, size.1 as f32 * 0.5);
        Affine::translate(-center).then(Affine::rotate(-self.angle)).then(Affine::translate(center))
    }

    /// The whole straightened image's bounding box in the frame (what Crop mode shows).
    pub fn frame_bounds(&self, size: (u32, u32)) -> Rect {
        let transform = self.image_to_frame(size);
        let corners = Rect::new(0.0, 0.0, size.0 as f32, size.1 as f32).corners().map(|corner| transform.apply(corner));
        Rect::bounding(corners).unwrap_or_default()
    }

    /// The area kept, in the frame: the crop rectangle, or the largest rectangle of the image's
    /// shape that fits inside the straightened image.
    pub fn kept(&self, size: (u32, u32)) -> Rect {
        match self.rect {
            Some(rect) => rect,
            None => largest_fitting(size, self.angle, size.0 as f32 / size.1.max(1) as f32),
        }
    }

    /// Whether every corner of `rect` (in the frame) lands on the photo.
    pub fn fits(&self, rect: Rect, size: (u32, u32)) -> bool {
        fits(rect, size, self.angle)
    }

    /// `rect` made to fit on the photo: slid toward the photo's middle just far enough, or, if it
    /// is too big to fit anywhere, shrunk in the middle until it does. Its shape is kept.
    pub fn constrain(&self, rect: Rect, size: (u32, u32)) -> Rect {
        constrain(rect, size, self.angle)
    }

    /// Turns the photo a quarter turn (clockwise or not), keeping the crop on the same part of it.
    pub fn turn(&mut self, clockwise: bool, size_before: (u32, u32)) {
        let (width, height) = (size_before.0 as f32, size_before.1 as f32);
        // A mirrored photo turns the other way in the original's terms, so the stored turn
        // count runs backwards while exactly one flip is on.
        let mirrored = self.flip_horizontal != self.flip_vertical;
        let step = if clockwise != mirrored { 1 } else { 3 };
        self.quarter_turns = (self.quarter_turns + step) % 4;
        self.rect = self.rect.map(|rect| {
            if clockwise {
                Rect::new(height - rect.y - rect.height, rect.x, rect.height, rect.width)
            } else {
                Rect::new(rect.y, width - rect.x - rect.width, rect.height, rect.width)
            }
        });
    }

    /// Mirrors the photo left to right (`horizontal`) or top to bottom, keeping the crop on the
    /// same part of it.
    pub fn flip(&mut self, horizontal: bool, size: (u32, u32)) {
        let (width, height) = (size.0 as f32, size.1 as f32);
        if horizontal {
            self.flip_horizontal = !self.flip_horizontal;
            self.rect = self.rect.map(|rect| Rect::new(width - rect.x - rect.width, rect.y, rect.width, rect.height));
        } else {
            self.flip_vertical = !self.flip_vertical;
            self.rect = self.rect.map(|rect| Rect::new(rect.x, height - rect.y - rect.height, rect.width, rect.height));
        }
        // A mirror image of a tilted horizon tilts the other way.
        self.angle = -self.angle;
    }

    /// Sets the straighten angle, shrinking the crop as needed so it stays on the photo.
    pub fn set_angle(&mut self, degrees: f32, size: (u32, u32)) {
        self.angle = if degrees.is_finite() { degrees.clamp(-MAX_ANGLE, MAX_ANGLE) } else { 0.0 };
        if let Some(rect) = self.rect {
            self.rect = Some(constrain(rect, size, self.angle));
        }
    }

    /// This crop's framing carried over to another photo, whose image is `to` pixels where
    /// this one's is `from`: the same shape, the same place and size in proportion, and the
    /// same straighten angle, made to fit on that photo. The other photo keeps its own turns
    /// and flips. When it stands the other way (upright where this one lies on its side), the
    /// framing is turned a quarter with it, so a crop of the left half stays a crop of one
    /// half rather than a thin strip; the second value says whether that was done.
    pub fn carried_to(&self, other: &Crop, from: (u32, u32), to: (u32, u32)) -> (Crop, bool) {
        let upright = |size: (u32, u32)| size.1 > size.0;
        let turned = from.0 != from.1 && to.0 != to.1 && upright(from) != upright(to);
        // The framing as it would be on this photo turned a quarter, when the other is.
        let (rect, from) = if turned {
            let side = from.1 as f32;
            (self.rect.map(|rect| Rect::new(side - rect.y - rect.height, rect.x, rect.height, rect.width)), (from.1, from.0))
        } else {
            (self.rect, from)
        };
        let mut carried = Crop { angle: self.angle, rect: None, ..other.clone() };
        // One scale for both sides keeps the shape; the smaller of the two keeps it on the photo.
        let (across, down) = (to.0 as f32 / from.0.max(1) as f32, to.1 as f32 / from.1.max(1) as f32);
        let scale = across.min(down);
        carried.rect = rect.map(|rect| {
            let center = rect.center();
            let (width, height) = (rect.width * scale, rect.height * scale);
            carried.constrain(Rect::new(center.x * across - width * 0.5, center.y * down - height * 0.5, width, height), to)
        });
        (carried, turned)
    }

    /// Cuts the frame's area `area` (frame points) out of `image`, which is the oriented image
    /// at `scale` times its size. The result is `area` at `scale`.
    pub fn cut(&self, image: &Bitmap, size: (u32, u32), area: Rect, scale: f32) -> Bitmap {
        let width = (area.width * scale).round().max(1.0) as u32;
        let height = (area.height * scale).round().max(1.0) as u32;
        let upright = self.angle.abs() < 1e-4;
        // Without a straighten angle and on whole pixels, a plain copy is exact and quick.
        let origin = area.min() * scale;
        if upright && (origin.x - origin.x.round()).abs() < 1e-3 && (origin.y - origin.y.round()).abs() < 1e-3 {
            return image.crop(crate::geom::IRect::new(origin.x.round() as i32, origin.y.round() as i32, width as i32, height as i32));
        }
        let transform = Affine::scale(1.0 / scale, 1.0 / scale)
            .then(self.image_to_frame(size))
            .then(Affine::translate(-area.min()))
            .then(Affine::scale(scale, scale));
        image.transformed(transform, width, height)
    }
}

/// Whether every corner of `rect` (frame points) lands inside an image of `size` turned by `angle`.
fn fits(rect: Rect, size: (u32, u32), angle: f32) -> bool {
    let crop = Crop { angle, ..Crop::default() };
    let back = crop.frame_to_image(size);
    // A sliver of tolerance, so a rectangle touching the edge exactly still counts.
    let image = Rect::new(0.0, 0.0, size.0 as f32, size.1 as f32).inflate(0.01);
    rect.corners().into_iter().all(|corner| image.contains(back.apply(corner)))
}

fn constrain(rect: Rect, size: (u32, u32), angle: f32) -> Rect {
    let image_center = Vec2::new(size.0 as f32 * 0.5, size.1 as f32 * 0.5);
    let rect = Rect::new(rect.x, rect.y, rect.width.max(MIN_SIDE.min(size.0 as f32)), rect.height.max(MIN_SIDE.min(size.1 as f32)));
    if fits(rect, size, angle) {
        return rect;
    }
    // First slide it toward the photo's middle, only as far as needed.
    let moved = |amount: f32| {
        let center = rect.center().lerp(image_center, amount);
        Rect::new(center.x - rect.width * 0.5, center.y - rect.height * 0.5, rect.width, rect.height)
    };
    let centered = moved(1.0);
    if fits(centered, size, angle) {
        let (mut low, mut high) = (0.0f32, 1.0f32);
        for _ in 0..30 {
            let middle = (low + high) * 0.5;
            if fits(moved(middle), size, angle) { high = middle } else { low = middle }
        }
        return moved(high);
    }
    // Too big to fit anywhere: shrink it in the middle, keeping its shape, to the largest that fits.
    let scaled = |factor: f32| Rect::new(image_center.x - rect.width * factor * 0.5, image_center.y - rect.height * factor * 0.5, rect.width * factor, rect.height * factor);
    let (mut low, mut high) = (0.0f32, 1.0f32);
    for _ in 0..30 {
        let middle = (low + high) * 0.5;
        if fits(scaled(middle), size, angle) { low = middle } else { high = middle }
    }
    scaled(low)
}

/// The largest upright rectangle with sides in `ratio` (width over height), centered, that
/// fits inside an image of `size` turned by `angle`.
pub fn largest_fitting(size: (u32, u32), angle: f32, ratio: f32) -> Rect {
    let (width, height) = (size.0 as f32, size.1 as f32);
    let center = Vec2::new(width * 0.5, height * 0.5);
    let ratio = if ratio.is_finite() && ratio > 0.0 { ratio } else { width / height.max(1.0) };
    // Start from the largest of that shape inside the unturned image, then shrink it to fit.
    let (start_width, start_height) = if width / height.max(1.0) > ratio { (height * ratio, height) } else { (width, width / ratio) };
    constrain(Rect::new(center.x - start_width * 0.5, center.y - start_height * 0.5, start_width, start_height), size, angle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Rgba;

    fn marked_picture() -> Bitmap {
        // 4 × 2, with a red top-left pixel to follow through turns and flips.
        let mut bitmap = Bitmap::filled(4, 2, Rgba::WHITE);
        bitmap.set_pixel(0, 0, [255, 0, 0, 255]);
        bitmap
    }

    #[test]
    fn turns_and_flips_move_the_marked_corner() {
        let mut crop = Crop::default();
        crop.turn(true, (4, 2));
        let turned = crop.orient(&marked_picture());
        assert_eq!((turned.width, turned.height), (2, 4));
        assert_eq!(turned.pixel(1, 0), [255, 0, 0, 255], "top-left goes to top-right on a clockwise turn");
        crop.flip(true, (2, 4));
        assert_eq!(crop.orient(&marked_picture()).pixel(0, 0), [255, 0, 0, 255]);
        // Four clockwise turns are no turn at all.
        let mut crop = Crop::default();
        for _ in 0..4 {
            crop.turn(true, (4, 2));
        }
        assert_eq!(crop.quarter_turns, 0);
    }

    #[test]
    fn a_turned_mirrored_photo_turns_the_way_it_looks() {
        // Mirrored first, then turned clockwise on screen: the red corner (now top-right) moves
        // to the bottom-right, as a clockwise turn of what is shown.
        let mut crop = Crop::default();
        crop.flip(true, (4, 2));
        assert_eq!(crop.orient(&marked_picture()).pixel(3, 0), [255, 0, 0, 255]);
        crop.turn(true, (4, 2));
        let shown = crop.orient(&marked_picture());
        assert_eq!(shown.pixel(1, 3), [255, 0, 0, 255]);
    }

    #[test]
    fn the_crop_follows_the_photo_through_a_turn() {
        let mut crop = Crop { rect: Some(Rect::new(0.0, 0.0, 10.0, 20.0)), ..Crop::default() };
        crop.turn(true, (100, 50));
        assert_eq!(crop.rect, Some(Rect::new(30.0, 0.0, 20.0, 10.0)));
        crop.turn(false, (50, 100));
        assert_eq!(crop.rect, Some(Rect::new(0.0, 0.0, 10.0, 20.0)));
    }

    #[test]
    fn a_crop_is_carried_to_another_photo_in_proportion() {
        // The right half of a 600 × 400 photo, straightened a little.
        let crop = Crop { rect: Some(Rect::new(300.0, 100.0, 240.0, 160.0)), angle: 1.5, ..Crop::default() };
        // A photo twice the size gets the same framing, twice as big.
        let (bigger, turned) = crop.carried_to(&Crop::default(), (600, 400), (1200, 800));
        assert!(!turned);
        assert_eq!((bigger.rect, bigger.angle), (Some(Rect::new(600.0, 200.0, 480.0, 320.0)), 1.5));
        // A photo of another shape keeps the crop's own shape, at the same place in proportion.
        let (squarer, _) = crop.carried_to(&Crop::default(), (600, 400), (400, 400));
        let rect = squarer.rect.unwrap();
        assert!((rect.width / rect.height - 1.5).abs() < 0.01 && squarer.fits(rect, (400, 400)), "{rect:?}");
        assert!((rect.center().x / 400.0 - 0.7).abs() < 0.02 && (rect.center().y / 400.0 - 0.45).abs() < 0.02, "{rect:?}");
        // The other photo keeps its own turns and flips.
        let flipped = Crop { flip_horizontal: true, quarter_turns: 2, ..Crop::default() };
        let (kept, _) = crop.carried_to(&flipped, (600, 400), (600, 400));
        assert_eq!((kept.flip_horizontal, kept.quarter_turns, kept.rect), (true, 2, crop.rect));
        // An upright photo gets the framing turned with it: the crop becomes upright too.
        let (upright, turned) = crop.carried_to(&Crop::default(), (600, 400), (400, 600));
        let rect = upright.rect.unwrap();
        assert!(turned && (rect.height / rect.width - 1.5).abs() < 0.01 && upright.fits(rect, (400, 600)), "{rect:?}");
        // With no crop rectangle, only the angle is carried, and a crop too big is made to fit.
        let (plain, _) = Crop { angle: -3.0, ..Crop::default() }.carried_to(&Crop { rect: Some(Rect::new(0.0, 0.0, 50.0, 50.0)), ..Crop::default() }, (600, 400), (300, 200));
        assert_eq!((plain.rect, plain.angle), (None, -3.0));
        let whole = Crop { rect: Some(Rect::new(0.0, 0.0, 600.0, 400.0)), angle: 5.0, ..Crop::default() };
        let (fitted, _) = whole.carried_to(&Crop::default(), (600, 400), (300, 200));
        assert!(fitted.fits(fitted.rect.unwrap(), (300, 200)));
    }

    #[test]
    fn straightening_keeps_the_crop_on_the_photo() {
        let size = (300, 200);
        let mut crop = Crop { rect: Some(Rect::new(0.0, 0.0, 300.0, 200.0)), ..Crop::default() };
        crop.set_angle(10.0, size);
        let rect = crop.rect.unwrap();
        assert!(crop.fits(rect, size));
        assert!(rect.width < 300.0 && (rect.width / rect.height - 1.5).abs() < 0.01, "shrunk, same shape: {rect:?}");
        // The automatic crop fits too, and is as big as can be.
        let auto = Crop { angle: 10.0, ..Crop::default() }.kept(size);
        assert!(crop.fits(auto, size) && !crop.fits(auto.inflate(2.0), size));
    }

    #[test]
    fn a_crop_off_the_photo_is_brought_back() {
        let size = (100, 100);
        let crop = Crop::default();
        let rect = crop.constrain(Rect::new(80.0, 80.0, 50.0, 50.0), size);
        assert!(crop.fits(rect, size), "{rect:?}");
        assert!((rect.width - 50.0).abs() < 0.01 && (rect.x - 50.0).abs() < 0.1, "slid back, same size: {rect:?}");
    }

    #[test]
    fn cutting_matches_the_crop() {
        let mut picture = Bitmap::filled(20, 10, Rgba::WHITE);
        picture.set_pixel(5, 3, [0, 0, 255, 255]);
        let crop = Crop { rect: Some(Rect::new(5.0, 3.0, 4.0, 4.0)), ..Crop::default() };
        let cut = crop.cut(&picture, (20, 10), crop.kept((20, 10)), 1.0);
        assert_eq!((cut.width, cut.height), (4, 4));
        assert_eq!(cut.pixel(0, 0), [0, 0, 255, 255]);
        // At half size the crop is half as big.
        let half = crop.cut(&picture.resized(10, 5), (20, 10), crop.kept((20, 10)), 0.5);
        assert_eq!((half.width, half.height), (2, 2));
        // Straightened, the corners of the whole frame fall off the photo and are transparent.
        let tilted = Crop { angle: 20.0, ..Crop::default() };
        let whole = tilted.cut(&picture, (20, 10), tilted.frame_bounds((20, 10)), 1.0);
        assert_eq!(whole.pixel(0, 0)[3], 0);
    }
}

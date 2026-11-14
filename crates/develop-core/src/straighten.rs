//! Finding how far a photo is tilted, for Crop & Straighten's Level. Nothing here guesses:
//! the edges in a small gray copy of the photo are measured, and an angle is given only when
//! one direction close to level or plumb clearly stands out among them (a horizon, a shoreline,
//! the side of a building). A photo of foliage, a face or a sunset with no clear line gets no
//! angle at all, which is a better answer than a wrong one.
//!
//! How it works: every pixel on an edge says which way that edge runs (at right angles to the
//! direction in which the brightness changes) and how strong it is. Edges within a few degrees
//! of level are gathered by their tilt into fine bins, weighted by strength, and so are edges
//! within a few degrees of plumb. A straight line across the photo puts all its pixels into
//! the same bin, so it shows as a sharp peak; texture spreads thinly over every bin.

use crate::bitmap::Bitmap;
use crate::blur::gaussian_f32;

/// The longest side of the copy that is measured, in pixels. Small enough to be quick, large
/// enough to tell a tenth of a degree.
const MEASURED_SIDE: u32 = 512;
/// The steepest tilt looked for, either way, in degrees. A steeper line is more likely a
/// slope or a roof than a horizon.
pub const MOST_TILT: f32 = 10.0;
/// How wide a bin of tilts is, in degrees.
const BIN: f32 = 0.1;
/// How far either side of the strongest bin still counts as the same line, in degrees.
const PEAK_REACH: f32 = 1.0;
/// The share of the edge strength within `MOST_TILT` of level (or plumb) the line must hold.
/// Texture alone, spread evenly, puts about a tenth there.
const LEAST_SHARE_OF_WINDOW: f32 = 0.3;
/// The share of all the edge strength in the photo the line must hold, so one faint line in
/// a photo full of other things isn't taken for its horizon.
const LEAST_SHARE_OF_ALL: f32 = 0.06;
/// How long the line must be, as a share of the copy's longer side.
const LEAST_LENGTH: f32 = 0.3;
/// How sharply the brightness must change at a pixel for it to count as on an edge, as the
/// square of the change measured there. This is what tells a horizon from the gentle ramp of
/// an evening sky, which has one direction all over and would otherwise look like a perfect
/// line.
const LEAST_EDGE: f32 = 0.12;
/// A tilt smaller than this is level: straightening by less can't be seen.
const LEVEL_WITHIN: f32 = 0.1;

/// The edges close to one of the two directions (level or plumb), gathered by tilt.
struct Tilts {
    /// The edge strength in each bin of tilt, from `-MOST_TILT` to `MOST_TILT`.
    strength: Vec<f32>,
    /// The sum of tilt × strength in each bin, for the peak's exact place.
    moment: Vec<f32>,
    /// How many edge pixels fell in each bin.
    pixels: Vec<u32>,
}

impl Tilts {
    fn new() -> Self {
        let bins = (2.0 * MOST_TILT / BIN).round() as usize + 1;
        Self { strength: vec![0.0; bins], moment: vec![0.0; bins], pixels: vec![0; bins] }
    }

    fn add(&mut self, tilt: f32, strength: f32) {
        if tilt.abs() > MOST_TILT {
            return;
        }
        let bin = (((tilt + MOST_TILT) / BIN).round() as usize).min(self.strength.len() - 1);
        self.strength[bin] += strength;
        self.moment[bin] += tilt * strength;
        self.pixels[bin] += 1;
    }

    /// The tilt of the line that stands out, and how much edge strength it holds. `None` when
    /// nothing stands out clearly: `all` is the edge strength of the whole photo, and
    /// `longest` its longer side in pixels.
    fn clear_line(&self, all: f32, longest: f32) -> Option<(f32, f32)> {
        let reach = (PEAK_REACH / BIN).round() as usize;
        let within: f32 = self.strength.iter().sum();
        if within <= 0.0 {
            return None;
        }
        // The strongest stretch of bins `reach` either side of a middle one.
        let held = |middle: usize| {
            let (from, to) = (middle.saturating_sub(reach), (middle + reach).min(self.strength.len() - 1));
            (from..=to).fold((0.0f32, 0.0f32, 0u32), |sum, bin| (sum.0 + self.strength[bin], sum.1 + self.moment[bin], sum.2 + self.pixels[bin]))
        };
        let (strength, moment, pixels) = (0..self.strength.len()).map(held).max_by(|a, b| a.0.total_cmp(&b.0))?;
        let clear = strength >= within * LEAST_SHARE_OF_WINDOW && strength >= all * LEAST_SHARE_OF_ALL && pixels as f32 >= longest * LEAST_LENGTH;
        clear.then(|| (moment / strength, strength))
    }
}

/// How far the photo in `bitmap` is tilted, in degrees clockwise, if a line close to level or
/// plumb clearly shows it. `Some(0.0)` when that line is level already.
pub fn tilt(bitmap: &Bitmap) -> Option<f32> {
    if bitmap.width < 16 || bitmap.height < 16 {
        return None;
    }
    let longest = bitmap.width.max(bitmap.height);
    let scale = (MEASURED_SIDE as f32 / longest as f32).min(1.0);
    let small = if scale < 1.0 { bitmap.resized(((bitmap.width as f32 * scale).round() as u32).max(1), ((bitmap.height as f32 * scale).round() as u32).max(1)) } else { bitmap.clone() };
    let (width, height) = (small.width as usize, small.height as usize);
    // Brightness, lightly smoothed so single noisy pixels don't count as edges.
    let mut gray: Vec<f32> = small.data.as_chunks::<4>().0.iter().map(|pixel| (0.2126 * f32::from(pixel[0]) + 0.7152 * f32::from(pixel[1]) + 0.0722 * f32::from(pixel[2])) / 255.0).collect();
    gaussian_f32(&mut gray, width, height, 1.2);
    let at = |x: usize, y: usize| gray[y * width + x];
    let (mut level, mut plumb) = (Tilts::new(), Tilts::new());
    let mut all = 0.0f32;
    // The outermost pixels are left out: the smoothing has nothing beyond them to go on.
    let margin = 3;
    for y in margin..height.saturating_sub(margin) {
        for x in margin..width.saturating_sub(margin) {
            // How the brightness changes across and down (Scharr's weights, which keep the
            // direction true at every angle).
            let across = 3.0 * (at(x + 1, y - 1) - at(x - 1, y - 1)) + 10.0 * (at(x + 1, y) - at(x - 1, y)) + 3.0 * (at(x + 1, y + 1) - at(x - 1, y + 1));
            let down = 3.0 * (at(x - 1, y + 1) - at(x - 1, y - 1)) + 10.0 * (at(x, y + 1) - at(x, y - 1)) + 3.0 * (at(x + 1, y + 1) - at(x + 1, y - 1));
            let strength = across * across + down * down;
            if strength < LEAST_EDGE {
                continue;
            }
            all += strength;
            // The brightness changes at right angles to the edge. Folded to a half turn, that
            // direction is 90° for a level edge and 0° for a plumb one, plus the edge's tilt.
            let mut changes = down.atan2(across).to_degrees();
            if changes > 90.0 {
                changes -= 180.0;
            } else if changes <= -90.0 {
                changes += 180.0;
            }
            level.add(if changes > 0.0 { changes - 90.0 } else { changes + 90.0 }, strength);
            plumb.add(changes, strength);
        }
    }
    let longest = width.max(height) as f32;
    // A level line is the better witness (uprights lean with perspective), unless a plumb
    // one is far stronger.
    let found = match (level.clear_line(all, longest), plumb.clear_line(all, longest)) {
        (Some((tilt, strength)), Some((_, upright))) if strength * 2.0 >= upright => tilt,
        (_, Some((tilt, _))) | (Some((tilt, _)), None) => tilt,
        (None, None) => return None,
    };
    Some(if found.abs() < LEVEL_WITHIN { 0.0 } else { found })
}

/// The straighten angle that levels the photo in `bitmap` (the photo as it came, turned and
/// flipped as it is shown but not yet straightened), if its tilt is clear. This is the value
/// for `Crop::angle`: the tilt, taken out.
pub fn level_angle(bitmap: &Bitmap) -> Option<f32> {
    tilt(bitmap).map(|tilt| if tilt == 0.0 { 0.0 } else { (-tilt * 100.0).round() / 100.0 })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crop::Crop;
    use crate::edit::{Edit, View};

    /// A repeatable scatter of values from 0 to 1, for grain and texture.
    fn scatter(x: u32, y: u32) -> f32 {
        let mut hash = x.wrapping_mul(0x27d4_eb2d) ^ y.wrapping_mul(0x1656_67b1);
        hash ^= hash >> 15;
        hash = hash.wrapping_mul(0x85eb_ca6b);
        hash ^= hash >> 13;
        (hash & 0xffff) as f32 / 65535.0
    }

    /// A 640 × 420 photo whose brightness at each point is `shade` of where the point lies
    /// once the photo's tilt (degrees clockwise) is taken out, with a little grain.
    fn photo(tilt: f32, shade: impl Fn(f32, f32) -> f32) -> Bitmap {
        let (width, height) = (640u32, 420u32);
        let (sin, cos) = tilt.to_radians().sin_cos();
        let mut bitmap = Bitmap::new(width, height);
        for y in 0..height {
            for x in 0..width {
                let (dx, dy) = (x as f32 - width as f32 * 0.5, y as f32 - height as f32 * 0.5);
                // The same point in the level scene.
                let (across, down) = (dx * cos + dy * sin, dy * cos - dx * sin);
                let value = (shade(across, down) + (scatter(x, y) - 0.5) * 0.06).clamp(0.0, 1.0);
                let byte = (value * 255.0).round() as u8;
                bitmap.set_pixel(x as i32, y as i32, [byte, byte, byte, 255]);
            }
        }
        bitmap
    }

    /// Sky above a horizon a little below the middle, darker land under it, both with
    /// soft gradients as real ones have.
    fn seascape(across: f32, down: f32) -> f32 {
        let edge = ((down - 30.0) / 1.5).clamp(-1.0, 1.0) * 0.5 + 0.5;
        let sky = 0.78 - down * 0.0004 + across * 0.0001;
        let sea = 0.32 + down * 0.0003;
        sky + (sea - sky) * edge
    }

    /// A building's uprights: broad vertical bands, light and dark by turns.
    fn facade(across: f32, _down: f32) -> f32 {
        let band = ((across * std::f32::consts::PI / 55.0).sin() * 8.0).clamp(-1.0, 1.0);
        0.5 + 0.15 * band
    }

    #[test]
    fn a_tilted_horizon_is_found_to_a_third_of_a_degree() {
        for tilted in [-8.0f32, -4.5, -2.0, -0.7, 0.5, 1.3, 3.1, 6.4, 9.0] {
            let found = tilt(&photo(tilted, seascape)).unwrap_or_else(|| panic!("a horizon tilted {tilted}° should be found"));
            assert!((found - tilted).abs() < 0.33, "tilted {tilted}°, found {found}°");
            assert_eq!(level_angle(&photo(tilted, seascape)).map(f32::signum), Some(-tilted.signum()), "the angle that levels it turns it back");
        }
    }

    #[test]
    fn uprights_show_the_tilt_when_there_is_no_horizon() {
        for tilted in [-5.0f32, -1.5, 2.2, 7.0] {
            let found = tilt(&photo(tilted, facade)).unwrap_or_else(|| panic!("uprights tilted {tilted}° should be found"));
            assert!((found - tilted).abs() < 0.33, "tilted {tilted}°, found {found}°");
        }
    }

    #[test]
    fn a_level_photo_is_called_level() {
        assert_eq!(tilt(&photo(0.0, seascape)), Some(0.0));
        assert_eq!(level_angle(&photo(0.0, seascape)), Some(0.0));
        assert_eq!(tilt(&photo(0.05, facade)), Some(0.0), "too little to see");
    }

    #[test]
    fn photos_with_no_clear_line_get_no_angle() {
        // Grain alone, a soft round glow, a smooth ramp of light across the frame, blobs like
        // foliage, and a line far too steep to be a horizon.
        let grain = photo(0.0, |_, _| 0.5);
        let ramp = photo(0.0, |across, down| 0.5 + across * 0.0004 + down * 0.0012);
        let glow = photo(0.0, |across, down| 0.8 - (across * across + down * down).sqrt() * 0.002);
        let foliage = photo(0.0, |across, down| 0.5 + 0.2 * ((across * 0.21).sin() * (down * 0.17).cos() + (across * 0.09 + down * 0.13).sin()) * 0.5);
        let slope = photo(25.0, seascape);
        for (name, bitmap) in [("grain", grain), ("glow", glow), ("ramp", ramp), ("foliage", foliage), ("slope", slope)] {
            assert_eq!(tilt(&bitmap), None, "{name}");
        }
        assert_eq!(tilt(&Bitmap::new(8, 8)), None, "too small to measure");
    }

    #[test]
    fn a_faint_line_among_stronger_things_is_not_taken_for_the_horizon() {
        // Strong foliage with one faint level line through it.
        let busy = photo(3.0, |across, down| {
            let line = if (down - 40.0).abs() < 1.0 { 0.03 } else { 0.0 };
            0.5 + 0.25 * ((across * 0.21).sin() * (down * 0.17).cos() + (across * 0.09 + down * 0.13).sin()) * 0.5 + line
        });
        assert_eq!(tilt(&busy), None);
    }

    #[test]
    fn straightening_by_the_found_angle_levels_the_photo() {
        for tilted in [-6.0f32, 2.5] {
            let original = photo(tilted, seascape);
            let size = (original.width, original.height);
            let angle = level_angle(&original).expect("the horizon is clear");
            let edit = Edit { crop: Crop { angle, ..Crop::default() }, ..Edit::default() };
            let leveled = edit.render(&original, size, 1.0, View::Result);
            let left = tilt(&leveled).expect("the horizon is still there");
            assert!(left.abs() < 0.33, "tilted {tilted}°, straightened by {angle}°, still {left}° off");
        }
    }
}

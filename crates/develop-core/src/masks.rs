//! Masks: local adjustments, as in Lightroom's Masking panel. Each mask is a shape that says
//! how strongly it covers each part of the photo (a linear gradient fading across it, or a
//! radial gradient around an ellipse) and a set of light and color settings applied there.
//!
//! A mask's shape is placed in the image (the photo turned and flipped, before straightening
//! and cropping), measured in fractions of its width and height, so it stays on the same part
//! of the photo whatever the crop and at any preview size.

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::bitmap::Bitmap;
use crate::develop::{Develop, DevelopSetting, develop_without_effects};
use crate::geom::Vec2;

/// The settings a mask can change, in the order its panel lists them.
pub const MASK_SETTINGS: [DevelopSetting; 12] = [
    DevelopSetting::Temperature,
    DevelopSetting::Tint,
    DevelopSetting::Exposure,
    DevelopSetting::Contrast,
    DevelopSetting::Highlights,
    DevelopSetting::Shadows,
    DevelopSetting::Whites,
    DevelopSetting::Blacks,
    DevelopSetting::Texture,
    DevelopSetting::Clarity,
    DevelopSetting::Dehaze,
    DevelopSetting::Saturation,
];

/// Where a mask applies, in image fractions (0–1 across and down).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum MaskShape {
    /// Full strength on `start`'s side, fading to nothing at `end`.
    Linear { start: Vec2, end: Vec2 },
    /// Full strength inside an ellipse around `center`, fading out toward its edge. `radius` is
    /// the half-width and half-height (image fractions), `angle` its turn in degrees, and
    /// `feather` (0–100) how much of the ellipse the fade takes.
    Radial { center: Vec2, radius: Vec2, angle: f32, feather: f32 },
}

impl MaskShape {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Linear { .. } => "Linear Gradient",
            Self::Radial { .. } => "Radial Gradient",
        }
    }

    /// A linear gradient from `start` to `end`.
    pub fn linear(start: Vec2, end: Vec2) -> Self {
        Self::Linear { start, end }
    }

    /// A radial gradient filling the ellipse between two opposite corners, with the standard feather.
    pub fn radial(center: Vec2, radius: Vec2) -> Self {
        Self::Radial { center, radius: Vec2::new(radius.x.abs().max(0.005), radius.y.abs().max(0.005)), angle: 0.0, feather: 50.0 }
    }

    /// The shape after the photo is turned a quarter turn (clockwise or not), so it stays on
    /// the same part of the photo. Fractions of the old width become fractions of the new
    /// height, so a radial gradient's two radii swap and its angle stays.
    pub fn turned(self, clockwise: bool) -> Self {
        let point = |p: Vec2| if clockwise { Vec2::new(1.0 - p.y, p.x) } else { Vec2::new(p.y, 1.0 - p.x) };
        match self {
            Self::Linear { start, end } => Self::Linear { start: point(start), end: point(end) },
            Self::Radial { center, radius, angle, feather } => Self::Radial { center: point(center), radius: Vec2::new(radius.y, radius.x), angle, feather },
        }
    }

    /// The shape after the photo is mirrored left to right (`horizontal`) or top to bottom, so
    /// it stays on the same part of the photo. A mirrored ellipse leans the other way.
    pub fn flipped(self, horizontal: bool) -> Self {
        let point = |p: Vec2| if horizontal { Vec2::new(1.0 - p.x, p.y) } else { Vec2::new(p.x, 1.0 - p.y) };
        match self {
            Self::Linear { start, end } => Self::Linear { start: point(start), end: point(end) },
            Self::Radial { center, radius, angle, feather } => Self::Radial { center: point(center), radius, angle: (-angle).rem_euclid(360.0), feather },
        }
    }

    /// How strongly (0–1) the shape covers the point `at`, in image pixels of an image of `size`.
    fn coverage(&self, at: Vec2, size: Vec2) -> f32 {
        match *self {
            Self::Linear { start, end } => {
                let (start, end) = (scale(start, size), scale(end, size));
                let along = end - start;
                let length_squared = along.dot(along);
                if length_squared < 1e-6 {
                    return if (at - start).dot(along) <= 0.0 { 1.0 } else { 0.0 };
                }
                let t = (at - start).dot(along) / length_squared;
                1.0 - smoothstep(0.0, 1.0, t)
            }
            Self::Radial { center, radius, angle, feather } => {
                let offset = at - scale(center, size);
                // Measured in the ellipse's own turned axes, as a fraction of its radii.
                let (sin, cos) = (-angle).to_radians().sin_cos();
                let turned = Vec2::new(offset.x * cos - offset.y * sin, offset.x * sin + offset.y * cos);
                let radius = scale(radius, size);
                let distance = Vec2::new(turned.x / radius.x.max(1e-3), turned.y / radius.y.max(1e-3)).length();
                let fade = (feather / 100.0).clamp(0.0, 1.0);
                if fade < 1e-3 {
                    return if distance <= 1.0 { 1.0 } else { 0.0 };
                }
                1.0 - smoothstep(1.0 - fade, 1.0, distance)
            }
        }
    }
}

fn scale(point: Vec2, size: Vec2) -> Vec2 {
    Vec2::new(point.x * size.x, point.y * size.y)
}

fn smoothstep(edge0: f32, edge1: f32, value: f32) -> f32 {
    let t = ((value - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// One local adjustment.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Mask {
    pub name: String,
    pub shape: MaskShape,
    /// Covers everything the shape doesn't.
    pub invert: bool,
    /// Turned off, the mask is kept but changes nothing.
    pub enabled: bool,
    /// How much of the settings apply, 0–100 (Lightroom's Amount).
    pub amount: f32,
    /// The mask's settings. Only those in `MASK_SETTINGS` are used; the rest stay at their defaults.
    pub settings: Develop,
}

impl Default for Mask {
    fn default() -> Self {
        Self { name: "Mask".into(), shape: MaskShape::radial(Vec2::new(0.5, 0.5), Vec2::new(0.25, 0.25)), invert: false, enabled: true, amount: 100.0, settings: Develop::default() }
    }
}

impl Mask {
    pub fn new(name: impl Into<String>, shape: MaskShape) -> Self {
        Self { name: name.into(), shape, ..Self::default() }
    }

    /// Whether the mask changes anything.
    pub fn is_active(&self) -> bool {
        self.enabled && self.amount > 0.0 && !self.local_settings().is_neutral()
    }

    /// The mask's settings with everything outside `MASK_SETTINGS` back at its default, so a
    /// stray value from an older file can't change more than the panel shows.
    pub fn local_settings(&self) -> Develop {
        let mut local = Develop::default();
        for setting in MASK_SETTINGS {
            setting.set(&mut local, setting.get(&self.settings));
        }
        local
    }

    /// How strongly the mask covers each pixel of an image `width` × `height` (at any preview
    /// size), 0–1, with Amount applied.
    pub fn coverage(&self, width: u32, height: u32) -> Vec<f32> {
        let size = Vec2::new(width as f32, height as f32);
        let amount = (self.amount / 100.0).clamp(0.0, 1.0);
        let mut weights = vec![0.0f32; width as usize * height as usize];
        weights.par_chunks_exact_mut(width.max(1) as usize).enumerate().for_each(|(y, row)| {
            for (x, weight) in row.iter_mut().enumerate() {
                let covered = self.shape.coverage(Vec2::new(x as f32 + 0.5, y as f32 + 0.5), size);
                *weight = if self.invert { 1.0 - covered } else { covered } * amount;
            }
        });
        weights
    }
}

/// Applies each active mask in turn to `image` (the oriented image, already developed, at
/// `scale` times its full size).
pub fn apply_masks(image: &Bitmap, masks: &[Mask], scale: f32) -> Bitmap {
    let mut out = image.clone();
    for mask in masks.iter().filter(|mask| mask.is_active()) {
        let adjusted = develop_without_effects(&out, &mask.local_settings(), scale);
        let weights = mask.coverage(out.width, out.height);
        out.data.par_chunks_exact_mut(4).zip(adjusted.data.par_chunks_exact(4)).zip(weights.par_iter()).for_each(|((pixel, adjusted), weight)| {
            if *weight <= 0.0 {
                return;
            }
            for channel in 0..4 {
                let (from, to) = (f32::from(pixel[channel]), f32::from(adjusted[channel]));
                pixel[channel] = (from + (to - from) * weight).round().clamp(0.0, 255.0) as u8;
            }
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Rgba;

    #[test]
    fn a_linear_gradient_fades_from_start_to_end() {
        let shape = MaskShape::linear(Vec2::new(0.0, 0.5), Vec2::new(1.0, 0.5));
        let size = Vec2::new(100.0, 10.0);
        assert!(shape.coverage(Vec2::new(0.0, 5.0), size) > 0.99);
        assert!((shape.coverage(Vec2::new(50.0, 5.0), size) - 0.5).abs() < 0.01);
        assert!(shape.coverage(Vec2::new(100.0, 5.0), size) < 0.01);
    }

    #[test]
    fn a_radial_gradient_covers_its_middle_and_not_its_outside() {
        let shape = MaskShape::radial(Vec2::new(0.5, 0.5), Vec2::new(0.25, 0.25));
        let size = Vec2::new(100.0, 100.0);
        assert_eq!(shape.coverage(Vec2::new(50.0, 50.0), size), 1.0);
        assert_eq!(shape.coverage(Vec2::new(90.0, 50.0), size), 0.0);
        // A turned ellipse reaches farther along its long side.
        let long = MaskShape::Radial { center: Vec2::new(0.5, 0.5), radius: Vec2::new(0.4, 0.1), angle: 90.0, feather: 0.0 };
        assert_eq!(long.coverage(Vec2::new(50.0, 85.0), size), 1.0);
        assert_eq!(long.coverage(Vec2::new(85.0, 50.0), size), 0.0);
    }

    #[test]
    fn masks_change_only_where_they_cover() {
        let picture = Bitmap::filled(40, 4, Rgba::rgb(0.4, 0.4, 0.4));
        let mut mask = Mask::new("Brighter left", MaskShape::linear(Vec2::new(0.0, 0.5), Vec2::new(0.5, 0.5)));
        mask.settings.exposure = 1.0;
        let out = apply_masks(&picture, &[mask.clone()], 1.0);
        assert!(out.pixel(0, 2)[0] > picture.pixel(0, 2)[0] + 30);
        assert_eq!(out.pixel(39, 2), picture.pixel(39, 2));
        // Inverted, it's the other way round; turned off, nothing changes.
        let inverted = apply_masks(&picture, &[Mask { invert: true, ..mask.clone() }], 1.0);
        assert_eq!(inverted.pixel(0, 2), picture.pixel(0, 2));
        assert_eq!(apply_masks(&picture, &[Mask { enabled: false, ..mask }], 1.0), picture);
    }

    #[test]
    fn settings_a_mask_has_no_slider_for_are_ignored() {
        let mut mask = Mask::default();
        mask.settings.grain = 80.0;
        assert!(!mask.is_active());
        mask.settings.shadows = 20.0;
        assert!(mask.is_active());
        assert_eq!(mask.local_settings().grain, Develop::default().grain);
    }
}

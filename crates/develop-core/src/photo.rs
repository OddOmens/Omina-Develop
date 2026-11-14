//! A photo being edited: the original as imported (never changed) and what is done to it, with
//! its snapshots. This is everything an open document holds and everything a project file saves.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::adjust::CurvePoints;
use crate::bitmap::Bitmap;
use crate::crop::{Crop, MAX_ANGLE};
use crate::develop::{Develop, DevelopSetting};
use crate::edit::Edit;
use crate::geom::{Rect, Vec2};
use crate::masks::{Mask, MaskShape};
use crate::presets::Preset;

/// The most snapshots and masks a photo keeps, so a damaged or hostile file can't ask for
/// millions of them.
pub const MAX_SNAPSHOTS: usize = 500;
pub const MAX_MASKS: usize = 100;
/// The most points a tone curve may have.
const MAX_CURVE_POINTS: usize = 64;

/// The photo as imported: its file's name and bytes (saved again exactly as they came, so a
/// JPEG is never compressed twice) and the decoded pixels.
pub struct Source {
    pub file_name: String,
    pub bytes: Vec<u8>,
    pub bitmap: Bitmap,
    /// A number no other original read in this run has had (see `id`).
    id: u64,
}

/// The number the next original read gets.
static NEXT_SOURCE_ID: AtomicU64 = AtomicU64::new(1);

impl std::fmt::Debug for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Source({}, {}×{})", self.file_name, self.bitmap.width, self.bitmap.height)
    }
}

impl Source {
    /// An original from its file's name and bytes and its decoded pixels.
    pub fn new(file_name: impl Into<String>, bytes: Vec<u8>, bitmap: Bitmap) -> Source {
        Source { file_name: file_name.into(), bytes, bitmap, id: NEXT_SOURCE_ID.fetch_add(1, Ordering::Relaxed) }
    }

    /// Which original this is: never the same for two originals read in one run, even after one
    /// is closed and its memory reused, so what is kept for one photo (its previews, what was
    /// measured in it) can't be taken for another's.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Decodes a picture file's bytes.
    pub fn decode(file_name: impl Into<String>, bytes: Vec<u8>) -> Result<Source, String> {
        let bitmap = crate::image_io::decode(&bytes)?;
        Ok(Source::new(file_name, bytes, bitmap))
    }

    pub fn size(&self) -> (u32, u32) {
        (self.bitmap.width, self.bitmap.height)
    }
}

/// A named copy of the edit to come back to (Lightroom's Snapshots).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub name: String,
    pub edit: Edit,
}

/// A photo and its edit: the open document.
#[derive(Clone, Debug)]
pub struct Photo {
    pub name: String,
    pub source: Arc<Source>,
    pub edit: Edit,
    pub snapshots: Vec<Snapshot>,
}

/// Two photos are the same when they show the same original (the very same one, not merely
/// equal pixels, which would be slow to compare) with the same edit, name and snapshots. The
/// history compares photos after every change, so this must be quick.
impl PartialEq for Photo {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.source, &other.source) && self.name == other.name && self.edit == other.edit && self.snapshots == other.snapshots
    }
}

impl Photo {
    /// A photo with nothing done to it yet, named after its file.
    pub fn new(source: Source) -> Photo {
        let name = std::path::Path::new(&source.file_name).file_stem().map_or_else(|| "Untitled".to_owned(), |stem| stem.to_string_lossy().into_owned());
        Photo { name, source: Arc::new(source), edit: Edit::default(), snapshots: Vec::new() }
    }

    /// The image's size: the original turned as the crop says.
    pub fn image_size(&self) -> (u32, u32) {
        let (width, height) = self.source.size();
        self.edit.crop.oriented_size(width, height)
    }
}

/// Fixes anything in an edit the rest of the app can't take: values out of range or not
/// numbers at all, curves out of order, too many masks, a crop off the photo. A file is
/// untrusted, so everything read from one goes through here. `size` is the original's size.
pub fn sanitize(edit: &mut Edit, size: (u32, u32)) {
    sanitize_develop(&mut edit.develop);
    let crop = &mut edit.crop;
    crop.quarter_turns %= 4;
    crop.angle = if crop.angle.is_finite() { crop.angle.clamp(-MAX_ANGLE, MAX_ANGLE) } else { 0.0 };
    let image = crop.oriented_size(size.0, size.1);
    crop.rect = crop.rect.filter(|rect| [rect.x, rect.y, rect.width, rect.height].iter().all(|v| v.is_finite()) && rect.width > 0.0 && rect.height > 0.0).map(|rect| crop.constrain(rect, image));
    edit.masks.truncate(MAX_MASKS);
    for mask in &mut edit.masks {
        sanitize_mask(mask);
    }
}

/// Repairs a preset read from the settings file (which can be edited by hand), as `sanitize`
/// repairs an edit: every setting in range and the curves usable, so applying it gives the
/// same look a saved and reopened project would show.
pub fn sanitize_preset(preset: &mut Preset) {
    sanitize_develop(&mut preset.settings);
    preset.name = preset.name.chars().take(200).collect();
    preset.group = preset.group.chars().take(200).collect();
}

/// Every Develop setting brought into its slider's range, and the curves made usable.
fn sanitize_develop(develop: &mut Develop) {
    let mut settings: Vec<DevelopSetting> = DevelopSetting::BASIC.into_iter().chain(DevelopSetting::DETAIL).chain(DevelopSetting::EFFECTS).collect();
    for band in 0..8 {
        settings.extend([DevelopSetting::MixerHue(band), DevelopSetting::MixerSaturation(band), DevelopSetting::MixerLuminance(band)]);
    }
    for wheel in 0..3 {
        settings.extend((0..3).map(|value| DevelopSetting::Grade(wheel, value)));
    }
    settings.extend([DevelopSetting::GradeBlending, DevelopSetting::GradeBalance]);
    for setting in settings {
        // `set` keeps the value in range and puts the default in place of a non-number.
        setting.set(develop, setting.get(develop));
    }
    for curve in [&mut develop.curve, &mut develop.curve_red, &mut develop.curve_green, &mut develop.curve_blue] {
        sanitize_curve(curve);
    }
}

fn sanitize_curve(curve: &mut CurvePoints) {
    curve.retain(|[x, y]| x.is_finite() && y.is_finite());
    for point in curve.iter_mut() {
        *point = point.map(|value| value.clamp(0.0, 1.0));
    }
    curve.sort_by(|a, b| a[0].total_cmp(&b[0]));
    curve.truncate(MAX_CURVE_POINTS);
    if curve.len() < 2 {
        *curve = crate::adjust::identity_curve();
    }
}

fn sanitize_mask(mask: &mut Mask) {
    sanitize_develop(&mut mask.settings);
    mask.amount = if mask.amount.is_finite() { mask.amount.clamp(0.0, 100.0) } else { 100.0 };
    mask.name = mask.name.chars().take(200).collect();
    let point = |point: Vec2| if point.is_finite() { Vec2::new(point.x.clamp(-2.0, 3.0), point.y.clamp(-2.0, 3.0)) } else { Vec2::new(0.5, 0.5) };
    mask.shape = match mask.shape {
        MaskShape::Linear { start, end } => MaskShape::Linear { start: point(start), end: point(end) },
        MaskShape::Radial { center, radius, angle, feather } => MaskShape::Radial {
            center: point(center),
            radius: Vec2::new(finite_or(radius.x, 0.25).clamp(0.005, 3.0), finite_or(radius.y, 0.25).clamp(0.005, 3.0)),
            angle: finite_or(angle, 0.0).rem_euclid(360.0),
            feather: finite_or(feather, 50.0).clamp(0.0, 100.0),
        },
    };
}

fn finite_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}

/// The crop as a rectangle with `ratio` (width over height; 0 for the original photo's), as big
/// as fits and centered on the current crop.
pub fn crop_to_ratio(crop: &Crop, size: (u32, u32), ratio: f32) -> Rect {
    let current = crop.kept(size);
    let ratio = if ratio > 0.0 { ratio } else { size.0 as f32 / size.1.max(1) as f32 };
    // A portrait crop takes the ratio the other way up.
    let ratio = if (current.width >= current.height) == (ratio >= 1.0) { ratio } else { 1.0 / ratio };
    let largest = crate::crop::largest_fitting(size, crop.angle, ratio);
    let center = current.center();
    crop.constrain(Rect::new(center.x - largest.width * 0.5, center.y - largest.height * 0.5, largest.width, largest.height), size)
}

/// The crop turned onto its other side (portrait to landscape or back) about its own middle,
/// made to fit on the photo.
pub fn crop_swapped(crop: &Crop, size: (u32, u32)) -> Rect {
    let rect = crop.kept(size);
    let center = rect.center();
    crop.constrain(Rect::new(center.x - rect.height * 0.5, center.y - rect.width * 0.5, rect.height, rect.width), size)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_preset_from_a_hand_edited_file_is_brought_into_range() {
        let mut preset = Preset::default();
        preset.settings.exposure = 50.0;
        preset.settings.contrast = f32::NAN;
        preset.settings.curve = vec![[f32::INFINITY, 0.0]];
        sanitize_preset(&mut preset);
        assert_eq!(preset.settings.exposure, 5.0);
        assert_eq!(preset.settings.contrast, 0.0);
        assert_eq!(preset.settings.curve, crate::adjust::identity_curve());
    }

    #[test]
    fn photos_compare_by_their_original_and_edit() {
        let source = Source::new("IMG_0001.jpg", Vec::new(), Bitmap::new(4, 4));
        let photo = Photo::new(source);
        assert_eq!(photo.name, "IMG_0001");
        let mut changed = photo.clone();
        assert_eq!(changed, photo);
        changed.edit.develop.exposure = 1.0;
        assert_ne!(changed, photo);
        // The same pixels from another import are another photo.
        let other = Photo::new(Source::new("IMG_0001.jpg", Vec::new(), Bitmap::new(4, 4)));
        assert_ne!(other, photo);
    }

    #[test]
    fn sanitizing_repairs_every_kind_of_bad_value() {
        let mut edit = Edit::default();
        edit.develop.exposure = f32::NAN;
        edit.develop.contrast = 9_000.0;
        edit.develop.curve = vec![[2.0, f32::INFINITY]];
        edit.crop.quarter_turns = 7;
        edit.crop.angle = 400.0;
        edit.crop.rect = Some(Rect::new(-50.0, -50.0, 1e9, 10.0));
        let mask = Mask { amount: -5.0, shape: MaskShape::Radial { center: Vec2::new(f32::NAN, 0.5), radius: Vec2::new(0.0, 99.0), angle: -30.0, feather: 500.0 }, ..Mask::default() };
        edit.masks = vec![mask; MAX_MASKS + 5];
        sanitize(&mut edit, (100, 50));
        assert_eq!(edit.develop.exposure, 0.0);
        assert_eq!(edit.develop.contrast, 100.0);
        assert_eq!(edit.develop.curve, crate::adjust::identity_curve());
        assert_eq!(edit.crop.quarter_turns, 3);
        assert_eq!(edit.crop.angle, MAX_ANGLE);
        let rect = edit.crop.rect.unwrap();
        assert!(edit.crop.fits(rect, edit.crop.oriented_size(100, 50)), "{rect:?}");
        assert_eq!(edit.masks.len(), MAX_MASKS);
        assert_eq!(edit.masks[0].amount, 0.0);
        assert!(matches!(edit.masks[0].shape, MaskShape::Radial { center, feather, angle, .. } if center.is_finite() && feather == 100.0 && angle == 330.0));
    }

    #[test]
    fn ratios_follow_the_crop_and_fit() {
        let size = (600, 400);
        let crop = Crop::default();
        let square = crop_to_ratio(&crop, size, 1.0);
        assert!((square.width - square.height).abs() < 0.5 && (square.height - 400.0).abs() < 0.5, "{square:?}");
        // A portrait crop asked for 3 × 2 becomes 2 × 3.
        let portrait = Crop { rect: Some(Rect::new(250.0, 0.0, 100.0, 400.0)), ..Crop::default() };
        let tall = crop_to_ratio(&portrait, size, 1.5);
        assert!(tall.height > tall.width && crop.fits(tall, size), "{tall:?}");
    }
}

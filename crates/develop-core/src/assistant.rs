//! Iris for photos: a set of plain rules that measure the finished photo and point out what a
//! photographer would usually want to fix (blown highlights, crushed shadows, too much
//! contrast, a color cast and so on), each with a one-click fix. As in Studio, nothing here
//! guesses or learns: every suggestion comes from a rule that can be switched off and says
//! what it measured, so it can be judged rather than taken on trust.
//!
//! The rules read a small render of the finished photo (`Measure::of`), and for the color
//! cast also the photo as it came (`neutral_color`), whose grays the white balance is set from.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::bitmap::Bitmap;
use crate::color::srgb_to_linear;
use crate::develop::{Develop, DevelopSetting};
use crate::edit::Edit;
use crate::geom::Vec2;
use crate::masks::{Mask, MaskShape};

/// What a rule is about. Suggestions are grouped and colored by it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Area {
    Light,
    Contrast,
    Color,
    Detail,
}

impl Area {
    pub const ALL: [Self; 4] = [Self::Light, Self::Contrast, Self::Color, Self::Detail];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Light => "Light",
            Self::Contrast => "Contrast",
            Self::Color => "Color",
            Self::Detail => "Detail",
        }
    }
}

/// One of the rules.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Check {
    Overexposed,
    Underexposed,
    BlownHighlights,
    CrushedShadows,
    TooContrasty,
    Flat,
    Oversaturated,
    Dull,
    ColorCast,
    Overprocessed,
    BrightSky,
    NoisyShadows,
}

impl Check {
    pub const ALL: [Self; 12] = [
        Self::Overexposed,
        Self::Underexposed,
        Self::BlownHighlights,
        Self::CrushedShadows,
        Self::TooContrasty,
        Self::Flat,
        Self::Oversaturated,
        Self::Dull,
        Self::ColorCast,
        Self::Overprocessed,
        Self::BrightSky,
        Self::NoisyShadows,
    ];

    pub const fn area(self) -> Area {
        match self {
            Self::Overexposed | Self::Underexposed | Self::BlownHighlights | Self::CrushedShadows | Self::BrightSky => Area::Light,
            Self::TooContrasty | Self::Flat => Area::Contrast,
            Self::Oversaturated | Self::Dull | Self::ColorCast => Area::Color,
            Self::Overprocessed | Self::NoisyShadows => Area::Detail,
        }
    }

    /// The rule's name, as shown beside its switch.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Overexposed => "Too bright overall",
            Self::Underexposed => "Too dark overall",
            Self::BlownHighlights => "Highlights blown to white",
            Self::CrushedShadows => "Shadows crushed to black",
            Self::TooContrasty => "Too much contrast",
            Self::Flat => "Flat, washed-out tones",
            Self::Oversaturated => "Colors too strong",
            Self::Dull => "Colors dull",
            Self::ColorCast => "A color cast",
            Self::Overprocessed => "Settings pushed far enough to show",
            Self::BrightSky => "A sky much brighter than the land",
            Self::NoisyShadows => "Lifted shadows showing noise",
        }
    }

    /// What the rule looks for, in a sentence.
    pub const fn about(self) -> &'static str {
        match self {
            Self::Overexposed => "The middle of the photo's brightness sits well above middle gray.",
            Self::Underexposed => "The middle of the photo's brightness sits well below middle gray.",
            Self::BlownHighlights => "A noticeable share of the photo is pure white, with no detail left in it.",
            Self::CrushedShadows => "A noticeable share of the photo is pure black, with no detail left in it.",
            Self::TooContrasty => "Tones bunched at both ends, with little in between, and both ends clipping.",
            Self::Flat => "The darkest and lightest tones sit far from black and white.",
            Self::Oversaturated => "Many colors at or near full strength, which looks garish and loses detail.",
            Self::Dull => "Very little color in a photo that isn't meant to be black and white.",
            Self::ColorCast => "Things that should be gray lean toward one color.",
            Self::Overprocessed => "Clarity, Dehaze, Texture or Sharpening set high enough to cause halos or grit.",
            Self::BrightSky => "The top of the photo is far brighter than the rest, as a sky often is over a landscape.",
            Self::NoisyShadows => "Shadows or exposure raised a long way with little noise reduction, which brings up grain.",
        }
    }
}

/// Which rules run. Saved with the app's settings: a rule is on unless listed here.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Options {
    pub off: BTreeSet<Check>,
}

impl Options {
    pub fn runs(&self, check: Check) -> bool {
        !self.off.contains(&check)
    }

    pub fn set(&mut self, check: Check, on: bool) {
        if on {
            self.off.remove(&check);
        } else {
            self.off.insert(check);
        }
    }
}

/// A fix: Develop settings to set, each to a value, and sometimes a mask to add.
#[derive(Clone, Debug, PartialEq)]
pub struct Fix {
    /// The name on the fix's button, and of its step in the History panel.
    pub label: String,
    pub changes: Vec<(DevelopSetting, f32)>,
    /// A mask the fix adds (a graduated filter over a bright sky, say).
    pub mask: Option<Mask>,
}

impl Fix {
    /// A fix that sets these settings.
    pub fn settings(label: impl Into<String>, changes: Vec<(DevelopSetting, f32)>) -> Fix {
        Fix { label: label.into(), changes, mask: None }
    }

    /// What the fix sets, in words: "Contrast +25, Whites +36", and the mask it adds.
    pub fn summary(&self) -> String {
        let mut parts: Vec<String> = self
            .changes
            .iter()
            .map(|(setting, value)| {
                let info = setting.slider();
                format!("{} {:+.*}", info.name, info.decimals, value)
            })
            .collect();
        if let Some(mask) = &self.mask {
            parts.push(format!("adds a {} mask ({})", mask.shape.name(), mask.name));
        }
        parts.join(", ")
    }

    /// `edit` with this fix made: its settings set and its mask added.
    pub fn apply_to_edit(&self, edit: &Edit) -> Edit {
        let mut out = edit.clone();
        out.develop = self.apply_to(&edit.develop);
        if let Some(mask) = &self.mask {
            out.masks.push(mask.clone());
        }
        out
    }

    /// `develop` with this fix's settings set.
    pub fn apply_to(&self, develop: &Develop) -> Develop {
        let mut out = develop.clone();
        for (setting, value) in &self.changes {
            setting.set(&mut out, *value);
        }
        out
    }
}

/// One thing Iris noticed.
#[derive(Clone, Debug, PartialEq)]
pub struct Suggestion {
    pub check: Check,
    /// What was noticed, in a few words.
    pub title: String,
    /// Why it matters and what the fix does.
    pub detail: String,
    /// What was measured, so the suggestion can be judged.
    pub evidence: String,
    pub fix: Option<Fix>,
}

/// What the rules read from a render of the finished photo. Brightness is perceived
/// brightness (sRGB luma), 0–1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Measure {
    /// Share of pixels (0–1) with a channel at white.
    pub highlights_clipped: f32,
    /// Share of pixels with every channel at black.
    pub shadows_clipped: f32,
    /// The brightness half the photo is darker than.
    pub median: f32,
    /// The brightness 2% of the photo is darker than, and 98%.
    pub low: f32,
    pub high: f32,
    /// How spread out the brightness is (its standard deviation).
    pub spread: f32,
    /// Share of the pixels in the darkest and the lightest tenth of the range.
    pub ends: f32,
    /// Average saturation, 0–1.
    pub saturation: f32,
    /// Share of pixels whose color is at full strength (a channel at white and one near black).
    pub saturated_clipped: f32,
    /// The middle brightness of the top quarter of the photo, and of its bottom half.
    pub top: f32,
    pub bottom: f32,
    /// The average color of the near-gray parts, if there are enough of them.
    pub neutral: Option<[f32; 3]>,
}

/// The brightness of an sRGB color, 0–1.
fn luma(rgb: [f32; 3]) -> f32 {
    0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]
}

/// How colorful a color is, 0 (gray) to 1.
fn chroma(rgb: [f32; 3]) -> f32 {
    let max = rgb[0].max(rgb[1]).max(rgb[2]);
    let min = rgb[0].min(rgb[1]).min(rgb[2]);
    if max > 1e-3 { (max - min) / max } else { 0.0 }
}

/// Every opaque pixel as a straight sRGB color, sampled on a grid so big pictures stay quick.
fn colors(bitmap: &Bitmap) -> Vec<[f32; 3]> {
    colors_in_rows(bitmap, 0.0, 1.0)
}

/// The opaque pixels between `from` and `to` of the way down (0 at the top, 1 at the foot).
fn colors_in_rows(bitmap: &Bitmap, from: f32, to: f32) -> Vec<[f32; 3]> {
    let pixels = bitmap.width as usize * bitmap.height as usize;
    let step = ((pixels / 250_000).max(1) as f32).sqrt().ceil() as usize;
    let mut out = Vec::new();
    let height = bitmap.height as f32;
    let (first, last) = ((height * from).floor() as i32, (height * to).ceil() as i32);
    for y in (first.max(0)..last.min(bitmap.height as i32)).step_by(step) {
        for x in (0..bitmap.width as i32).step_by(step) {
            let [r, g, b, a] = bitmap.pixel(x, y);
            if a < 128 {
                continue;
            }
            let straight = |channel: u8| (f32::from(channel) / f32::from(a)).min(1.0);
            out.push([straight(r), straight(g), straight(b)]);
        }
    }
    out
}

/// The average color of the near-gray, mid-bright parts of `bitmap`, if at least a twentieth of
/// it is near gray. A cast tints exactly these, so their average says which way and how much.
///
/// Only the lower 60% of the frame is read: a pale sky is near gray but meant to be blue, and
/// counting it would call every landscape "cool". Pale walls, paper, shirts and ground, where
/// a real cast shows, are mostly below the sky.
pub fn neutral_color(bitmap: &Bitmap) -> Option<[f32; 3]> {
    let found = colors_in_rows(bitmap, NEUTRAL_FROM, 1.0);
    let neutral: Vec<[f32; 3]> = found.iter().copied().filter(|rgb| (0.2..0.9).contains(&luma(*rgb)) && chroma(*rgb) < NEUTRAL_CHROMA).collect();
    if found.is_empty() || (neutral.len() as f32) < found.len() as f32 * 0.05 {
        return None;
    }
    let count = neutral.len() as f32;
    let sum = neutral.iter().fold([0.0f32; 3], |sum, rgb| [sum[0] + rgb[0], sum[1] + rgb[1], sum[2] + rgb[2]]);
    Some(sum.map(|channel| channel / count))
}

/// How far down the frame the near-gray parts are read from, so a sky isn't taken for a cast.
const NEUTRAL_FROM: f32 = 0.4;
/// How colorful a color may be and still count as near gray. A real cast leaves grays this
/// tinted at most; anything stronger is a color of its own (foliage, water, a red car).
const NEUTRAL_CHROMA: f32 = 0.25;
/// How far near-gray parts must lean warm (red over blue), cool, or green/magenta to be a cast.
const WARM_CAST: f32 = 0.05;
const COOL_CAST: f32 = 0.1;
/// Green leans need more: natural greens (grass, leaves, pale hills) sit near gray and pull the
/// average green, while a real green cast (old fluorescent tubes) is strong.
const GREEN_CAST: f32 = 0.07;
/// How far Exposure may already be moved (in stops, either way) before the light rules leave it.
const ENOUGH_STOPS: f32 = 1.0;
/// How much Contrast may already be added before the flat rule leaves it.
const ENOUGH_CONTRAST: f32 = 20.0;
/// The share of the photo clipped to white past which it counts as badly blown.
const HEAVILY_CLIPPED: f32 = 0.25;
/// The most the overexposure fix darkens a badly blown photo, in stops.
const MOST_STOPS_WHEN_BLOWN: f32 = 1.0;
/// The most the color-cast fix moves Temperature or Tint, so a misjudged photo is never wrecked.
const MOST_BALANCE: f32 = 80.0;

/// The brightness half of `colors` are darker than.
fn median_brightness(colors: &[[f32; 3]]) -> f32 {
    let mut brightness: Vec<f32> = colors.iter().map(|rgb| luma(*rgb)).collect();
    brightness.sort_by(f32::total_cmp);
    brightness.get(brightness.len() / 2).copied().unwrap_or(0.5)
}

impl Measure {
    /// Measures a render of the finished photo.
    pub fn of(bitmap: &Bitmap) -> Measure {
        let found = colors(bitmap);
        let total = found.len().max(1) as f32;
        let mut brightness: Vec<f32> = found.iter().map(|rgb| luma(*rgb)).collect();
        brightness.sort_by(f32::total_cmp);
        let at = |fraction: f32| brightness.get(((brightness.len() as f32 - 1.0) * fraction).round().max(0.0) as usize).copied().unwrap_or(0.5);
        let mean = brightness.iter().sum::<f32>() / total;
        let spread = (brightness.iter().map(|value| (value - mean).powi(2)).sum::<f32>() / total).sqrt();
        let share = |test: &dyn Fn(&[f32; 3]) -> bool| found.iter().filter(|rgb| test(rgb)).count() as f32 / total;
        let white = 252.5 / 255.0;
        let black = 2.5 / 255.0;
        Measure {
            highlights_clipped: share(&|rgb| rgb.iter().any(|channel| *channel >= white) && luma(*rgb) > 0.7),
            shadows_clipped: share(&|rgb| rgb.iter().all(|channel| *channel <= black)),
            median: at(0.5),
            low: at(0.02),
            high: at(0.98),
            spread,
            ends: share(&|rgb| !(0.1..=0.9).contains(&luma(*rgb))),
            saturation: found.iter().map(|rgb| chroma(*rgb)).sum::<f32>() / total,
            saturated_clipped: share(&|rgb| chroma(*rgb) > 0.95 && rgb.iter().any(|channel| *channel >= white)),
            top: median_brightness(&colors_in_rows(bitmap, 0.0, 0.25)),
            bottom: median_brightness(&colors_in_rows(bitmap, 0.5, 1.0)),
            neutral: neutral_color(bitmap),
        }
    }
}

/// The exposure change (in stops) that brings brightness `from` to `to`, gently: a little short
/// of the whole way, since tone rarely wants to land exactly on middle gray.
fn stops_between(from: f32, to: f32) -> f32 {
    (srgb_to_linear(to).max(1e-4) / srgb_to_linear(from).max(1e-4)).log2() * 0.85
}

/// A value rounded to a step, as the sliders show it.
fn rounded(value: f32, step: f32) -> f32 {
    (value / step).round() * step
}

fn percent(share: f32) -> String {
    if share < 0.001 { "under 0.1%".to_owned() } else { format!("{:.1}%", share * 100.0) }
}

/// What the rules notice in the finished photo (measured as `measure`) made with `edit`.
/// `original_neutral` is the near-gray average of the photo as it came, which the color-cast
/// fix sets the white balance from.
pub fn suggestions(measure: &Measure, original_neutral: Option<[f32; 3]>, edit: &Edit, options: &Options) -> Vec<Suggestion> {
    let develop = &edit.develop;
    let mut found = Vec::new();
    let mut add = |check: Check, title: String, detail: &str, evidence: String, fix: Option<Fix>| {
        if options.runs(check) {
            found.push(Suggestion { check, title, detail: detail.to_owned(), evidence, fix });
        }
    };
    let m = measure;
    let brightness = format!("Middle brightness {:.0}%, darkest 2% below {:.0}%, lightest 2% above {:.0}%.", m.median * 100.0, m.low * 100.0, m.high * 100.0);

    // Light.
    // Once Exposure has been moved a long way (by a fix, or by hand), going further is a matter
    // of taste, not a problem, so the light rules stop suggesting more.
    if m.median > 0.66 && (m.highlights_clipped > 0.01 || m.low > 0.25) && develop.exposure > -ENOUGH_STOPS {
        // Where much of the photo is pure white, the middle brightness says little (it's white
        // too), and taking it toward gray only turns the blown parts flat gray: so less, then.
        let badly_blown = m.highlights_clipped > HEAVILY_CLIPPED;
        let darker = stops_between(m.median, 0.5).max(if badly_blown { -MOST_STOPS_WHEN_BLOWN } else { -5.0 });
        let exposure = rounded((develop.exposure + darker).clamp(-5.0, 5.0), 0.05);
        let highlights = develop.highlights.min(-30.0);
        let detail = if badly_blown {
            "Most of it is brighter than it should be, and a good part is pure white: detail there is gone and can't be brought back, only toned down. A little less Exposure and Highlights pulled back recover what's left."
        } else {
            "Most of it is brighter than it should be, so light areas run together. Lower Exposure brings the middle tones down; pulling Highlights back recovers the brightest parts."
        };
        let evidence = if badly_blown { format!("{brightness} {} is pure white.", percent(m.highlights_clipped)) } else { brightness.clone() };
        add(
            Check::Overexposed,
            "The photo looks overexposed".to_owned(),
            detail,
            evidence,
            Some(Fix::settings(format!("Exposure {exposure:+.2}, Highlights {highlights:.0}"), vec![(DevelopSetting::Exposure, exposure), (DevelopSetting::Highlights, highlights)])),
        );
    } else if m.median < 0.24 && m.high < 0.85 && develop.exposure < ENOUGH_STOPS {
        let exposure = rounded((develop.exposure + stops_between(m.median, 0.42)).clamp(-5.0, 5.0), 0.05);
        let shadows = develop.shadows.max(25.0);
        add(
            Check::Underexposed,
            "The photo looks underexposed".to_owned(),
            "Most of it is darker than it should be, so detail hides in the dark. Raising Exposure lifts the middle tones; opening Shadows brings out the darkest parts.",
            brightness.clone(),
            Some(Fix::settings(format!("Exposure {exposure:+.2}, Shadows +{shadows:.0}"), vec![(DevelopSetting::Exposure, exposure), (DevelopSetting::Shadows, shadows)])),
        );
    }
    if m.highlights_clipped > 0.02 {
        let highlights = (develop.highlights - 40.0).clamp(-100.0, -40.0);
        let whites = (develop.whites - 20.0).max(-100.0);
        add(
            Check::BlownHighlights,
            "Highlights are blown out".to_owned(),
            "Part of the photo is pure white, with no detail in it (a sky, a window, skin in the sun). Lowering Highlights and Whites brings back what the photo still holds there.",
            format!("{} of the photo is clipped to white.", percent(m.highlights_clipped)),
            Some(Fix::settings(format!("Highlights {highlights:.0}, Whites {whites:.0}"), vec![(DevelopSetting::Highlights, highlights), (DevelopSetting::Whites, whites)])),
        );
    }
    if m.shadows_clipped > 0.03 {
        let shadows = (develop.shadows + 35.0).clamp(35.0, 100.0);
        let blacks = (develop.blacks + 15.0).min(100.0);
        add(
            Check::CrushedShadows,
            "Shadows are crushed to black".to_owned(),
            "Part of the photo is pure black, with no detail in it. Raising Shadows and Blacks opens the darkest parts back up.",
            format!("{} of the photo is clipped to black.", percent(m.shadows_clipped)),
            Some(Fix::settings(format!("Shadows +{shadows:.0}, Blacks {blacks:+.0}"), vec![(DevelopSetting::Shadows, shadows), (DevelopSetting::Blacks, blacks)])),
        );
    }

    // Contrast.
    if m.spread > 0.3 && m.ends > 0.45 && (m.highlights_clipped > 0.005 || m.shadows_clipped > 0.005) {
        let contrast = (develop.contrast - 30.0).max(-100.0);
        let highlights = (develop.highlights - 25.0).max(-100.0);
        let shadows = (develop.shadows + 25.0).min(100.0);
        add(
            Check::TooContrasty,
            "There's too much contrast".to_owned(),
            "Most of the photo sits at the very dark or very light end, with little in between, so it looks harsh and both ends lose detail. Less Contrast, with Highlights down and Shadows up, fills in the middle.",
            format!("{} of the photo is in the darkest or lightest tenth; brightness spread {:.2}.", percent(m.ends), m.spread),
            Some(Fix::settings(format!("Contrast {contrast:+.0}, Highlights {highlights:.0}, Shadows {shadows:+.0}"), vec![(DevelopSetting::Contrast, contrast), (DevelopSetting::Highlights, highlights), (DevelopSetting::Shadows, shadows)])),
        );
    } else if m.high - m.low < 0.55 && m.highlights_clipped < 0.005 && m.shadows_clipped < 0.005 && develop.contrast < ENOUGH_CONTRAST {
        let contrast = (develop.contrast + 25.0).min(100.0);
        // Whites and Blacks stretch the range toward its ends, more the further they fall short.
        let whites = rounded((develop.whites + (0.95 - m.high).max(0.0) * 150.0).min(60.0), 1.0);
        let blacks = rounded((develop.blacks - (m.low - 0.05).max(0.0) * 150.0).max(-60.0), 1.0);
        add(
            Check::Flat,
            "The photo looks flat".to_owned(),
            "Its darkest tones are well above black and its lightest well below white, so it looks hazy or washed out. More Contrast, with Whites up and Blacks down, gives it depth.",
            brightness.clone(),
            Some(Fix::settings(format!("Contrast +{contrast:.0}, Whites {whites:+.0}, Blacks {blacks:+.0}"), vec![(DevelopSetting::Contrast, contrast), (DevelopSetting::Whites, whites), (DevelopSetting::Blacks, blacks)])),
        );
    }

    // Color.
    let black_and_white = develop.saturation <= -95.0;
    if !black_and_white && (m.saturation > 0.6 || m.saturated_clipped > 0.04) {
        let saturation = (develop.saturation - 25.0).max(-100.0);
        add(
            Check::Oversaturated,
            "Colors are too strong".to_owned(),
            "Many colors are at full strength, which looks garish and flattens detail in them. Lowering Saturation calms every color; Vibrance is the gentler way to add color back where it's missing.",
            format!("Average saturation {:.0}%; {} of the photo at full color.", m.saturation * 100.0, percent(m.saturated_clipped)),
            Some(Fix::settings(format!("Saturation {saturation:+.0}"), vec![(DevelopSetting::Saturation, saturation)])),
        );
    } else if !black_and_white && m.saturation < 0.1 && develop.saturation > -50.0 {
        let vibrance = (develop.vibrance + 30.0).min(100.0);
        add(
            Check::Dull,
            "Colors look dull".to_owned(),
            "There is very little color in the photo. Vibrance strengthens the quieter colors most and leaves skin and already-strong colors nearly alone. (If you're after black and white, Saturation all the way down makes it.)",
            format!("Average saturation {:.0}%.", m.saturation * 100.0),
            Some(Fix::settings(format!("Vibrance +{vibrance:.0}"), vec![(DevelopSetting::Vibrance, vibrance)])),
        );
    }
    // A cast only counts while the white balance hasn't been set by hand, so a warm look chosen
    // on purpose isn't "corrected".
    let balanced_by_hand = develop.temperature.abs() > 0.5 || develop.tint.abs() > 0.5;
    if !black_and_white
        && !balanced_by_hand
        && let Some(neutral) = m.neutral
    {
        let warm = neutral[0] - neutral[2];
        let green = neutral[1] - (neutral[0] + neutral[2]) * 0.5;
        // A warm lean is the classic cast (indoor bulbs, late light) and counts early. A slight
        // cool lean is usually sky or water in the frame, so only a strong one counts.
        let cast = warm > WARM_CAST || warm < -COOL_CAST || green.abs() > GREEN_CAST;
        if cast {
            let lean = match (warm.abs() >= green.abs() * 1.4, warm > 0.0, green > 0.0) {
                (true, true, _) => "warm (yellow-orange)",
                (true, false, _) => "cool (blue)",
                (false, _, true) => "green",
                (false, _, false) => "magenta",
            };
            let fix = original_neutral.and_then(|color| {
                let mut balanced = develop.clone();
                balanced.neutralize(color).then(|| {
                    let temperature = balanced.temperature.clamp(-MOST_BALANCE, MOST_BALANCE);
                    let tint = balanced.tint.clamp(-MOST_BALANCE, MOST_BALANCE);
                    Fix::settings(format!("Temperature {temperature:+.0}, Tint {tint:+.0}"), vec![(DevelopSetting::Temperature, temperature), (DevelopSetting::Tint, tint)])
                })
            });
            add(
                Check::ColorCast,
                format!("The photo has a {lean} cast"),
                "Things that should be gray or white lean toward one color, usually from the light it was taken in. Setting the white balance from the photo's grays makes them neutral; the White Balance Selector does the same from one spot you pick.",
                format!("Near-gray parts average red {:.0}%, green {:.0}%, blue {:.0}%.", neutral[0] * 100.0, neutral[1] * 100.0, neutral[2] * 100.0),
                fix,
            );
        }
    }

    // Detail: settings that show their seams when pushed.
    let pushed: Vec<(DevelopSetting, f32, f32)> = [
        (DevelopSetting::Clarity, develop.clarity, 60.0),
        (DevelopSetting::Dehaze, develop.dehaze, 55.0),
        (DevelopSetting::Texture, develop.texture, 70.0),
        (DevelopSetting::SharpenAmount, develop.sharpen_amount, 110.0),
    ]
    .into_iter()
    .filter(|(_, value, limit)| *value > *limit)
    .collect();
    if !pushed.is_empty() {
        let names: Vec<String> = pushed.iter().map(|(setting, value, _)| format!("{} {value:.0}", setting.slider().name)).collect();
        add(
            Check::Overprocessed,
            "Some settings are pushed hard".to_owned(),
            "Set this high, Clarity and Dehaze leave halos around edges, and Texture and Sharpening bring up grit and noise. Easing them back keeps most of the punch without the seams.",
            format!("{}.", names.join(", ")),
            Some(Fix::settings("Ease Them Back", pushed.iter().map(|(setting, value, limit)| (*setting, rounded((*value).min(*limit * 0.75), 1.0))).collect())),
        );
    }
    // A graduated filter over a bright sky: Lightroom's oldest trick. Not when the whole photo
    // is bright (that's overexposure) or a mask already darkens the top.
    let sky_masked = edit.masks.iter().any(|mask| mask.is_active() && matches!(mask.shape, MaskShape::Linear { start, end } if start.y < end.y && start.y < 0.3));
    if !sky_masked && m.top > 0.72 && m.top - m.bottom > 0.3 && m.median < 0.66 {
        let exposure = rounded(stops_between(m.top, 0.6).clamp(-2.0, 0.0), 0.05);
        let mut sky = Mask::new("Sky", MaskShape::linear(Vec2::new(0.5, 0.0), Vec2::new(0.5, 0.45)));
        sky.settings.exposure = exposure;
        sky.settings.highlights = -40.0;
        sky.settings.dehaze = 10.0;
        add(
            Check::BrightSky,
            "The sky is much brighter than the land".to_owned(),
            "Exposing for the land leaves the sky pale, and darkening the whole photo would sink the land. A Linear Gradient mask darkens just the top, fading out toward the horizon, as a photographer's graduated filter does.",
            format!("Top quarter's middle brightness {:.0}%, bottom half's {:.0}%.", m.top * 100.0, m.bottom * 100.0),
            Some(Fix { label: format!("Add a Sky Mask (Exposure {exposure:+.2})"), changes: Vec::new(), mask: Some(sky) }),
        );
    }
    // Lifting the shadows a long way lifts their noise with them.
    let lifted = develop.shadows > 60.0 || develop.exposure > 1.5 || (develop.shadows > 35.0 && develop.exposure > 0.8);
    if lifted && develop.noise_luminance < 15.0 {
        add(
            Check::NoisyShadows,
            "Lifted shadows may show noise".to_owned(),
            "Brightening dark areas this much brings up their grain and color speckle. A little Luminance and Color noise reduction smooths it while keeping edges.",
            format!("Shadows {:+.0}, Exposure {:+.2}, Luminance noise reduction {:.0}.", develop.shadows, develop.exposure, develop.noise_luminance),
            Some(Fix::settings("Noise Reduction 25", vec![(DevelopSetting::NoiseLuminance, 25.0), (DevelopSetting::NoiseColor, develop.noise_color.max(25.0))])),
        );
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Rgba;
    use crate::color::linear_to_srgb;
    use crate::develop::develop;

    /// A neutral `srgb` brightness lifted by `stops` of exposure.
    fn lifted(srgb: f32, stops: f32) -> f32 {
        linear_to_srgb(srgb_to_linear(srgb) * 2f32.powf(stops))
    }

    /// A gentle gradient scene of gray tones from `low` to `high` brightness with some color,
    /// 200 × 100.
    fn scene(low: f32, high: f32, tint: [f32; 3]) -> Bitmap {
        let mut bitmap = Bitmap::new(200, 100);
        for x in 0..200 {
            for y in 0..100 {
                let t = (x as f32 / 199.0 + y as f32 / 99.0) * 0.5;
                let v = low + (high - low) * t;
                let color = Rgba::rgb((v * tint[0]).min(1.0), (v * tint[1]).min(1.0), (v * tint[2]).min(1.0));
                bitmap.set_pixel(x, y, color.to_premultiplied());
            }
        }
        bitmap
    }

    fn checks(bitmap: &Bitmap, develop: &Develop) -> Vec<Check> {
        suggestions(&Measure::of(bitmap), neutral_color(bitmap), &Edit { develop: develop.clone(), ..Edit::default() }, &Options::default()).into_iter().map(|found| found.check).collect()
    }

    #[test]
    fn a_well_exposed_photo_gets_no_light_suggestions() {
        let photo = scene(0.05, 0.95, [1.0, 1.0, 1.0]);
        let found = checks(&photo, &Develop::default());
        assert!(!found.iter().any(|check| check.area() == Area::Light || check.area() == Area::Contrast), "{found:?}");
    }

    #[test]
    fn an_overexposed_photo_is_noticed_and_its_fix_brings_it_down() {
        let photo = scene(0.45, 1.0, [1.0, 1.0, 1.0]);
        let measure = Measure::of(&photo);
        let found = suggestions(&measure, None, &Edit::default(), &Options::default());
        let over = found.iter().find(|found| found.check == Check::Overexposed).expect("overexposed is noticed");
        let fixed = over.fix.as_ref().unwrap().apply_to(&Develop::default());
        assert!(fixed.exposure < 0.0 && fixed.highlights < 0.0, "{fixed:?}");
        // Developed with the fix, the photo is no longer overexposed.
        let after = Measure::of(&develop(&photo, &fixed, 1.0));
        assert!(after.median < measure.median - 0.1, "{} → {}", measure.median, after.median);
        assert!(!suggestions(&after, None, &Edit { develop: fixed.clone(), ..Edit::default() }, &Options::default()).iter().any(|found| found.check == Check::Overexposed));
    }

    #[test]
    fn a_badly_blown_photo_is_toned_down_gently_and_told_its_detail_is_gone() {
        // Two-thirds pure white.
        let mut photo = Bitmap::filled(200, 150, Rgba::WHITE);
        for y in 100..150 {
            for x in 0..200 {
                photo.set_pixel(x, y, [120, 150, 110, 255]);
            }
        }
        let found = suggestions(&Measure::of(&photo), None, &Edit::default(), &Options::default());
        let over = found.iter().find(|found| found.check == Check::Overexposed).expect("noticed");
        let exposure = over.fix.as_ref().unwrap().changes[0].1;
        assert!((-1.0..0.0).contains(&exposure), "{exposure}");
        assert!(over.detail.contains("can't be brought back"), "{}", over.detail);
    }

    #[test]
    fn rules_stop_asking_for_more_once_their_control_has_moved_far() {
        let bright = scene(0.45, 1.0, [1.0, 1.0, 1.0]);
        let darkened = Edit { develop: Develop { exposure: -1.0, ..Develop::default() }, ..Edit::default() };
        assert!(!suggestions(&Measure::of(&bright), None, &darkened, &Options::default()).iter().any(|found| found.check == Check::Overexposed));
        let flat = scene(0.35, 0.65, [1.0, 1.0, 1.0]);
        let punchier = Edit { develop: Develop { contrast: 25.0, ..Develop::default() }, ..Edit::default() };
        assert!(!suggestions(&Measure::of(&flat), None, &punchier, &Options::default()).iter().any(|found| found.check == Check::Flat));
    }

    #[test]
    fn pale_green_hills_are_not_a_green_cast() {
        let hills = scene(0.3, 0.85, [0.9, 1.0, 0.92]);
        assert!(!checks(&hills, &Develop::default()).contains(&Check::ColorCast));
    }

    #[test]
    fn an_underexposed_photo_is_noticed_and_brightened() {
        let photo = scene(0.0, 0.4, [1.0, 1.0, 1.0]);
        let found = suggestions(&Measure::of(&photo), None, &Edit::default(), &Options::default());
        let under = found.iter().find(|found| found.check == Check::Underexposed).expect("underexposed is noticed");
        let fixed = under.fix.as_ref().unwrap().apply_to(&Develop::default());
        assert!(fixed.exposure > 0.3, "{}", fixed.exposure);
        assert!(lifted(0.2, fixed.exposure) > 0.3);
    }

    #[test]
    fn harsh_contrast_and_flat_photos_are_told_apart() {
        // Half black, half white, with a thin band between: harsh.
        let mut harsh = Bitmap::filled(200, 100, Rgba::rgb(0.0, 0.0, 0.0));
        for x in 100..200 {
            for y in 0..100 {
                harsh.set_pixel(x, y, [255, 255, 255, 255]);
            }
        }
        for x in 95..105 {
            for y in 0..100 {
                harsh.set_pixel(x, y, [128, 128, 128, 255]);
            }
        }
        let found = checks(&harsh, &Develop::default());
        assert!(found.contains(&Check::TooContrasty), "{found:?}");
        assert!(!found.contains(&Check::Flat));
        let flat = checks(&scene(0.35, 0.65, [1.0, 1.0, 1.0]), &Develop::default());
        assert!(flat.contains(&Check::Flat) && !flat.contains(&Check::TooContrasty), "{flat:?}");
    }

    #[test]
    fn a_warm_cast_is_found_and_its_fix_neutralizes_it() {
        let photo = scene(0.25, 0.8, [1.08, 1.0, 0.82]);
        let measure = Measure::of(&photo);
        let found = suggestions(&measure, neutral_color(&photo), &Edit::default(), &Options::default());
        let cast = found.iter().find(|found| found.check == Check::ColorCast).expect("cast is noticed");
        assert!(cast.title.contains("warm"), "{}", cast.title);
        let fixed = cast.fix.as_ref().unwrap().apply_to(&Develop::default());
        assert!(fixed.temperature < 0.0);
        let after = Measure::of(&develop(&photo, &fixed, 1.0));
        let neutral = after.neutral.unwrap();
        assert!((neutral[0] - neutral[2]).abs() < 0.03, "{neutral:?}");
        // Once the white balance has been set by hand, the rule leaves it alone.
        assert!(!suggestions(&after, None, &Edit { develop: fixed.clone(), ..Edit::default() }, &Options::default()).iter().any(|found| found.check == Check::ColorCast));
    }

    #[test]
    fn a_blue_sky_over_green_land_is_not_a_cast() {
        // Pale blue sky above, green hills and blue water below: natural colors, no cast.
        let mut photo = Bitmap::new(200, 150);
        for y in 0..150 {
            for x in 0..200 {
                let pixel = if y < 60 {
                    let v = 150 + y as u8;
                    [v - 25, v - 5, v + 20, 255]
                } else if y < 110 {
                    [60 + (x % 20) as u8, 95 + (x % 20) as u8, 70, 255]
                } else {
                    [90, 125, 160, 255]
                };
                photo.set_pixel(x, y, pixel);
            }
        }
        let found = suggestions(&Measure::of(&photo), neutral_color(&photo), &Edit::default(), &Options::default());
        assert!(!found.iter().any(|found| found.check == Check::ColorCast), "{found:?}");
    }

    #[test]
    fn only_a_strong_cool_lean_counts_as_a_cast() {
        // Slightly blue grays (a reflection of the sky, say) are left alone…
        let slight = scene(0.25, 0.8, [0.95, 1.0, 1.05]);
        assert!(!checks(&slight, &Develop::default()).contains(&Check::ColorCast));
        // …a strong blue cast (open shade, a flash gel) is found.
        let strong = scene(0.25, 0.8, [0.85, 0.95, 1.1]);
        let found = suggestions(&Measure::of(&strong), neutral_color(&strong), &Edit::default(), &Options::default());
        let cast = found.iter().find(|found| found.check == Check::ColorCast).expect("a strong cool cast is noticed");
        assert!(cast.title.contains("cool"), "{}", cast.title);
    }

    #[test]
    fn strong_and_dull_color_and_pushed_settings_are_noticed() {
        let vivid = checks(&scene(0.3, 1.0, [1.0, 0.05, 0.0]), &Develop::default());
        assert!(vivid.contains(&Check::Oversaturated), "{vivid:?}");
        let dull = checks(&scene(0.05, 0.95, [1.0, 1.0, 1.0]), &Develop::default());
        assert!(dull.contains(&Check::Dull), "{dull:?}");
        // A black-and-white photo isn't dull, it's black and white.
        assert!(!checks(&scene(0.05, 0.95, [1.0, 1.0, 1.0]), &Develop { saturation: -100.0, ..Develop::default() }).contains(&Check::Dull));
        let pushed = Develop { clarity: 90.0, dehaze: 20.0, ..Develop::default() };
        let found = suggestions(&Measure::of(&scene(0.05, 0.95, [1.0, 0.9, 0.8])), None, &Edit { develop: pushed, ..Edit::default() }, &Options::default());
        let over = found.iter().find(|found| found.check == Check::Overprocessed).expect("pushed clarity is noticed");
        assert_eq!(over.fix.as_ref().unwrap().changes, vec![(DevelopSetting::Clarity, 45.0)]);
    }

    #[test]
    fn a_bright_sky_over_darker_land_gets_a_graduated_mask() {
        // A bright pale sky over the top third, mid-toned land below.
        let mut photo = Bitmap::new(200, 150);
        for y in 0..150 {
            for x in 0..200 {
                let v = if y < 50 { 245 - (y as u8) } else { 70 + ((x + y) % 60) as u8 };
                photo.set_pixel(x, y, [v, v, v.saturating_add(5), 255]);
            }
        }
        let measure = Measure::of(&photo);
        let found = suggestions(&measure, None, &Edit::default(), &Options::default());
        let sky = found.iter().find(|found| found.check == Check::BrightSky).expect("the bright sky is noticed");
        let fix = sky.fix.as_ref().unwrap();
        let mask = fix.mask.as_ref().expect("the fix adds a mask");
        assert!(mask.settings.exposure < 0.0 && matches!(mask.shape, MaskShape::Linear { .. }));
        // With the mask in place, the top is darker and the rule is satisfied.
        let edit = fix.apply_to_edit(&Edit::default());
        let after = Measure::of(&crate::masks::apply_masks(&photo, &edit.masks, 1.0));
        assert!(after.top < measure.top - 0.05, "{} → {}", measure.top, after.top);
        assert!(!suggestions(&after, None, &edit, &Options::default()).iter().any(|found| found.check == Check::BrightSky));
    }

    #[test]
    fn lifting_shadows_far_suggests_noise_reduction_until_some_is_used() {
        let photo = scene(0.05, 0.95, [1.0, 0.95, 0.9]);
        let lifted = Edit { develop: Develop { shadows: 80.0, ..Develop::default() }, ..Edit::default() };
        let found = suggestions(&Measure::of(&photo), None, &lifted, &Options::default());
        let noisy = found.iter().find(|found| found.check == Check::NoisyShadows).expect("noted");
        let fixed = noisy.fix.as_ref().unwrap().apply_to_edit(&lifted);
        assert_eq!(fixed.develop.noise_luminance, 25.0);
        assert!(!suggestions(&Measure::of(&photo), None, &fixed, &Options::default()).iter().any(|found| found.check == Check::NoisyShadows));
    }

    #[test]
    fn rules_switched_off_stay_quiet() {
        let photo = scene(0.45, 1.0, [1.0, 1.0, 1.0]);
        let mut options = Options::default();
        options.set(Check::Overexposed, false);
        let found = suggestions(&Measure::of(&photo), None, &Edit::default(), &options);
        assert!(!found.iter().any(|found| found.check == Check::Overexposed));
        options.set(Check::Overexposed, true);
        assert!(options.runs(Check::Overexposed));
    }
}

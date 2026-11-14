//! Presets: saved looks, as in Lightroom. A preset holds Develop settings and which groups of
//! them it sets; applying it changes only those groups, so a black-and-white preset leaves the
//! white balance alone and a grain preset leaves the color alone.

use serde::{Deserialize, Serialize};

use crate::develop::{Develop, GradeWheel};

/// The groups of Develop settings a preset can set, as the Develop panel's sections divide them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PresetParts {
    pub white_balance: bool,
    /// Exposure, contrast, highlights, shadows, whites and blacks.
    pub tone: bool,
    /// Texture, clarity, dehaze, vibrance and saturation.
    pub presence: bool,
    pub tone_curve: bool,
    pub color_mixer: bool,
    pub color_grading: bool,
    /// Sharpening and noise reduction.
    pub detail: bool,
    /// Vignette and grain.
    pub effects: bool,
}

impl Default for PresetParts {
    /// Everything but the white balance, which belongs to the light each photo was taken in.
    fn default() -> Self {
        Self { white_balance: false, tone: true, presence: true, tone_curve: true, color_mixer: true, color_grading: true, detail: true, effects: true }
    }
}

impl PresetParts {
    pub const NONE: PresetParts = PresetParts { white_balance: false, tone: false, presence: false, tone_curve: false, color_mixer: false, color_grading: false, detail: false, effects: false };

    /// Each group's name and a way to reach its flag, in the Develop panel's order.
    pub fn named() -> [(&'static str, PartFlag); 8] {
        [
            ("White Balance", |parts| &mut parts.white_balance),
            ("Tone", |parts| &mut parts.tone),
            ("Presence", |parts| &mut parts.presence),
            ("Tone Curve", |parts| &mut parts.tone_curve),
            ("Color Mixer", |parts| &mut parts.color_mixer),
            ("Color Grading", |parts| &mut parts.color_grading),
            ("Detail", |parts| &mut parts.detail),
            ("Effects", |parts| &mut parts.effects),
        ]
    }
}

/// Reaches one group's flag in a `PresetParts`.
pub type PartFlag = fn(&mut PresetParts) -> &mut bool;

/// A saved look.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preset {
    pub name: String,
    /// The heading it's listed under.
    pub group: String,
    pub settings: Develop,
    pub parts: PresetParts,
}

impl Default for Preset {
    fn default() -> Self {
        Self { name: "Preset".into(), group: "User Presets".into(), settings: Develop::default(), parts: PresetParts::default() }
    }
}

impl Preset {
    /// A preset of `settings`, setting the groups in `parts`.
    pub fn new(name: impl Into<String>, group: impl Into<String>, settings: Develop, parts: PresetParts) -> Self {
        Self { name: name.into(), group: group.into(), settings, parts }
    }

    /// `develop` with this preset's groups set from it.
    pub fn apply_to(&self, develop: &Develop) -> Develop {
        let from = &self.settings;
        let mut out = develop.clone();
        let parts = self.parts;
        if parts.white_balance {
            out.temperature = from.temperature;
            out.tint = from.tint;
        }
        if parts.tone {
            out.exposure = from.exposure;
            out.contrast = from.contrast;
            out.highlights = from.highlights;
            out.shadows = from.shadows;
            out.whites = from.whites;
            out.blacks = from.blacks;
        }
        if parts.presence {
            out.texture = from.texture;
            out.clarity = from.clarity;
            out.dehaze = from.dehaze;
            out.vibrance = from.vibrance;
            out.saturation = from.saturation;
        }
        if parts.tone_curve {
            out.curve = from.curve.clone();
            out.curve_red = from.curve_red.clone();
            out.curve_green = from.curve_green.clone();
            out.curve_blue = from.curve_blue.clone();
        }
        if parts.color_mixer {
            out.mixer_hue = from.mixer_hue;
            out.mixer_saturation = from.mixer_saturation;
            out.mixer_luminance = from.mixer_luminance;
        }
        if parts.color_grading {
            out.grade_shadows = from.grade_shadows;
            out.grade_midtones = from.grade_midtones;
            out.grade_highlights = from.grade_highlights;
            out.grade_blending = from.grade_blending;
            out.grade_balance = from.grade_balance;
        }
        if parts.detail {
            out.sharpen_amount = from.sharpen_amount;
            out.sharpen_radius = from.sharpen_radius;
            out.noise_luminance = from.noise_luminance;
            out.noise_color = from.noise_color;
        }
        if parts.effects {
            out.vignette = from.vignette;
            out.vignette_midpoint = from.vignette_midpoint;
            out.vignette_feather = from.vignette_feather;
            out.grain = from.grain;
            out.grain_size = from.grain_size;
        }
        out
    }
}

/// Every setting desaturated, the start of each black-and-white look.
fn monochrome() -> Develop {
    Develop { saturation: -100.0, ..Develop::default() }
}

/// The looks that come with Develop.
pub fn builtin() -> Vec<Preset> {
    let parts = PresetParts::default();
    let color = "Color";
    let mono = "Black & White";
    let creative = "Creative";
    let subject = "Portrait & Landscape";
    let wheel = |hue: f32, saturation: f32| GradeWheel { hue, saturation, luminance: 0.0 };
    vec![
        Preset::new("Vivid", color, Develop { contrast: 15.0, vibrance: 35.0, saturation: 8.0, clarity: 10.0, ..Develop::default() }, parts),
        Preset::new("Soft", color, Develop { contrast: -20.0, highlights: -25.0, shadows: 20.0, clarity: -15.0, vibrance: 10.0, ..Develop::default() }, parts),
        Preset::new("Bright & Airy", color, Develop { exposure: 0.35, contrast: -15.0, highlights: -30.0, shadows: 35.0, whites: 15.0, vibrance: 10.0, saturation: -10.0, ..Develop::default() }, parts),
        Preset::new("Moody", color, Develop { exposure: -0.3, contrast: 20.0, highlights: -40.0, shadows: -10.0, blacks: -15.0, saturation: -25.0, vignette: -25.0, ..Develop::default() }, parts),
        Preset::new(
            "Matte Fade",
            color,
            Develop { contrast: -10.0, curve: vec![[0.0, 0.08], [0.25, 0.25], [0.75, 0.75], [1.0, 0.95]], saturation: -15.0, ..Develop::default() },
            parts,
        ),
        Preset::new("High Contrast", mono, Develop { contrast: 45.0, clarity: 20.0, whites: 20.0, blacks: -25.0, ..monochrome() }, parts),
        Preset::new("Soft Gray", mono, Develop { contrast: -20.0, highlights: -20.0, shadows: 25.0, ..monochrome() }, parts),
        Preset::new("Silver Grain", mono, Develop { contrast: 20.0, grain: 35.0, grain_size: 30.0, vignette: -15.0, ..monochrome() }, parts),
        Preset::new(
            "Teal & Orange",
            creative,
            Develop {
                contrast: 15.0,
                grade_shadows: wheel(195.0, 35.0),
                grade_highlights: wheel(35.0, 30.0),
                vibrance: 15.0,
                ..Develop::default()
            },
            parts,
        ),
        Preset::new(
            "Warm Film",
            creative,
            Develop {
                temperature: 12.0,
                contrast: -10.0,
                curve: vec![[0.0, 0.05], [0.5, 0.52], [1.0, 0.96]],
                grade_shadows: wheel(25.0, 15.0),
                saturation: -10.0,
                grain: 20.0,
                ..Develop::default()
            },
            PresetParts { white_balance: true, ..parts },
        ),
        Preset::new(
            "Cool Fade",
            creative,
            Develop {
                temperature: -15.0,
                curve: vec![[0.0, 0.1], [0.5, 0.5], [1.0, 0.94]],
                grade_highlights: wheel(210.0, 15.0),
                saturation: -20.0,
                ..Develop::default()
            },
            PresetParts { white_balance: true, ..parts },
        ),
        Preset::new("Vintage", creative, Develop { contrast: -15.0, grade_midtones: wheel(45.0, 25.0), saturation: -25.0, vignette: -30.0, grain: 25.0, curve: vec![[0.0, 0.1], [1.0, 0.9]], ..Develop::default() }, parts),
        Preset::new("Portrait Glow", subject, Develop { exposure: 0.15, clarity: -20.0, texture: -15.0, highlights: -15.0, vibrance: 5.0, mixer_luminance: [0.0, 15.0, 5.0, 0.0, 0.0, 0.0, 0.0, 0.0], ..Develop::default() }, parts),
        Preset::new("Landscape Punch", subject, Develop { contrast: 20.0, clarity: 25.0, dehaze: 15.0, vibrance: 30.0, highlights: -35.0, shadows: 25.0, mixer_saturation: [0.0, 0.0, 10.0, 20.0, 15.0, 20.0, 0.0, 0.0], ..Develop::default() }, parts),
        Preset::new("Clear Sky", subject, Develop { highlights: -40.0, dehaze: 20.0, mixer_luminance: [0.0, 0.0, 0.0, 0.0, -10.0, -25.0, 0.0, 0.0], mixer_saturation: [0.0, 0.0, 0.0, 0.0, 10.0, 20.0, 0.0, 0.0], ..Develop::default() }, parts),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_set_only_their_groups() {
        let mut develop = Develop { temperature: 30.0, exposure: 1.0, ..Develop::default() };
        develop.grain = 10.0;
        let preset = Preset::new("Gray", "Test", monochrome(), PresetParts { presence: true, ..PresetParts::NONE });
        let applied = preset.apply_to(&develop);
        assert_eq!(applied.saturation, -100.0);
        assert_eq!((applied.temperature, applied.exposure, applied.grain), (30.0, 1.0, 10.0), "other groups untouched");
    }

    #[test]
    fn builtin_presets_have_unique_names_and_change_something() {
        let presets = builtin();
        let mut names: Vec<&str> = presets.iter().map(|preset| preset.name.as_str()).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), presets.len());
        for preset in &presets {
            assert_ne!(preset.apply_to(&Develop::default()), Develop::default(), "{} changes nothing", preset.name);
        }
    }

    #[test]
    fn presets_round_trip_through_json() {
        let preset = builtin().remove(0);
        let text = serde_json::to_string(&preset).unwrap();
        assert_eq!(serde_json::from_str::<Preset>(&text).unwrap(), preset);
    }
}

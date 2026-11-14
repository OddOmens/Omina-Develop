//! Looks asked for by a word: "moody", "bright and airy", "a bit cinematic", "very vintage".
//! Each is a short list of slider moves written by hand, made from where the photo is now
//! (so corrections already made stay), scaled by "a bit" or "a lot", and named in the reply so
//! the look can be tuned slider by slider. "Less moody" makes the same moves the other way.
//!
//! These are nudges; the presets of the same names set whole groups of settings instead.

use super::{listed_changes, strength};
use crate::assistant::Fix;
use crate::develop::{Develop, DevelopSetting};

/// One move a look makes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Move {
    /// Add this to the slider (scaled by how strongly the look was asked for).
    By(f32),
    /// Set the slider to this, however strongly it was asked for: a tint's hue, say.
    To(f32),
}

/// A look that can be asked for.
pub struct Look {
    pub name: &'static str,
    /// The ways it is asked for: each is a set of words that must all be said.
    phrases: &'static [&'static [&'static str]],
    /// Whether one of its phrases is an everyday word ("soft", "cool"), so the request must
    /// also say it is a look that is wanted.
    needs_cue: bool,
    pub moves: &'static [(DevelopSetting, Move)],
}

use DevelopSetting::{Blacks, Clarity, Contrast, Exposure, Grade, Grain, Highlights, Saturation, Shadows, Temperature, Texture, Tint, Vibrance, Vignette, Whites};
use Move::{By, To};

/// Color Grading's wheels and values, as `DevelopSetting::Grade` numbers them.
const SHADOWS: usize = 0;
const MIDTONES: usize = 1;
const HIGHLIGHTS: usize = 2;
const HUE: usize = 0;
const STRENGTH: usize = 1;

/// Every look, in the order `/looks` lists them. Where two could match, the earlier wins, so
/// "dramatic black and white" comes before anything "black and white" alone might mean.
pub const LOOKS: &[Look] = &[
    Look {
        name: "Dramatic Black & White",
        phrases: &[&["dramatic", "black", "white"], &["dramatic", "b&w"], &["dramatic", "bw"], &["dramatic", "monochrome"], &["noir"], &["contrasty", "black", "white"], &["punchy", "black", "white"]],
        needs_cue: false,
        moves: &[(Saturation, To(-100.0)), (Contrast, By(35.0)), (Clarity, By(18.0)), (Whites, By(12.0)), (Blacks, By(-20.0)), (Vignette, By(-15.0))],
    },
    Look {
        name: "Moody",
        phrases: &[&["moody"], &["moodier"], &["brooding"]],
        needs_cue: false,
        moves: &[(Exposure, By(-0.25)), (Contrast, By(15.0)), (Highlights, By(-30.0)), (Shadows, By(-10.0)), (Blacks, By(-12.0)), (Saturation, By(-18.0)), (Vignette, By(-20.0))],
    },
    Look {
        name: "Bright & Airy",
        phrases: &[&["airy"], &["airier"], &["light", "fresh"]],
        needs_cue: false,
        moves: &[(Exposure, By(0.3)), (Contrast, By(-12.0)), (Highlights, By(-25.0)), (Shadows, By(30.0)), (Whites, By(12.0)), (Vibrance, By(8.0)), (Saturation, By(-6.0))],
    },
    Look {
        name: "Punchy",
        phrases: &[&["punchy"], &["bold", "look"], &["bold", "color"]],
        needs_cue: false,
        moves: &[(Contrast, By(20.0)), (Clarity, By(12.0)), (Vibrance, By(18.0)), (Whites, By(8.0)), (Blacks, By(-8.0))],
    },
    Look {
        name: "Matte",
        phrases: &[&["matte"], &["faded"], &["fade", "look"], &["film", "fade"]],
        needs_cue: false,
        moves: &[(Blacks, By(40.0)), (Contrast, By(-12.0)), (Whites, By(-10.0)), (Saturation, By(-12.0))],
    },
    Look {
        name: "Cinematic",
        phrases: &[&["cinematic"], &["cinema"], &["movie", "look"], &["teal", "orange"]],
        needs_cue: false,
        moves: &[
            (Contrast, By(12.0)),
            (Highlights, By(-20.0)),
            (Shadows, By(8.0)),
            (Saturation, By(-8.0)),
            (Vibrance, By(10.0)),
            (Grade(SHADOWS, HUE), To(200.0)),
            (Grade(SHADOWS, STRENGTH), By(22.0)),
            (Grade(HIGHLIGHTS, HUE), To(38.0)),
            (Grade(HIGHLIGHTS, STRENGTH), By(18.0)),
            (Vignette, By(-12.0)),
        ],
    },
    Look {
        name: "Vintage",
        phrases: &[&["vintage"], &["retro"], &["old", "film"], &["nostalgic"]],
        needs_cue: false,
        moves: &[
            (Temperature, By(8.0)),
            (Contrast, By(-12.0)),
            (Blacks, By(20.0)),
            (Saturation, By(-20.0)),
            (Grade(MIDTONES, HUE), To(45.0)),
            (Grade(MIDTONES, STRENGTH), By(18.0)),
            (Vignette, By(-22.0)),
            (Grain, By(22.0)),
        ],
    },
    Look {
        name: "Warm Sunset",
        phrases: &[&["warm", "sunset"], &["sunset", "look"], &["sunset", "feel"], &["sunset", "glow"], &["golden", "hour"], &["golden", "glow"], &["warm", "glow"]],
        needs_cue: false,
        moves: &[(Temperature, By(22.0)), (Tint, By(6.0)), (Highlights, By(-15.0)), (Shadows, By(10.0)), (Vibrance, By(15.0)), (Grade(HIGHLIGHTS, HUE), To(40.0)), (Grade(HIGHLIGHTS, STRENGTH), By(15.0))],
    },
    Look {
        name: "High-Key",
        phrases: &[&["high", "key"], &["highkey"]],
        needs_cue: false,
        moves: &[(Exposure, By(0.5)), (Contrast, By(-15.0)), (Highlights, By(-15.0)), (Shadows, By(35.0)), (Whites, By(20.0)), (Blacks, By(15.0))],
    },
    Look {
        name: "Low-Key",
        phrases: &[&["low", "key"], &["lowkey"]],
        needs_cue: false,
        moves: &[(Exposure, By(-0.5)), (Contrast, By(20.0)), (Highlights, By(-10.0)), (Shadows, By(-25.0)), (Blacks, By(-20.0)), (Vignette, By(-15.0))],
    },
    Look {
        name: "Soft",
        phrases: &[&["dreamy"], &["soft"]],
        needs_cue: true,
        moves: &[(Contrast, By(-15.0)), (Highlights, By(-15.0)), (Shadows, By(15.0)), (Texture, By(-10.0)), (Clarity, By(-15.0))],
    },
    Look {
        name: "Cool",
        phrases: &[&["icy"], &["wintry"], &["cool"], &["cold"]],
        needs_cue: true,
        moves: &[(Temperature, By(-18.0)), (Tint, By(-3.0)), (Contrast, By(6.0)), (Saturation, By(-8.0)), (Grade(SHADOWS, HUE), To(215.0)), (Grade(SHADOWS, STRENGTH), By(12.0))],
    },
];

/// Words that say a look is wanted when they follow an everyday word: "a soft look", "cool
/// tones".
const AFTER: &[&str] = &["look", "feel", "style", "vibe", "mood", "tone", "finish", "glow"];
/// Words that say so from anywhere in the request: "make it soft", "go cool".
const ASKING: &[&str] = &["make", "go", "give", "want"];
/// Words that never need a cue, though their look does: nobody says "dreamy" by accident.
const PLAIN_ENOUGH: &[&str] = &["dreamy", "icy", "wintry"];
/// Words that turn a look round: "less moody", "not so vintage", "too cinematic".
const LESS: &[&str] = &["less", "too", "not", "reduce"];
/// Words that ask for a look to be taken away altogether, which is Undo's job, not a look's.
const AWAY: &[&str] = &["undo", "remove", "without", "no", "drop"];

/// The look a message names, if it names one. A look named by an everyday word ("soft",
/// "cool") also needs the request to say a look is wanted: "it looks soft" is a complaint.
fn find(said: &[String]) -> Option<&'static Look> {
    let has = |word: &str| said.iter().any(|each| each == word);
    LOOKS.iter().find(|look| {
        look.phrases.iter().any(|phrase| {
            let cued = || {
                let followed = said.windows(2).any(|pair| phrase.contains(&pair[0].as_str()) && AFTER.contains(&pair[1].as_str()));
                followed || ASKING.iter().any(|word| has(word)) || phrase.iter().any(|word| PLAIN_ENOUGH.contains(word))
            };
            phrase.iter().all(|word| has(word)) && (!look.needs_cue || cued())
        })
    })
}

/// `look` made on `develop`, `scale` times as strongly (negative for less of it).
pub fn apply(look: &Look, develop: &Develop, scale: f32) -> Fix {
    let changes = look
        .moves
        .iter()
        .filter_map(|(setting, change)| {
            let info = setting.slider();
            let target = match change {
                Move::By(by) => setting.get(develop) + by * scale,
                // Taking a look back leaves a tint's hue alone: its strength going is enough.
                Move::To(_) if scale < 0.0 => return None,
                Move::To(value) => *value,
            };
            let places = 10f32.powi(info.decimals as i32);
            Some((*setting, ((target * places).round() / places).clamp(info.min, info.max)))
        })
        .collect();
    Fix::settings(format!("{} Look", look.name), changes)
}

/// A look asked for in `said`, made on `develop`. With `lessened` it reads the requests for
/// less of a look ("less moody", "too vintage") and only those; without, only plain requests
/// for one. They are asked for apart because "too soft" and "too cool" mean something else
/// first (sharpen, warm up), which the caller reads in between.
pub(super) fn request(said: &[String], develop: &Develop, lessened: bool) -> Option<(String, Fix)> {
    let look = find(said)?;
    if said.iter().any(|word| AWAY.contains(&word.as_str())) {
        return None;
    }
    let less = said.iter().any(|word| LESS.contains(&word.as_str()));
    if less != lessened {
        return None;
    }
    let k = strength(said);
    let fix = apply(look, develop, if less { -0.5 * k } else { k });
    let text = if less {
        format!("Less of the {} look: {}. Say “less” again to go further, or name a slider.", look.name, listed_changes(&fix))
    } else {
        format!("{} look, nudged from where it was: {}. Say “more” or “less” to tune it, or name a slider.", look.name, listed_changes(&fix))
    };
    Some((text, fix))
}

/// The looks by name, for listing.
pub fn names() -> Vec<&'static str> {
    LOOKS.iter().map(|look| look.name).collect()
}

/// Every word this reader listens for, for putting a mistyped message right.
pub(super) fn vocabulary() -> Vec<&'static str> {
    LOOKS.iter().flat_map(|look| look.phrases.iter().flat_map(|phrase| phrase.iter().copied())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_look_changes_something_and_stays_in_range() {
        for look in LOOKS {
            let fix = apply(look, &Develop::default(), 2.0);
            assert!(!fix.changes.is_empty(), "{}", look.name);
            assert_ne!(fix.apply_to(&Develop::default()), Develop::default(), "{} changes nothing", look.name);
            for (setting, value) in &fix.changes {
                let info = setting.slider();
                assert!((info.min..=info.max).contains(value), "{}: {setting:?} {value}", look.name);
            }
            // Asked for from a photo pushed to its limits, it still stays within every slider.
            let pushed = Develop { exposure: 5.0, contrast: 100.0, blacks: -100.0, ..Develop::default() };
            for (setting, value) in &apply(look, &pushed, 2.0).changes {
                let info = setting.slider();
                assert!((info.min..=info.max).contains(value), "{}: {setting:?} {value}", look.name);
            }
        }
    }

    #[test]
    fn less_of_a_look_moves_the_sliders_back_and_leaves_hues_alone() {
        let cinematic = LOOKS.iter().find(|look| look.name == "Cinematic").unwrap();
        let made = apply(cinematic, &Develop::default(), 1.0).apply_to(&Develop::default());
        assert_eq!((made.grade_shadows.hue, made.grade_shadows.saturation), (200.0, 22.0));
        let eased = apply(cinematic, &made, -0.5);
        assert!(!eased.changes.iter().any(|(setting, _)| matches!(setting, DevelopSetting::Grade(_, 0))), "{eased:?}");
        let after = eased.apply_to(&made);
        assert_eq!(after.grade_shadows.saturation, 11.0);
        assert_eq!(after.contrast, 6.0);
    }
}

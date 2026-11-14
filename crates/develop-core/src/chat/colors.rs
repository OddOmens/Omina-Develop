//! One color at a time, through the Color Mixer: "make the sky bluer", "deeper blues",
//! "greener grass", "desaturate the oranges", "darken the blues", "softer skin tones", "blue
//! saturation -20". A color is named by its own word ("the blues"), by a comparative
//! ("bluer"), or by something that is usually that color (the sky, grass, skin).
//!
//! A color's plain name is not enough on its own: "too blue" is about the white balance, so
//! it is left for the reader of plain edits.

use super::edits::{Amount, amount, moved, named_setting, number};
use super::{described, raw_words, strength, valued, words};
use crate::assistant::Fix;
use crate::develop::{Develop, DevelopSetting, MIXER_BANDS};

/// The words for each of the mixer's colors, by its place in `MIXER_BANDS`: the color itself,
/// then its plural.
const COLORS: &[(&str, &str, usize)] = &[
    ("red", "reds", 0),
    ("orange", "oranges", 1),
    ("yellow", "yellows", 2),
    ("green", "greens", 3),
    ("aqua", "aquas", 4),
    ("cyan", "cyans", 4),
    ("teal", "teals", 4),
    ("turquoise", "turquoises", 4),
    ("blue", "blues", 5),
    ("purple", "purples", 6),
    ("violet", "violets", 6),
    ("magenta", "magentas", 7),
    ("pink", "pinks", 7),
];

/// Words that mean "more of this color".
const COMPARATIVES: &[(&str, usize)] = &[("redder", 0), ("yellower", 2), ("greener", 3), ("bluer", 5), ("pinker", 7)];

/// Things that are usually one color (or two), the colors they are read as, and what the
/// reply calls them.
const THINGS: &[(&[&str], &[usize], &str)] = &[
    (&["sky", "skies"], &[5], "sky"),
    (&["grass", "foliage", "leaves", "trees", "lawn", "greenery"], &[3, 2], "grass"),
    (&["skin", "complexion", "face", "faces"], &[1], "skin tones"),
    (&["water", "sea", "ocean", "lake"], &[4, 5], "water"),
];

/// What a request asks of a color.
#[derive(Clone, Copy, PartialEq)]
enum Wish {
    MoreColor,
    LessColor,
    Darker,
    Lighter,
    /// Darker and richer at once.
    Deeper,
    /// Calmer and a little lighter, as flatters skin.
    Softer,
    Reset,
}

/// Whether `message` names one of the mixer's colors, something read as one, or the mixer
/// itself.
pub(super) fn mentions(message: &str) -> bool {
    target(&raw_words(message), false).is_some()
}

/// The colors a message is about, and what to call them. A color's bare name counts only
/// when `plainly` (the request says outright what to do with it), since "too blue" and "teal
/// and orange" are not about the mixer.
fn target(raw: &[String], plainly: bool) -> Option<(Vec<usize>, String)> {
    let said = |word: &str| raw.iter().any(|each| each == word);
    if said("mixer") || said("hsl") {
        return Some(((0..MIXER_BANDS.len()).collect(), "the Color Mixer".to_owned()));
    }
    if let Some((_, bands, name)) = THINGS.iter().find(|(names, _, _)| names.iter().any(|name| said(name))) {
        return Some((bands.to_vec(), format!("the {name}")));
    }
    if let Some((_, plural, band)) = COLORS.iter().find(|(_, plural, _)| said(plural)) {
        return Some((vec![*band], format!("the {plural}")));
    }
    if let Some((_, band)) = COMPARATIVES.iter().find(|(word, _)| said(word)) {
        return Some((vec![*band], format!("the {}s", MIXER_BANDS[*band].0.to_lowercase())));
    }
    let (_, plural, band) = COLORS.iter().find(|(color, _, _)| said(color)).filter(|_| plainly)?;
    Some((vec![*band], format!("the {plural}")))
}

/// Which of a color's three sliders a message names, if it names one.
fn channel(raw: &[String]) -> Option<fn(usize) -> DevelopSetting> {
    let said = |list: &[&str]| raw.iter().any(|word| list.contains(&word.as_str()));
    if said(&["hue"]) {
        Some(DevelopSetting::MixerHue)
    } else if said(&["saturation"]) {
        Some(DevelopSetting::MixerSaturation)
    } else if said(&["luminance", "lightness"]) {
        Some(DevelopSetting::MixerLuminance)
    } else {
        None
    }
}

/// What a message asks of the colors it names.
fn wish(raw: &[String]) -> Option<Wish> {
    let said = |list: &[&str]| raw.iter().any(|word| list.contains(&word.as_str()));
    let pair = |first: &str, second: &[&str]| raw.windows(2).any(|pair| pair[0] == first && second.contains(&pair[1].as_str()));
    let too = |list: &[&str]| pair("too", list);
    Some(if said(&["reset", "remove"]) && !said(&["cast"]) {
        Wish::Reset
    } else if said(&["soften", "softer", "smooth", "smoother", "flatter", "flattering"]) {
        Wish::Softer
    } else if said(&["desaturate", "desaturated", "mute", "muted", "calm", "calmer", "duller", "quieter", "less", "fewer"]) || pair("tone", &["down"]) || too(&["strong", "saturated", "vivid", "much", "intense", "loud"]) {
        Wish::LessColor
    } else if said(&["deeper", "deepen", "richer"]) {
        Wish::Deeper
    } else if said(&["darken", "darker"]) || too(&["bright", "light", "pale"]) {
        Wish::Darker
    } else if said(&["lighten", "lighter", "brighten", "brighter"]) || too(&["dark"]) {
        Wish::Lighter
    } else if said(&["saturate", "saturated", "vivid", "stronger", "boost", "pop", "intensify", "punchier", "more", "bolder"]) || COMPARATIVES.iter().any(|(word, _)| said(&[word])) {
        Wish::MoreColor
    } else {
        return None;
    })
}

/// One of a color's own sliders set or moved by name: "blue saturation -20", "orange
/// luminance up", "set the green hue to 15".
pub(super) fn slider(message: &str, develop: &Develop) -> Option<(String, Fix)> {
    let raw = raw_words(message);
    let said = words(message);
    let slider = channel(&raw)?;
    let (bands, _) = target(&raw, true)?;
    let how = amount(message, &said)?;
    let mut now = develop.clone();
    for band in &bands {
        now = moved(slider(*band), how, &now).1.apply_to(&now);
    }
    let (listed, fix) = reply(bands.iter().map(|band| (slider(*band), slider(*band).get(&now))).collect());
    Some((format!("{listed}."), fix))
}

/// A change to one color asked for in `message`, read against the settings in `develop`.
pub(super) fn request(message: &str, develop: &Develop) -> Option<(String, Fix)> {
    if let Some(found) = slider(message, develop) {
        return Some(found);
    }
    let raw = raw_words(message);
    let said = words(message);
    // "More contrast in the sky" names a slider of the whole photo, not a color's.
    if named_setting(&said).is_some() {
        return None;
    }
    let wished = wish(&raw);
    let outright = raw.iter().any(|word| matches!(word.as_str(), "desaturate" | "saturate" | "mute" | "boost"));
    let plainly = outright || wished.is_some_and(|wish| !matches!(wish, Wish::MoreColor | Wish::LessColor));
    let (bands, name) = target(&raw, plainly)?;
    let typed = number(message);
    let wished = wished?;
    // A number sizes the change ("desaturate the oranges by 30"), or with "to" is the value
    // itself; without one the change is a fifth of the slider's travel, more or less.
    let k = strength(&said);
    let exact = matches!(amount(message, &said), Some(Amount::To(_))).then(|| typed.map(|number| number.value)).flatten();
    let shifted = |setting: DevelopSetting, usual: f32| {
        let info = setting.slider();
        let by = typed.map_or(usual * k, |number| number.value.abs() * usual.signum());
        (setting, exact.unwrap_or(setting.get(develop) + by).round().clamp(info.min, info.max))
    };
    let mut changes: Vec<(DevelopSetting, f32)> = Vec::new();
    for &band in &bands {
        match wished {
            Wish::MoreColor => changes.push(shifted(DevelopSetting::MixerSaturation(band), 20.0)),
            Wish::LessColor => changes.push(shifted(DevelopSetting::MixerSaturation(band), -20.0)),
            Wish::Darker => changes.push(shifted(DevelopSetting::MixerLuminance(band), -20.0)),
            Wish::Lighter => changes.push(shifted(DevelopSetting::MixerLuminance(band), 20.0)),
            Wish::Deeper => changes.extend([shifted(DevelopSetting::MixerLuminance(band), -15.0), shifted(DevelopSetting::MixerSaturation(band), 10.0)]),
            Wish::Softer => changes.extend([shifted(DevelopSetting::MixerSaturation(band), -10.0), shifted(DevelopSetting::MixerLuminance(band), 10.0)]),
            Wish::Reset => changes.extend([DevelopSetting::MixerHue(band), DevelopSetting::MixerSaturation(band), DevelopSetting::MixerLuminance(band)].map(|setting| (setting, 0.0))),
        }
    }
    let lead = match wished {
        Wish::MoreColor => "More color in",
        Wish::LessColor => "Quieter color in",
        Wish::Darker => "Darkening",
        Wish::Lighter => "Lightening",
        Wish::Deeper => "Deepening",
        Wish::Softer => "Softening",
        Wish::Reset => "Reset",
    };
    let (listed, fix) = reply(changes);
    // Resetting every color would list two dozen zeros; the name says it.
    let text = if wished == Wish::Reset { format!("Reset {name}: its hue, saturation and luminance are back to 0.") } else { format!("{lead} {name}: {listed}.") };
    // Brightness in the mixer reaches only what is colored, which a pale sky often is not.
    let pale_sky = name == "the sky" && matches!(wished, Wish::Darker | Wish::Deeper | Wish::Lighter);
    let text = if pale_sky { format!("{text} This reaches only the parts that are blue; for a pale or white sky, a Linear Gradient mask does it better (ask me “how do I darken a sky?”).") } else { text };
    Some((text, fix))
}

/// What the changes set, in words, and the fix that makes them.
fn reply(changes: Vec<(DevelopSetting, f32)>) -> (String, Fix) {
    let listed: Vec<String> = changes.iter().map(|(setting, value)| valued(*setting, *value)).collect();
    let label = match changes.as_slice() {
        [(setting, _)] => described(*setting),
        _ => "Color Mixer".to_owned(),
    };
    (listed.join(", "), Fix::settings(label, changes))
}

/// Every word this reader listens for, for putting a mistyped message right.
pub(super) fn vocabulary() -> Vec<&'static str> {
    let colors = COLORS.iter().flat_map(|(color, plural, _)| [*color, *plural]);
    let things = THINGS.iter().flat_map(|(names, _, _)| names.iter().copied());
    let own = ["saturation", "luminance", "lightness", "desaturate", "saturate", "deeper", "deepen", "richer", "soften", "softer", "smoother", "calmer", "duller", "bolder", "intensify"];
    colors.chain(things).chain(COMPARATIVES.iter().map(|(word, _)| *word)).chain(own).collect()
}

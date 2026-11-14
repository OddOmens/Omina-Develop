//! Edits to the Develop settings asked for in words: by feel ("a bit brighter", "less contrast",
//! "add a vignette", "reduce the noise"), by slider ("shadows +30", "set exposure to 0.5",
//! "raise the highlights a lot", "reset clarity"), and several at once ("shadows +30,
//! highlights -20 and a bit warmer"), which become one change so one Undo takes them all back.
//!
//! Every reader here turns words into a `Fix`: the settings to set and what to say about them.
//! Nothing is applied here; the caller decides whether to make the fix.

use super::{colors, listed_changes, looks, raw_words, strength, words};
use crate::assistant::Fix;
use crate::develop::{Develop, DevelopSetting};

/// A Develop setting changed by `by` from where it is now, kept within its range.
pub(super) fn nudge(develop: &Develop, setting: DevelopSetting, by: f32) -> (DevelopSetting, f32) {
    let info = setting.slider();
    let scale = 10f32.powi(info.decimals as i32);
    let value = ((setting.get(develop) + by) * scale).round() / scale;
    (setting, value.clamp(info.min, info.max))
}

/// How many stops of exposure a message names: "by a stop", "half a stop", "2 stops", "+1 EV".
fn stops(message: &str) -> Option<f32> {
    let tokens = raw_words(message);
    let at = tokens.iter().position(|word| matches!(word.as_str(), "stop" | "stops" | "ev"))?;
    let before = |back: usize| at.checked_sub(back).and_then(|index| tokens.get(index)).map(String::as_str);
    // "Half a stop", "a third of a stop".
    let amount = match (before(1), before(2), before(3)) {
        (Some("a"), Some("half"), _) => 0.5,
        (Some("a"), Some("of"), Some("third")) => 1.0 / 3.0,
        (Some("a"), Some("of"), Some("quarter")) => 0.25,
        (Some("a" | "one" | "1"), ..) => 1.0,
        (Some("half"), ..) => 0.5,
        (Some("two"), ..) => 2.0,
        (Some("three"), ..) => 3.0,
        _ => number(message).map(|number| number.value.abs()).filter(|value| *value > 0.0)?,
    };
    Some(amount)
}

/// An edit asked for by how the photo should feel, with what to say about it.
pub(super) fn plain(message: &str, said: &[String], develop: &Develop) -> Option<(String, Fix)> {
    let raw = raw_words(message);
    let has = |list: &[&str]| said.iter().any(|word| list.contains(&word.as_str()));
    let exactly = |list: &[&str]| raw.iter().any(|word| list.contains(&word.as_str()));
    let phrase = |first: &str, second: &[&str]| said.windows(2).any(|pair| pair[0] == first && second.contains(&pair[1].as_str()));
    let k = strength(said);
    let fix = |label: &str, changes: Vec<(DevelopSetting, f32)>| Fix::settings(label, changes);
    // "too dark", and "too much contrast" alike.
    let too = |words: &[&str]| phrase("too", words) || said.windows(3).any(|three| three[0] == "too" && three[1] == "much" && words.contains(&three[2].as_str()));
    let more = |words: &[&str]| phrase("more", words);
    let less = |words: &[&str]| phrase("less", words) || phrase("fewer", words);
    let named = named_setting(said);
    let wants_less = has(&["less", "fewer", "reduce", "lower", "weaker", "lessen", "ease"]) || said.iter().any(|word| word == "too");

    // "Blacks" and "whites" are sliders; only the two words themselves mean the look.
    if (exactly(&["black"]) && exactly(&["white"])) || has(&["b&w", "bw", "monochrome", "grayscale", "greyscale", "mono"]) {
        return Some(("Black and white it is: Saturation all the way down.".to_owned(), fix("Black & White", vec![(DevelopSetting::Saturation, -100.0)])));
    }
    // The vignette is named by what it does to the corners, so it is read before "darker".
    let corners = has(&["corner", "edge"]) && has(&["darken", "darker", "lighten", "lighter", "brighten", "brighter"]);
    if has(&["vignette", "vignetting"]) && !has(&["midpoint", "feather"]) || corners {
        let current = develop.vignette;
        let lighter = has(&["lighten", "lighter", "brighten", "brighter", "white"]);
        let change = if wants_less && !corners {
            // Less of it, whichever way it goes, and never past none at all.
            let eased = current - current.signum() * (15.0 * k).min(current.abs());
            (DevelopSetting::Vignette, eased.round())
        } else if lighter {
            nudge(develop, DevelopSetting::Vignette, 25.0 * k)
        } else if corners || has(&["add", "more", "stronger", "darker", "darken", "give", "put", "apply", "heavier", "with", "want"]) {
            nudge(develop, DevelopSetting::Vignette, -25.0 * k)
        } else {
            return None;
        };
        let what = if change.1 < 0.0 { "darker corners" } else if change.1 > 0.0 { "lighter corners" } else { "none" };
        return Some((format!("Vignette {:+.0} ({what}).", change.1), fix("Vignette", vec![change])));
    }
    // Film grain. "Less grainy" on a photo with no grain added means its noise.
    // A bare "grain" or "noise" names a topic; a request says what to do about it.
    let wants_some = has(&["add", "more", "film", "some", "give", "put", "want", "make", "grainy", "with"]);
    let wants_rid = wants_less || has(&["remove", "clean", "fix", "smooth", "cut", "rid", "noisy", "denoise"]);
    let grainy = has(&["grain", "grainy"]) && !has(&["size"]) && (wants_some || wants_rid);
    let noisy = has(&["noise", "noisy", "denoise"]) && !has(&["reduction"]) && (wants_rid || has(&["add", "more"]));
    if grainy && !wants_less || noisy && has(&["add", "more"]) {
        let change = nudge(develop, DevelopSetting::Grain, 25.0 * k);
        return Some((format!("Film grain: Grain {:.0}.", change.1), fix("Grain", vec![change])));
    }
    if grainy && develop.grain > 0.5 {
        let change = nudge(develop, DevelopSetting::Grain, -25.0 * k);
        return Some((format!("Less grain: Grain {:.0}.", change.1), fix("Grain", vec![change])));
    }
    if grainy || noisy {
        let changes = vec![nudge(develop, DevelopSetting::NoiseLuminance, 25.0 * k), (DevelopSetting::NoiseColor, develop.noise_color.max(25.0))];
        return Some((format!("Less noise: Luminance Noise {:.0}, Color Noise {:.0}.", changes[0].1, changes[1].1), fix("Noise Reduction", changes)));
    }
    // Haze: taking it away is what Dehaze is for; adding it is Dehaze the other way.
    if has(&["dehaze", "haze", "hazy", "hazier", "fog", "foggy", "foggier", "mist", "misty"]) {
        let add_haze = has(&["hazier", "foggier"]) || (has(&["add", "more"]) && !has(&["dehaze"])) || (has(&["dehaze"]) && wants_less);
        let change = nudge(develop, DevelopSetting::Dehaze, if add_haze { -20.0 * k } else { 20.0 * k });
        return Some((format!("{}: Dehaze {:+.0}.", if add_haze { "Hazier" } else { "Less haze" }, change.1), fix("Dehaze", vec![change])));
    }
    // "Brighten the shadows" names a slider of its own, which `slider` reads.
    let whole_photo = named.is_none_or(|setting| setting == DevelopSetting::Exposure);
    if (has(&["brighter", "lighter", "brighten", "lighten"]) && whole_photo) || too(&["dark", "dim"]) && whole_photo || has(&["underexposed"]) {
        let change = nudge(develop, DevelopSetting::Exposure, stops(message).unwrap_or(0.3 * k));
        return Some((format!("Brighter: Exposure {:+.2}.", change.1), fix("Brighter", vec![change])));
    }
    if (has(&["darker", "darken", "dimmer"]) && whole_photo) || too(&["bright", "light"]) && whole_photo || has(&["overexposed", "blown"]) {
        let mut changes = vec![nudge(develop, DevelopSetting::Exposure, -stops(message).unwrap_or(0.3 * k))];
        if has(&["blown", "overexposed"]) || too(&["bright"]) {
            changes.push(nudge(develop, DevelopSetting::Highlights, -25.0 * k));
        }
        let text = changes.iter().map(|(setting, value)| format!("{} {value:+}", setting.slider().name)).collect::<Vec<_>>().join(", ");
        return Some((format!("Darker: {text}."), fix("Darker", changes)));
    }
    // Words that say how the photo should feel without naming a slider. Each has one stated
    // meaning, said back with what it came to, so it can be tuned from there.
    let said_as = |lead: &str, label: &str, changes: Vec<(DevelopSetting, f32)>| {
        let made = fix(label, changes);
        Some((format!("{lead}: {}.", listed_changes(&made)), made))
    };
    if has(&["clean", "tidy"]) && has(&["up", "it", "photo", "image", "picture"]) {
        let changes = vec![nudge(develop, DevelopSetting::NoiseLuminance, 25.0 * k), (DevelopSetting::NoiseColor, develop.noise_color.max(25.0)), nudge(develop, DevelopSetting::SharpenAmount, 20.0 * k)];
        return said_as("Cleaned up, which I take as less noise and a little sharpening", "Clean Up", changes);
    }
    if has(&["pop"]) && !wants_less {
        let changes = vec![nudge(develop, DevelopSetting::Contrast, 15.0 * k), nudge(develop, DevelopSetting::Clarity, 10.0 * k), nudge(develop, DevelopSetting::Vibrance, 15.0 * k)];
        return said_as("Pop, which I take as more contrast, clarity and color", "Pop", changes);
    }
    if has(&["dramatic", "drama"]) {
        let way = k * if wants_less { -0.5 } else { 1.0 };
        let changes = vec![nudge(develop, DevelopSetting::Contrast, 20.0 * way), nudge(develop, DevelopSetting::Clarity, 12.0 * way), nudge(develop, DevelopSetting::Highlights, -10.0 * way), nudge(develop, DevelopSetting::Blacks, -10.0 * way)];
        return said_as(if wants_less { "Less dramatic" } else { "More dramatic, which I take as more contrast and clarity with deeper blacks" }, "Dramatic", changes);
    }
    // "More subtle" eases the whole edit: every slider a part of the way back to where it starts.
    if named.is_none() && (has(&["subtler"]) || (has(&["subtle", "understated"]) && has(&["more", "make", "go", "keep", "bit", "lot"]))) {
        let back = if has(&["lot", "much", "way", "really", "very"]) { 0.5 } else if has(&["bit", "little", "slightly", "touch"]) { 0.125 } else { 0.25 };
        let changes: Vec<(DevelopSetting, f32)> = super::all_settings()
            .into_iter()
            .filter(|setting| (setting.get(develop) - setting.slider().default).abs() > 1e-3)
            .map(|setting| {
                let start = setting.slider().default;
                nudge(develop, setting, (start - setting.get(develop)) * back)
            })
            .collect();
        if changes.is_empty() {
            return Some(("No slider has been moved yet, so there is nothing to make more subtle.".to_owned(), fix("More Subtle", changes)));
        }
        return said_as(&format!("More subtle: every slider you've moved goes {} of the way back to where it starts", if back == 0.5 { "half" } else if back == 0.125 { "an eighth" } else { "a quarter" }), "More Subtle", changes);
    }
    if has(&["flatter"]) {
        let changes = vec![nudge(develop, DevelopSetting::Contrast, -20.0 * k), nudge(develop, DevelopSetting::Clarity, -8.0 * k)];
        return said_as("Flatter, which I take as less contrast and clarity", "Flatter", changes);
    }
    if less(&["contrast"]) || too(&["contrasty", "harsh", "contrast", "punchy"]) || has(&["contrasty", "harsh", "softer"]) {
        let changes = vec![nudge(develop, DevelopSetting::Contrast, -20.0 * k), nudge(develop, DevelopSetting::Highlights, -15.0 * k), nudge(develop, DevelopSetting::Shadows, 15.0 * k)];
        return said_as("Softer", "Less Contrast", changes);
    }
    if more(&["contrast", "punch"]) || has(&["punchier", "flat", "washed"]) || too(&["flat", "dull"]) && has(&["contrast", "flat"]) {
        let changes = vec![nudge(develop, DevelopSetting::Contrast, 20.0 * k), nudge(develop, DevelopSetting::Clarity, 8.0 * k)];
        return said_as("Punchier", "More Contrast", changes);
    }
    if has(&["warmer", "warm"]) || too(&["cool", "cold", "blue"]) {
        let change = nudge(develop, DevelopSetting::Temperature, 15.0 * k);
        return Some((format!("Warmer: Temperature {:+.0}.", change.1), fix("Warmer", vec![change])));
    }
    if has(&["cooler", "colder"]) || too(&["warm", "yellow", "orange"]) || (has(&["cool"]) && has(&["down"])) {
        let change = nudge(develop, DevelopSetting::Temperature, -15.0 * k);
        return Some((format!("Cooler: Temperature {:+.0}.", change.1), fix("Cooler", vec![change])));
    }
    if less(&["color", "colour", "saturation", "saturated", "colorful", "vivid"]) || too(&["saturated", "colorful", "vivid", "garish", "color", "colour", "saturation"]) || has(&["desaturate", "muted", "mute"]) {
        let change = nudge(develop, DevelopSetting::Saturation, -20.0 * k);
        return Some((format!("Quieter colors: Saturation {:+.0}.", change.1), fix("Less Color", vec![change])));
    }
    if more(&["color", "colour", "saturation", "vibrance", "saturated", "colorful", "vivid"]) || has(&["vivid", "colorful", "saturate", "vibrant"]) || too(&["dull"]) {
        let change = nudge(develop, DevelopSetting::Vibrance, 20.0 * k);
        return Some((format!("More color: Vibrance {:+.0}.", change.1), fix("More Color", vec![change])));
    }
    if too(&["sharp", "crisp", "crunchy"]) || less(&["sharp"]) || has(&["oversharpened"]) {
        let change = nudge(develop, DevelopSetting::SharpenAmount, -30.0 * k);
        return Some((format!("Less sharpening: Sharpening {:.0}.", change.1), fix("Less Sharpening", vec![change])));
    }
    if has(&["sharpen", "sharper", "crisper", "crisp"]) || too(&["soft", "blurry"]) {
        let change = nudge(develop, DevelopSetting::SharpenAmount, 30.0 * k);
        return Some((format!("Sharper: Sharpening {:.0}.", change.1), fix("Sharper", vec![change])));
    }
    None
}

/// The sliders that can be named in a message, by the words that name them (after `words`
/// has taken plurals off). Longer names come first so "luminance noise" isn't read as "noise".
const SETTING_NAMES: &[(&[&str], DevelopSetting)] = &[
    (&["luminance", "noise"], DevelopSetting::NoiseLuminance),
    (&["color", "noise"], DevelopSetting::NoiseColor),
    (&["colour", "noise"], DevelopSetting::NoiseColor),
    (&["noise", "reduction"], DevelopSetting::NoiseLuminance),
    (&["grain", "size"], DevelopSetting::GrainSize),
    (&["vignette", "midpoint"], DevelopSetting::VignetteMidpoint),
    (&["vignette", "feather"], DevelopSetting::VignetteFeather),
    (&["sharpening", "radiu"], DevelopSetting::SharpenRadius),
    (&["sharpen", "radiu"], DevelopSetting::SharpenRadius),
    (&["temperature"], DevelopSetting::Temperature),
    (&["temp"], DevelopSetting::Temperature),
    (&["tint"], DevelopSetting::Tint),
    (&["exposure"], DevelopSetting::Exposure),
    (&["contrast"], DevelopSetting::Contrast),
    (&["highlight"], DevelopSetting::Highlights),
    (&["shadow"], DevelopSetting::Shadows),
    (&["white"], DevelopSetting::Whites),
    (&["black"], DevelopSetting::Blacks),
    (&["texture"], DevelopSetting::Texture),
    (&["clarity"], DevelopSetting::Clarity),
    (&["dehaze"], DevelopSetting::Dehaze),
    (&["vibrance"], DevelopSetting::Vibrance),
    (&["saturation"], DevelopSetting::Saturation),
    (&["sharpening"], DevelopSetting::SharpenAmount),
    (&["sharpen"], DevelopSetting::SharpenAmount),
    (&["vignette"], DevelopSetting::Vignette),
    (&["grain"], DevelopSetting::Grain),
];

/// The slider a message names, if one.
pub(super) fn named_setting(said: &[String]) -> Option<DevelopSetting> {
    SETTING_NAMES.iter().find(|(names, _)| names.iter().all(|name| said.iter().any(|word| word == name))).map(|(_, setting)| *setting)
}

/// A number written in a message.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Number {
    pub value: f32,
    /// Whether it was written with a plus or minus in front, which makes it a change rather
    /// than a value to set.
    pub signed: bool,
    /// Whether it was written in kelvin ("5600K").
    pub kelvin: bool,
    /// The word just before it ("to", "by", "at"), which says how it is meant.
    pub after: Option<&'static str>,
}

/// The first number written in a message, with its sign: "+0.5", "-30", "20", "80%".
pub(super) fn number(message: &str) -> Option<Number> {
    let tokens: Vec<&str> = message.split(|character: char| character.is_whitespace() || matches!(character, ',' | '(' | ')' | '?' | '!' | ';')).filter(|token| !token.is_empty()).collect();
    tokens.iter().enumerate().find_map(|(index, token)| {
        let lower = token.to_lowercase().replace('−', "-");
        let bare = lower.trim_end_matches('.');
        let (digits, kelvin_suffix) = match bare.strip_suffix('k') {
            Some(digits) => (digits, true),
            None => (bare.trim_end_matches(['%', '°']).trim_end_matches("px"), false),
        };
        let value = digits.parse::<f32>().ok().filter(|value| value.is_finite() && digits.chars().any(|character| character.is_ascii_digit()))?;
        let next = tokens.get(index + 1).map(|next| next.to_lowercase());
        let kelvin = kelvin_suffix || matches!(next.as_deref().map(|next| next.trim_end_matches('.')), Some("k" | "kelvin" | "kelvins"));
        let after = index.checked_sub(1).and_then(|before| match tokens[before].to_lowercase().as_str() {
            "to" | "at" | "=" => Some("to"),
            "by" => Some("by"),
            _ => None,
        });
        Some(Number { value, signed: digits.starts_with(['+', '-']), kelvin, after })
    })
}

/// How a request asks for a slider to move.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Amount {
    /// To exactly this value.
    To(f32),
    /// By this much from where it is, up or down.
    By(f32),
    /// A step up (positive) or down, this many times the usual size ("a bit" is half).
    Step(f32),
    /// Back to where it starts.
    Reset,
}

/// Words that ask for a slider to go up, and to go down.
const UP: &[&str] = &["up", "raise", "increase", "more", "boost", "add", "plus", "lift", "open", "brighten", "lighten", "brighter", "lighter", "higher", "push", "bump", "strengthen", "stronger"];
const DOWN: &[&str] = &[
    "down", "lower", "decrease", "reduce", "less", "fewer", "minus", "drop", "cut", "darken", "darker", "pull", "recover", "tame", "lessen", "crush", "deepen", "deeper", "dim", "weaker", "soften", "ease",
];
/// Words that ask for a slider to go back to where it starts.
const RESET: &[&str] = &["reset", "default", "zero", "remove", "off", "without", "clear"];

/// How `message` asks for a slider to move, if it says.
pub(super) fn amount(message: &str, said: &[String]) -> Option<Amount> {
    let has = |list: &[&str]| said.iter().any(|word| list.contains(&word.as_str()));
    let too = |list: &[&str]| said.windows(2).any(|pair| pair[0] == "too" && list.contains(&pair[1].as_str()));
    let down = has(DOWN) || too(&["bright", "light", "high", "strong", "much", "harsh"]);
    let up = has(UP) || too(&["dark", "dim", "low", "weak", "little"]);
    let Some(number) = number(message) else {
        if has(RESET) || said.first().is_some_and(|first| first == "no") {
            return Some(Amount::Reset);
        }
        let step = strength(said);
        return if down { Some(Amount::Step(-step)) } else if up { Some(Amount::Step(step)) } else { None };
    };
    // "To 20" and "set … 20" give the value itself, whatever its sign; "by 20" a change.
    if number.after == Some("to") || (has(&["set"]) && number.after != Some("by")) {
        return Some(Amount::To(number.value));
    }
    Some(if down {
        Amount::By(-number.value.abs())
    } else if up {
        Amount::By(number.value.abs())
    } else if number.signed || number.after == Some("by") {
        Amount::By(number.value)
    } else {
        Amount::To(number.value)
    })
}

/// `setting` moved as `amount` says from where it is in `develop`, with what to say about it.
pub(super) fn moved(setting: DevelopSetting, amount: Amount, develop: &Develop) -> (String, Fix) {
    let info = setting.slider();
    let current = setting.get(develop);
    let target = match amount {
        Amount::To(value) => value,
        Amount::By(change) => current + change,
        // No number: a step of a twentieth of the slider's range, more or less.
        Amount::Step(steps) => current + (info.max - info.min) / 20.0 * steps,
        Amount::Reset => info.default,
    };
    let scale = 10f32.powi(info.decimals as i32);
    let value = ((target * scale).round() / scale).clamp(info.min, info.max);
    let shown = format!("{:.*}", info.decimals, value);
    let mut note = if (value - target).abs() > 1e-3 { format!(" (as far as it goes: {} to {})", info.min, info.max) } else { String::new() };
    // The vignette's sign is easy to get backwards, so the reply says which way it went.
    if setting == DevelopSetting::Vignette && value.abs() > 0.5 {
        note.push_str(if value < 0.0 { " (darker corners)" } else { " (lighter corners; a negative value darkens them)" });
    }
    (format!("{} {shown}{note}.", info.name), Fix::settings(info.name, vec![(setting, value)]))
}

/// A slider set or nudged by name: "set exposure to 0.5", "shadows +30", "contrast down 10",
/// "more clarity", "reset highlights".
pub(super) fn slider(message: &str, said: &[String], develop: &Develop) -> Option<(String, Fix)> {
    let setting = named_setting(said)?;
    Some(moved(setting, amount(message, said)?, develop))
}

/// A slider given an exact value, a numbered change or a reset. This is read before a message
/// is taken for a question or a feeling, since a number leaves no doubt about what is wanted.
pub(super) fn exact(message: &str, said: &[String], develop: &Develop) -> Option<(String, Fix)> {
    // "Blue saturation -20" names one color's slider, not the whole photo's Saturation.
    if let Some(found) = colors::slider(message, develop).filter(|_| amount(message, said).is_some_and(|how| !matches!(how, Amount::Step(_)))) {
        return Some(found);
    }
    let setting = named_setting(said)?;
    let how = amount(message, said).filter(|how| !matches!(how, Amount::Step(_)))?;
    Some(moved(setting, how, develop))
}

/// An edit said in words rather than numbers: one color, a look, a feeling, or a slider moved
/// a step. `asked` is whether the message is a question, which only a feeling still answers
/// to ("is it too dark?" was answered already; "should it be warmer?" makes it warmer).
pub(super) fn worded(message: &str, said: &[String], develop: &Develop, asked: bool) -> Option<(String, Fix)> {
    if asked {
        return plain(message, said, develop);
    }
    colors::request(message, develop)
        .or_else(|| looks::request(said, develop, false))
        .or_else(|| plain(message, said, develop))
        .or_else(|| slider(message, said, develop))
        .or_else(|| looks::request(said, develop, true))
}

/// Any one edit a request makes, read against the settings in `develop`.
pub(super) fn one(message: &str, develop: &Develop) -> Option<(String, Fix)> {
    let said = words(message);
    exact(message, &said, develop).or_else(|| worded(message, &said, develop, false))
}

/// Words on either side of an "and" that belong together, so the "and" joins them instead of
/// separating two requests: "black and white", "bright and airy".
const PAIRS: &[(&str, &str)] = &[("black", "white"), ("bright", "airy"), ("light", "airy"), ("teal", "orange"), ("before", "after"), ("crop", "straighten"), ("soft", "dreamy"), ("dark", "moody")];

/// Clauses that say nothing on their own.
const POLITE: &[&str] = &["please", "pls", "thanks", "thank", "you", "ok", "okay", "also", "too", "then", "cool", "nice", "great", "good", "perfect", "yes", "yeah", "sure", "right", "hmm", "oh", "well", "hi", "hello", "hey"];

/// `message` as the requests it lists: split at commas and at "and" (or "&"), except where the
/// "and" is part of a name such as "black and white". A message that lists one request comes
/// back as that one.
pub(super) fn clauses(message: &str) -> Vec<String> {
    let tokens: Vec<&str> = message.split_whitespace().collect();
    let bare = |token: &str| token.trim_matches(|character: char| !character.is_alphanumeric()).to_lowercase();
    let mut found: Vec<String> = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    let mut close = |current: &mut Vec<&str>| {
        let clause = current.join(" ");
        let clause = clause.trim().trim_end_matches([',', ';', '.', '!']).trim();
        let empty = words(clause).iter().all(|word| POLITE.contains(&word.as_str()));
        if !empty {
            found.push(clause.to_owned());
        }
        current.clear();
    };
    for (index, token) in tokens.iter().enumerate() {
        let joins = matches!(token.to_lowercase().as_str(), "and" | "&");
        if joins {
            let before = current.last().map(|last| bare(last));
            let after = tokens.get(index + 1).map(|next| bare(next));
            let paired = matches!((before, after), (Some(before), Some(after)) if PAIRS.contains(&(before.as_str(), after.as_str())));
            if paired {
                current.push(token);
            } else {
                close(&mut current);
            }
            continue;
        }
        // "Exposure 0.5 contrast 20": a second slider with a number of its own starts a new
        // clause, though nothing but a space separates them.
        let names_slider = |token: &str| named_setting(&words(token)).is_some();
        let is_number = |token: &str| number(token).is_some();
        let next_is_number = tokens.get(index + 1).is_some_and(|next| is_number(next));
        if names_slider(token) && next_is_number && current.iter().any(|said| names_slider(said)) && current.iter().any(|said| is_number(said)) {
            close(&mut current);
        }
        current.push(token);
        // A comma ends a clause; one inside a number ("1,5") is not at the end of its word.
        if token.ends_with(',') {
            close(&mut current);
        }
    }
    close(&mut current);
    found
}

/// Small words that may sit beside a slider's name without asking anything of it.
const BESIDE_A_NAME: &[&str] = &["the", "my", "its", "it", "a", "an", "of", "and", "also", "too", "photo", "image", "picture"];

/// A clause that only names a slider ("shadows" in "shadows and highlights up 20") moved the
/// way the nearest clause that does say how asks: the next one first, else the one before.
fn borrowed(clauses: &[String], index: usize, develop: &Develop) -> Option<(String, Fix)> {
    let said = words(&clauses[index]);
    let setting = named_setting(&said)?;
    let names: Vec<&str> = SETTING_NAMES.iter().filter(|(_, named)| *named == setting).flat_map(|(names, _)| names.iter().copied()).collect();
    let only_the_name = said.iter().all(|word| names.contains(&word.as_str()) || BESIDE_A_NAME.contains(&word.as_str()));
    if !only_the_name {
        return None;
    }
    let how = |clause: &String| {
        let said = words(clause);
        named_setting(&said)?;
        amount(clause, &said)
    };
    let lent = clauses[index + 1..].iter().find_map(how).or_else(|| clauses[..index].iter().rev().find_map(how))?;
    Some(moved(setting, lent, develop))
}

/// Several edits listed in one request, as one fix: each is read against the settings the ones
/// before it leave, and the reply says each. `None` unless every clause is an edit, so a list
/// that holds anything else is left for the caller to read another way.
pub(super) fn list(clauses: &[String], develop: &Develop) -> Option<(String, Fix)> {
    let mut now = develop.clone();
    let mut parts: Vec<(String, Fix)> = Vec::new();
    for (index, clause) in clauses.iter().enumerate() {
        let (text, fix) = one(clause, &now).or_else(|| borrowed(clauses, index, &now))?;
        // "It's too dark, make it brighter" says one thing twice: it is made once.
        let said_twice = !clause.chars().any(|character| character.is_ascii_digit()) && parts.iter().any(|(_, made)| made.label == fix.label);
        if said_twice {
            continue;
        }
        now = fix.apply_to(&now);
        parts.push((text, fix));
    }
    if parts.len() < 2 {
        return parts.pop();
    }
    // Each setting touched, once, at the value the whole list leaves it at.
    let mut changes: Vec<(DevelopSetting, f32)> = Vec::new();
    for (setting, _) in parts.iter().flat_map(|(_, fix)| &fix.changes) {
        if !changes.iter().any(|(listed, _)| listed == setting) {
            changes.push((*setting, setting.get(&now)));
        }
    }
    let label = if parts.len() <= 3 { parts.iter().map(|(_, fix)| fix.label.as_str()).collect::<Vec<_>>().join(", ") } else { format!("{} Edits", parts.len()) };
    let lines: Vec<String> = parts.iter().map(|(text, _)| format!("• {}", text.trim_end_matches('.'))).collect();
    Some((format!("{} changes, as one step:\n{}", parts.len(), lines.join("\n")), Fix::settings(label, changes)))
}

/// Every word these readers listen for, for putting a mistyped message right.
pub(super) fn vocabulary() -> Vec<&'static str> {
    let named = SETTING_NAMES.iter().flat_map(|(names, _)| names.iter().copied());
    let own = [
        "brighter", "lighter", "brighten", "lighten", "darker", "darken", "dimmer", "underexposed", "overexposed", "blown", "contrast", "contrasty", "harsh", "softer", "punchier", "washed", "warmer", "cooler", "colder", "saturated", "colorful", "vivid",
        "garish", "desaturate", "muted", "vibrant", "saturate", "sharpen", "sharper", "crisper", "blurry", "vignette", "corners", "edges", "grain", "grainy", "noise", "noisy", "denoise", "dehaze", "hazy", "foggy", "monochrome", "grayscale", "highlights",
        "shadows", "whites", "blacks", "exposure", "little", "slightly", "really", "stops",
    ];
    named.chain(UP.iter().copied()).chain(DOWN.iter().copied()).chain(RESET.iter().copied()).chain(own).collect()
}

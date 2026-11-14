//! Talking to Iris about a photo. As in Studio, a typed message is matched against a fixed
//! list of things people ask (by the words it contains) and answered from what the photo
//! records and what the rules measured: how bright it is, what's clipping, what has been
//! changed. It can also make edits asked for in plain words ("a bit brighter", "less
//! contrast", "warmer", "black and white"), each one undo step. There is no AI here and
//! nothing is made up: a message that fits nothing gets a reply saying so, with what can be asked.
//!
//! The readers are kept by what they read:
//! - `edits`: sliders and plain-word edits, alone or several in one request;
//! - `colors`: one color at a time, through the Color Mixer;
//! - `looks`: looks by a word ("moody", "bright and airy");
//! - `requests`: the crop, exporting, the other open photos, snapshots and presets.
//!
//! A message that joins requests of different kinds with "and" ("warmer and crop to square")
//! is put apart by the app before it gets here, since the app knows its menu commands and
//! recipes too; what arrives is one request, or a list of edits for `edits` to make together.
//!
//! This file puts them in order, answers the questions, and holds what they share. A reader
//! only says what is asked for (an `Action`); the app carries it out.
//!
//! Messages starting with `/` are commands (`/help`, `/check`, `/fix`…); see `COMMANDS`.

mod colors;
mod edits;
pub mod looks;
mod requests;

pub use requests::{Each, ExportAsk, FolderJob, FolderPlace, OtherApp, SizeAsk, SyncParts, Where, quote_name};

use crate::assistant::{Area, Check, Fix, Measure, Suggestion};
use crate::develop::{Develop, DevelopSetting};
use crate::edit::Edit;
use crate::photo::Photo;

/// What Iris can see when it answers.
pub struct Scene<'a> {
    pub photo: &'a Photo,
    /// The finished photo's measurements, once a render of it has been measured.
    pub measure: Option<&'a Measure>,
    /// What the rules currently notice.
    pub suggestions: &'a [Suggestion],
    /// What Iris is called.
    pub name: &'a str,
    /// The open photos' names, in the order of their tabs.
    pub photo_names: &'a [String],
    /// Which of the open photos this one is.
    pub active: usize,
    /// The presets that can be applied by name, built-in and your own.
    pub presets: &'a [String],
    /// Whether this photo has changes that aren't saved yet.
    pub unsaved: bool,
}

/// Something the app does for a reply, beyond showing its words.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Empty the conversation.
    Clear,
    /// Show the list of suggestions.
    ShowSuggestions,
    /// Make every suggested fix.
    FixAll,
    /// Make the photo look good: Auto for the light where it hasn't been set by hand, then
    /// every suggested fix.
    Improve,
    /// Open Settings.
    OpenSettings,
    /// Call Iris something else.
    Rename(String),
    /// Give the next tip.
    Tip,
    /// Set these Develop settings (one undo step named by the fix's label). Boxed, as a fix can
    /// carry a whole mask's settings.
    Adjust(Box<Fix>),
    /// Lightroom's Auto.
    Auto,
    /// Every Develop setting back to its default.
    Reset,
    /// Everything back as it came: the settings, the crop and the masks.
    ResetAll,
    /// Apply the preset with this name.
    ApplyPreset(String),
    Undo,
    Redo,
    /// Turn the photo this many quarter turns, clockwise or not.
    Rotate { clockwise: bool, turns: u8 },
    /// Mirror the photo, left to right or top to bottom.
    Flip { horizontal: bool },
    /// How the viewer compares: `None` for just the edit, `Some(false)` for Before, `Some(true)`
    /// for the side-by-side split.
    Compare(Option<bool>),
    /// Save the edit as a snapshot, with this name if one was given.
    Snapshot(Option<String>),
    /// Bring back the snapshot at this place in the photo's list.
    RestoreSnapshot(usize),
    /// Save the settings as a preset with this name, or ask for a name when none was given.
    SavePreset(Option<String>),
    /// Copy this photo's settings and framing.
    CopySettings,
    /// Paste what was copied onto this photo: its look (the Develop settings and masks, as
    /// Paste Settings does), its crop carried over in proportion, or both.
    PasteSettings { look: bool, crop: bool },
    /// Hold the crop to the shape at this place in `crop::ASPECTS`. `upright` asks for it
    /// taller than wide (or wider than tall); `None` keeps the way the crop lies now.
    Crop { aspect: usize, upright: Option<bool> },
    /// Stand the crop upright or lay it on its side; `None` swaps whichever it is now.
    TurnCrop(Option<bool>),
    /// Crop in by this share of the width and height (0.1 is a tenth off each), keeping the
    /// crop's shape and middle. A negative share crops back out.
    TightenCrop(f32),
    /// Straighten by the tilt measured in the photo's own lines, if they show one clearly.
    Level,
    /// The crop and straightening back to none.
    ResetCrop,
    /// Set the straighten angle, in degrees clockwise.
    Straighten(f32),
    /// Open the Crop & Straighten tool.
    OpenCrop,
    /// Show another of the open photos.
    Go(Where),
    /// Copy these parts of this photo's look onto every other open photo.
    Sync(SyncParts),
    /// Do this to every open photo.
    EachPhoto(Each),
    /// Open Export with these choices made.
    Export(ExportAsk),
    /// Do something to every photo in a folder, without opening them as tabs.
    Folder(FolderJob),
    /// Send the photo, as edited, to another of the Omina apps.
    SendTo(OtherApp),
    /// Show the words with these short replies to pick from.
    Offer(Vec<String>),
    /// Show the words as the full help, with what every app's Iris understands added.
    Help,
}

/// Iris's answer to one message.
#[derive(Clone, Debug, PartialEq)]
pub enum Reply {
    /// Words to show.
    Say(String),
    /// Words to show, and something for the app to do.
    Do(String, Action),
    /// Some of what the message lists was followed and some was not, so nothing should be
    /// done: doing half would leave the photo somewhere nobody asked for.
    Partly { followed: Vec<String>, missed: Vec<String> },
    /// Nothing about the photo answers it; the app may look in Iris's shared help instead.
    Unknown,
}

/// The commands, with what each does, in the order `/help` lists them.
pub const COMMANDS: &[(&str, &str)] = &[
    ("/help", "List these commands and what you can ask"),
    ("/clear", "Clear this conversation"),
    ("/check", "Show what I've noticed in this photo"),
    ("/fix", "Make every fix I suggest at once"),
    ("/ready", "Check the photo over before you export it"),
    ("/info", "This photo's size, file and crop"),
    ("/edits", "What has been changed so far"),
    ("/auto", "Set the light from the photo (Auto)"),
    ("/presets", "List the presets I can apply by name"),
    ("/looks", "List the looks I can nudge a photo toward"),
    ("/undo", "Undo the last change (/redo puts it back)"),
    ("/reset", "Put every Develop setting back to zero"),
    ("/recipes", "List the recipes I know"),
    ("/teach", "Teach me a recipe: /teach tidy = auto; sharpen"),
    ("/forget", "Forget a recipe: /forget tidy"),
    ("/tip", "Give a tip"),
    ("/name", "Rename me: /name Pixel"),
    ("/settings", "Open Settings"),
];

/// Words that end in "s" without being the plural of anything, so they are kept whole.
const NOT_PLURALS: &[&str] = &["this", "previous", "minus", "plus", "does", "always", "focus", "across", "canvas", "status"];

/// The words of a message as they were typed: lower case, punctuation dropped.
fn raw_words(message: &str) -> Vec<String> {
    message.split(|character: char| !character.is_alphanumeric() && character != '&').filter(|word| !word.is_empty()).map(str::to_lowercase).collect()
}

/// The words of a message as they are compared: lower case, punctuation dropped, plurals
/// reduced ("highlights" matches "highlight") except for very short words.
fn words(message: &str) -> Vec<String> {
    raw_words(message)
        .into_iter()
        .map(|word| match word.strip_suffix('s') {
            Some(single) if single.chars().count() >= 3 && !single.ends_with('s') && !NOT_PLURALS.contains(&word.as_str()) => single.to_owned(),
            _ => word,
        })
        .collect()
}

/// Whether a message is a question rather than a request ("is it too bright?" against "too
/// bright, fix it").
fn is_question(message: &str, said: &[String]) -> bool {
    message.trim_end().ends_with('?') || said.first().is_some_and(|first| ["is", "are", "does", "do", "am", "what", "which", "how", "why", "where", "should", "can", "could"].contains(&first.as_str()))
}

/// Whether a message asks politely for something to be done ("can you make it warmer?"),
/// which is a request however it is punctuated.
fn is_polite_request(said: &[String]) -> bool {
    let first = said.first().map(String::as_str);
    first == Some("please") || (matches!(first, Some("can" | "could" | "would" | "will")) && said.get(1).is_some_and(|second| second == "you"))
}

/// How strongly a request asks: "a little" halves it, "a lot" doubles it. A word that follows
/// "too" is a complaint, not a size ("too much", "too strong"), so it doesn't count.
fn strength(said: &[String]) -> f32 {
    let has = |list: &[&str]| said.iter().enumerate().any(|(index, word)| list.contains(&word.as_str()) && !(index > 0 && said[index - 1] == "too"));
    if has(&["little", "bit", "slightly", "slight", "touch", "tad", "tiny", "smidge", "subtle", "subtly", "gentle", "gently", "hint"]) {
        0.5
    } else if has(&["lot", "much", "way", "really", "very", "loads", "heap", "far", "strong", "strongly", "heavy", "heavily", "significantly"]) {
        2.0
    } else {
        1.0
    }
}

/// A setting's name as a reply says it. The Color Mixer's and Color Grading's sliders share
/// short names ("Blue", "Hue"), so theirs say which control they belong to.
pub fn described(setting: DevelopSetting) -> String {
    let name = setting.slider().name;
    match setting {
        DevelopSetting::MixerHue(_) => format!("{name} Hue"),
        DevelopSetting::MixerSaturation(_) => format!("{name} Saturation"),
        DevelopSetting::MixerLuminance(_) => format!("{name} Luminance"),
        DevelopSetting::Grade(wheel, _) => format!("{} tint {}", ["Shadows", "Midtones", "Highlights"][wheel.min(2)], name.to_lowercase()),
        _ => name.to_owned(),
    }
}

/// A setting with a value, as a reply says it: "Contrast +20", "Sharpening 40". Sliders that
/// only run upward from zero read better without a plus.
pub fn valued(setting: DevelopSetting, value: f32) -> String {
    let info = setting.slider();
    if info.min >= 0.0 { format!("{} {:.*}", described(setting), info.decimals, value) } else { format!("{} {:+.*}", described(setting), info.decimals, value) }
}

/// What a fix sets, in words: "Contrast +15, Blue Saturation +20".
pub fn listed_changes(fix: &Fix) -> String {
    fix.changes.iter().map(|(setting, value)| valued(*setting, *value)).collect::<Vec<_>>().join(", ")
}

/// Every slider there is, in the Develop panel's order.
fn all_settings() -> Vec<DevelopSetting> {
    let mut settings: Vec<DevelopSetting> = DevelopSetting::BASIC.to_vec();
    for band in 0..crate::develop::MIXER_BANDS.len() {
        settings.extend([DevelopSetting::MixerHue(band), DevelopSetting::MixerSaturation(band), DevelopSetting::MixerLuminance(band)]);
    }
    for wheel in 0..3 {
        settings.extend((0..3).map(|value| DevelopSetting::Grade(wheel, value)));
    }
    settings.extend([DevelopSetting::GradeBlending, DevelopSetting::GradeBalance]);
    settings.extend(DevelopSetting::DETAIL);
    settings.extend(DevelopSetting::EFFECTS);
    settings
}

/// What differs between two edits, each difference on a line of its own: every slider that
/// moved (from its value in `from` to its value in `to`), then the tone curve, crop and masks.
fn differences(from: &Edit, to: &Edit) -> Vec<String> {
    let mut found: Vec<String> = all_settings()
        .into_iter()
        .filter(|setting| (setting.get(&from.develop) - setting.get(&to.develop)).abs() > 1e-3)
        .map(|setting| {
            let decimals = setting.slider().decimals;
            format!("{}: {:.*} → {:.*}", described(setting), decimals, setting.get(&from.develop), decimals, setting.get(&to.develop))
        })
        .collect();
    let curves = |develop: &Develop| [develop.curve.clone(), develop.curve_red.clone(), develop.curve_green.clone(), develop.curve_blue.clone()];
    if curves(&from.develop) != curves(&to.develop) {
        found.push("The tone curve changed".to_owned());
    }
    if from.crop != to.crop {
        found.push("The crop changed".to_owned());
    }
    if from.masks != to.masks {
        found.push(format!("The masks changed ({} then, {} now)", from.masks.len(), to.masks.len()));
    }
    found
}

/// The presets that can be applied, as a reply.
fn preset_list(scene: &Scene) -> String {
    if scene.presets.is_empty() {
        return "There are no presets.".to_owned();
    }
    format!("I can apply any of these: {}. Say \"try Vivid\", say.", scene.presets.join(", "))
}

/// The looks that can be asked for, as a reply.
fn look_list() -> String {
    format!(
        "I can nudge a photo toward any of these looks: {}. Say “make it moody”, “a bit cinematic” or “very vintage”; I'll say which sliders moved, and “less moody” eases it back. (They build on your edit; a preset sets whole groups of settings instead.)",
        looks::names().join(", ")
    )
}

/// The preset a message asks to apply, by its name (the longest name written in it).
fn preset_request(message: &str, said: &[String], scene: &Scene) -> Option<String> {
    let lower = message.to_lowercase();
    let asks = said.iter().any(|word| ["apply", "use", "try", "preset", "give", "add", "put"].contains(&word.as_str()));
    let named = scene.presets.iter().filter(|name| lower.contains(&name.to_lowercase())).max_by_key(|name| name.len())?;
    (asks || lower.trim() == named.to_lowercase()).then(|| named.clone())
}

/// Things to do that aren't Develop settings: undo, turning, flipping and comparing.
fn command_request(said: &[String]) -> Option<Reply> {
    let has = |list: &[&str]| said.iter().any(|word| list.contains(&word.as_str()));
    let only = |list: &[&str]| said.iter().all(|word| list.contains(&word.as_str()) || ["please", "it", "that", "the", "photo", "image", "picture", "one", "last", "change", "edit"].contains(&word.as_str()));
    if has(&["undo"]) && only(&["undo"]) {
        return Some(Reply::Do("Undone.".to_owned(), Action::Undo));
    }
    if has(&["redo"]) && only(&["redo"]) {
        return Some(Reply::Do("Redone.".to_owned(), Action::Redo));
    }
    // "Turn" is said of sliders too ("turn the contrast down"), so on its own it needs a
    // direction, or nothing else beside it.
    let turns_photo = has(&["rotate"]) || (has(&["turn"]) && (has(&["left", "right", "clockwise", "counterclockwise", "anticlockwise", "ccw", "sideway", "around", "quarter", "upside"]) || only(&["turn"])));
    if turns_photo && edits::named_setting(said).is_none() {
        if has(&["upside", "around"]) {
            return Some(Reply::Do("Turned upside down (two quarter turns).".to_owned(), Action::Rotate { clockwise: true, turns: 2 }));
        }
        let clockwise = !has(&["left", "counterclockwise", "anticlockwise", "ccw"]);
        return Some(Reply::Do(format!("Turned a quarter turn {}.", if clockwise { "right" } else { "left" }), Action::Rotate { clockwise, turns: 1 }));
    }
    if has(&["flip", "mirror"]) {
        let horizontal = !has(&["vertical", "vertically", "upside"]);
        return Some(Reply::Do(format!("Flipped {}.", if horizontal { "left to right" } else { "top to bottom" }), Action::Flip { horizontal }));
    }
    let side_by_side = said.iter().filter(|word| *word == "side").count() >= 2;
    if has(&["split"]) || side_by_side || (has(&["side"]) && has(&["before", "after", "compare"])) || (has(&["compare"]) && only(&["compare"])) {
        return Some(Reply::Do("Before on the left, after on the right. Drag the divider to compare.".to_owned(), Action::Compare(Some(true))));
    }
    if has(&["before", "original"]) && has(&["show", "see", "view", "compare", "before", "original"]) && !has(&["hide"]) {
        return Some(Reply::Do("Showing the photo as it came. Say \"show after\" (or press \\) to go back.".to_owned(), Action::Compare(Some(false))));
    }
    if (has(&["after"]) && has(&["show", "back", "see", "view", "after"])) || (has(&["hide"]) && has(&["before", "original", "split", "comparison", "compare"])) {
        return Some(Reply::Do("Showing your edit.".to_owned(), Action::Compare(None)));
    }
    None
}

/// What the rules have noticed, as a reply; only those in `area` when given.
fn problems(scene: &Scene, area: Option<Area>) -> Reply {
    let found: Vec<&Suggestion> = scene.suggestions.iter().filter(|suggestion| area.is_none_or(|area| suggestion.check.area() == area)).collect();
    let about = area.map_or_else(|| "this photo".to_owned(), |area| format!("its {}", area.name().to_lowercase()));
    if scene.measure.is_none() {
        return Reply::Say("I'm still measuring the photo. Ask me again in a moment.".to_owned());
    }
    if found.is_empty() {
        return Reply::Say(format!("I don't see anything wrong with {about}. Nothing is badly clipped, too dark or too bright as far as my rules can tell."));
    }
    let lines: Vec<String> = found.iter().map(|suggestion| format!("• {} — {}", suggestion.title, suggestion.evidence)).collect();
    let fixable = found.iter().filter(|suggestion| suggestion.fix.is_some()).count();
    let tail = if fixable > 0 { "\nSay \"fix it\" (or /fix) to make my fixes, or open Suggestions to pick." } else { "" };
    Reply::Do(format!("Here's what I notice in {about}:\n{}{tail}", lines.join("\n")), Action::ShowSuggestions)
}

/// The photo's size, file and crop.
fn info(scene: &Scene) -> String {
    let photo = scene.photo;
    let (width, height) = photo.source.size();
    let (out_width, out_height) = photo.edit.output_size((width, height));
    let megapixels = f64::from(width) * f64::from(height) / 1_000_000.0;
    let mut lines = vec![
        format!("{} ({})", photo.name, photo.source.file_name),
        format!("• Original: {width} × {height} pixels ({megapixels:.1} megapixels)"),
    ];
    if (out_width, out_height) != (width, height) {
        lines.push(format!("• Cropped to {out_width} × {out_height}"));
    }
    if photo.edit.crop.angle.abs() > 0.005 {
        lines.push(format!("• Straightened {:+.2}°", photo.edit.crop.angle));
    }
    lines.push(format!("• {} mask{}, {} snapshot{}", photo.edit.masks.len(), if photo.edit.masks.len() == 1 { "" } else { "s" }, photo.snapshots.len(), if photo.snapshots.len() == 1 { "" } else { "s" }));
    if scene.photo_names.len() > 1 {
        lines.push(format!("• {} photos are open", scene.photo_names.len()));
    }
    lines.join("\n")
}

/// Every Develop setting moved from its default, by name.
fn edits(scene: &Scene) -> String {
    let develop = &scene.photo.edit.develop;
    let changed: Vec<String> = all_settings().into_iter().filter(|setting| (setting.get(develop) - setting.slider().default).abs() > 1e-3).map(|setting| valued(setting, setting.get(develop))).collect();
    let mut extras = Vec::new();
    if develop.curve != crate::adjust::identity_curve() || develop.curve_red != crate::adjust::identity_curve() || develop.curve_green != crate::adjust::identity_curve() || develop.curve_blue != crate::adjust::identity_curve() {
        extras.push("a tone curve".to_owned());
    }
    if !scene.photo.edit.crop.is_neutral() {
        extras.push("a crop".to_owned());
    }
    let masks = scene.photo.edit.masks.iter().filter(|mask| mask.is_active()).count();
    if masks > 0 {
        extras.push(format!("{masks} mask{}", if masks == 1 { "" } else { "s" }));
    }
    if changed.is_empty() && extras.is_empty() {
        return "Nothing yet: the photo is just as it came.".to_owned();
    }
    let mut reply = String::from("So far:");
    if !changed.is_empty() {
        reply.push_str(&format!("\n• {}", changed.join(", ")));
    }
    if !extras.is_empty() {
        reply.push_str(&format!("\n• Plus {}", extras.join(", ")));
    }
    reply
}

/// How bright the photo is and what's clipping, in words.
fn brightness(scene: &Scene) -> String {
    let Some(m) = scene.measure else { return "I'm still measuring the photo. Ask me again in a moment.".to_owned() };
    let overall = match m.median {
        value if value > 0.66 => "bright overall",
        value if value > 0.55 => "on the bright side",
        value if value < 0.24 => "dark overall",
        value if value < 0.35 => "on the dark side",
        _ => "about right overall",
    };
    format!(
        "It's {overall}: half the photo is darker than {:.0}% brightness. The darkest 2% are below {:.0}% and the brightest 2% above {:.0}%. {} is clipped to white and {} to black.",
        m.median * 100.0,
        m.low * 100.0,
        m.high * 100.0,
        clipped(m.highlights_clipped),
        clipped(m.shadows_clipped)
    )
}

fn clipped(share: f32) -> String {
    if share < 0.0005 { "Nothing".to_owned() } else { format!("{:.1}%", share * 100.0) }
}

/// Where the white balance is set, and which way the photo's grays lean now.
fn white_balance(scene: &Scene) -> String {
    let develop = &scene.photo.edit.develop;
    let set = format!(
        "Temperature is at {:+.0} and Tint at {:+.0}. Each runs from -100 to +100, with 0 as the photo came: Temperature toward blue or amber, Tint toward green or magenta.",
        develop.temperature, develop.tint
    );
    let measured = match scene.measure.map(|measure| measure.neutral) {
        None => " I'm still measuring the photo, so ask again in a moment for how its grays lean.".to_owned(),
        Some(None) => " Too little of the photo is near gray for me to judge a cast.".to_owned(),
        Some(Some([red, green, blue])) => {
            let warm = red - blue;
            let greenish = green - (red + blue) * 0.5;
            let lean = if warm.abs() < 0.02 && greenish.abs() < 0.02 {
                "close to neutral"
            } else if warm.abs() >= greenish.abs() {
                if warm > 0.0 { "leaning warm" } else { "leaning cool" }
            } else if greenish > 0.0 {
                "leaning green"
            } else {
                "leaning magenta"
            };
            format!(" Its near-gray parts now average red {:.0}%, green {:.0}%, blue {:.0}%: {lean}.", red * 100.0, green * 100.0, blue * 100.0)
        }
    };
    format!("{set}{measured}")
}

/// Where one slider is set, with its range.
fn value_of(setting: DevelopSetting, develop: &Develop) -> String {
    let info = setting.slider();
    let moved = if (setting.get(develop) - info.default).abs() < 1e-3 { ", where it starts" } else { "" };
    format!("{} is at {:.*}{moved}. It runs from {} to {}.", info.name, info.decimals, setting.get(develop), info.min, info.max)
}

/// One answer to "is this ready?": what was measured in the light, the color, the detail, the
/// frame and the file, a plain verdict, and what to do next.
fn ready(scene: &Scene) -> Reply {
    let Some(m) = scene.measure else { return Reply::Say("I'm still measuring the photo. Ask me again in a moment.".to_owned()) };
    let photo = scene.photo;
    let develop = &photo.edit.develop;
    let noticed = |area: Area| scene.suggestions.iter().filter(|suggestion| suggestion.check.area() == area).map(|suggestion| suggestion.title.clone()).collect::<Vec<_>>();
    // What was measured, then anything the rules make of it.
    let with_notes = |measured: String, notes: Vec<String>| if notes.is_empty() { measured } else { format!("{measured} {}.", notes.join(". ")) };
    let light = if m.highlights_clipped < 0.0005 && m.shadows_clipped < 0.0005 {
        "Nothing is clipped to pure white or black.".to_owned()
    } else {
        format!("{} is clipped to white and {} to black.", clipped(m.highlights_clipped), clipped(m.shadows_clipped).to_lowercase())
    };
    let mut tone_notes = noticed(Area::Light);
    tone_notes.extend(noticed(Area::Contrast));
    let color_notes = noticed(Area::Color);
    let color = if develop.saturation <= -95.0 {
        "Black and white.".to_owned()
    } else if color_notes.is_empty() {
        format!("No cast or color problem I can measure (Temperature {:+.0}, Tint {:+.0}).", develop.temperature, develop.tint)
    } else {
        format!("Temperature {:+.0}, Tint {:+.0}.", develop.temperature, develop.tint)
    };
    let detail = format!("Sharpening {:.0}, noise reduction {:.0}.", develop.sharpen_amount, develop.noise_luminance);
    let original = photo.source.size();
    let (width, height) = photo.edit.output_size(original);
    let mut frame = if photo.edit.crop.rect.is_some() { format!("Cropped to {width} × {height} pixels") } else { format!("Not cropped: {width} × {height} pixels") };
    if photo.edit.crop.angle.abs() > 0.005 {
        frame.push_str(&format!(", straightened {:+.2}°", photo.edit.crop.angle));
    }
    let file = if scene.unsaved { "It has changes that aren't saved yet." } else { "Saved." };
    let lines = [
        format!("• Light: {}", with_notes(light, tone_notes)),
        format!("• Color: {}", with_notes(color, color_notes)),
        format!("• Detail: {}", with_notes(detail, noticed(Area::Detail))),
        format!("• Frame: {frame}."),
        format!("• File: {file}"),
    ];
    let open = scene.suggestions.len();
    let fixable = scene.suggestions.iter().any(|suggestion| suggestion.fix.is_some());
    let verdict = match open {
        0 => "Verdict: ready to export. Nothing I measure is out of the ordinary.".to_owned(),
        1 => "Verdict: nearly. There is one thing I'd look at first.".to_owned(),
        count => format!("Verdict: not quite. There are {count} things I'd look at first."),
    };
    let mut offers = Vec::new();
    if fixable {
        offers.push("Fix it".to_owned());
    }
    offers.push("Export".to_owned());
    if scene.unsaved {
        offers.push("Save".to_owned());
    }
    Reply::Do(format!("Ready check for {}:\n{}\n{verdict}", photo.name, lines.join("\n")), Action::Offer(offers))
}

fn help(scene: &Scene) -> String {
    let commands: Vec<String> = COMMANDS.iter().map(|(command, about)| format!("{command} — {about}")).collect();
    format!(
        "I'm {}. I check your photo for problems like blown highlights, crushed shadows, too much contrast, a bright sky or a color cast, and I make edits when you ask.\n\nTry:\n• Questions: \"what's wrong?\", \"is it overexposed?\", \"what did I change?\", \"what's the white balance?\", \"is this ready?\"\n• Edits, several at once if you like: \"a bit brighter\", \"shadows +30, highlights -20 and a bit warmer\", \"set exposure to -0.5\", \"add a vignette\", \"reduce the noise\"\n• Roughly: \"make it look good\", \"make it pop\", \"more dramatic\", \"more subtle\", \"clean it up\", \"too dark\"\n• Looks: \"make it moody\", \"a bit cinematic\", \"bright and airy\", or a preset: \"try Vivid\"\n• One color: \"make the sky bluer\", \"desaturate the oranges\", \"softer skin tones\"\n• The frame: \"crop to 16:9\", \"crop it tighter\", \"straighten it\", \"straighten 1.5 degrees\", \"rotate left\"\n• Every open photo: \"sync to the others\", \"same for all the others\", \"auto all photos\", \"next photo\"\n• A whole folder, without opening it: \"apply portrait finish to every photo in this folder and export\", \"export everything in ~/Pictures/Trip as JPEG\"\n• Presets for export: \"export for Web\", \"export for Instagram\", \"save these export settings as Proofs\"\n• The other apps: \"open this photo in Edit\", \"put this photo in Motion\"\n• Keeping a look: \"save a snapshot called Before\", \"go back to the snapshot Before\", \"save this as a preset called Golden\", \"copy these settings\"\n• Finishing: \"export as JPEG at 80% quality, 2048 pixels wide\" (writes the file), \"export all\", \"show before\"\n• Different things at once: \"warmer and crop to square\", \"sharpen and save a snapshot called Sharp\"\n\n{}",
        scene.name,
        commands.join("\n")
    )
}

/// The reply to a command (a message starting with `/`).
fn command(message: &str, scene: &Scene) -> Reply {
    let mut parts = message.trim().splitn(2, char::is_whitespace);
    let name = parts.next().unwrap_or_default().to_lowercase();
    let rest = parts.next().unwrap_or_default().trim();
    match name.as_str() {
        "/help" | "/?" => Reply::Do(help(scene), Action::Help),
        "/clear" => Reply::Do(String::new(), Action::Clear),
        "/check" => problems(scene, None),
        "/fix" => Reply::Do(String::new(), Action::FixAll),
        "/ready" => ready(scene),
        "/info" => Reply::Say(info(scene)),
        "/edits" => Reply::Say(edits(scene)),
        "/auto" => Reply::Do("Auto: the light set from the photo.".to_owned(), Action::Auto),
        "/reset" => Reply::Do("Every Develop setting is back to zero (the crop and masks stay). Undo brings them back.".to_owned(), Action::Reset),
        "/tip" => Reply::Do(String::new(), Action::Tip),
        "/presets" => Reply::Say(preset_list(scene)),
        "/looks" => Reply::Say(look_list()),
        "/undo" => Reply::Do("Undone.".to_owned(), Action::Undo),
        "/redo" => Reply::Do("Redone.".to_owned(), Action::Redo),
        "/settings" => Reply::Do("Opening Settings.".to_owned(), Action::OpenSettings),
        "/name" if rest.is_empty() => Reply::Say("What should I be called? Type /name and the name, like /name Pixel.".to_owned()),
        "/name" => Reply::Do(String::new(), Action::Rename(rest.to_owned())),
        // A recipe that exists is run before a message gets here, so this one doesn't.
        "/forget" if !rest.is_empty() => Reply::Say(format!("I don't know a recipe called “{rest}”. /recipes lists the ones I know.")),
        "/forget" => Reply::Say("Which recipe should I forget? Type /forget and its name; /recipes lists them.".to_owned()),
        _ => Reply::Say(format!("I don't know {name}. Type /help for the commands I understand.")),
    }
}

/// Iris's reply to `message` about the photo in `scene`.
pub fn reply(message: &str, scene: &Scene) -> Reply {
    let trimmed = message.trim();
    if trimmed.starts_with('/') {
        return command(trimmed, scene);
    }
    listed(trimmed, scene).unwrap_or_else(|| single(trimmed, scene))
}

/// A message that lists several requests ("shadows +30, highlights -20 and a bit warmer"),
/// read as its parts. `None` when it holds one request, or is better read whole.
fn listed(message: &str, scene: &Scene) -> Option<Reply> {
    let said = words(message);
    // A question is one thing asked, and a name being given may hold an "and" of its own.
    if (is_question(message, &said) && !is_polite_request(&said)) || requests::names_something(message) {
        return None;
    }
    let clauses = edits::clauses(message);
    if clauses.len() < 2 {
        return None;
    }
    // All edits: they are made together, as one step.
    if let Some((text, fix)) = edits::list(&clauses, &scene.photo.edit.develop) {
        return Some(Reply::Do(text, Action::Adjust(Box::new(fix))));
    }
    let replies: Vec<Reply> = clauses.iter().map(|clause| single(clause, scene)).collect();
    let acts = |reply: &Reply| matches!(reply, Reply::Do(..));
    // Requests of different kinds are told apart by the app before a message gets here (it
    // knows its menu commands too), so a list like that is read whole. What is left to catch
    // is a slider edit beside something that isn't a request: read whole, the edit would be
    // made and the rest dropped without a word, so say what was missed instead.
    if replies.iter().all(acts) || !replies.iter().any(|reply| matches!(reply, Reply::Do(_, Action::Adjust(_)))) {
        return None;
    }
    let (mut followed, mut missed) = (Vec::new(), Vec::new());
    for (clause, reply) in clauses.into_iter().zip(&replies) {
        if acts(reply) { followed.push(clause) } else { missed.push(clause) }
    }
    Some(Reply::Partly { followed, missed })
}

/// Iris's reply to one request.
fn single(trimmed: &str, scene: &Scene) -> Reply {
    let said = words(trimmed);
    let has = |list: &[&str]| said.iter().any(|word| list.contains(&word.as_str()));
    let question = is_question(trimmed, &said) && !is_polite_request(&said);
    let develop = &scene.photo.edit.develop;

    if requests::asks_ready(trimmed, &said) {
        return ready(scene);
    }
    // A folder of photos, or another app, is named before anything about this photo is read:
    // "export everything in this folder" is not "export".
    if !question && let Some(reply) = requests::folder(trimmed) {
        return reply;
    }
    if !question && let Some(reply) = requests::send_to(&said) {
        return reply;
    }
    // "Change it", "do something": what to change isn't said, so it is asked rather than guessed.
    const VAGUE: &[&str] = &["change", "adjust", "edit", "tweak", "different", "something", "anything", "it", "the", "this", "photo", "image", "picture", "make", "do", "please", "to", "a", "bit", "up", "look", "with"];
    // ("Edits" and "changes" on their own ask for the list of them.)
    let lists_edits = raw_words(trimmed).iter().any(|word| word == "edits" || word == "changes");
    if !trimmed.ends_with('?') && !lists_edits && has(&["change", "adjust", "edit", "tweak", "different", "something", "anything"]) && said.iter().all(|word| VAGUE.contains(&word.as_str())) {
        let offers = ["Make it look good", "A bit brighter", "More contrast", "Warmer", "Make it moody", "Crop it tighter"].map(str::to_owned).to_vec();
        return Reply::Do("What would you like changed? Pick one, or say it your own way: the light (“brighter”, “shadows +20”), the color (“warmer”, “make the sky bluer”), a look (“moody”), or the frame (“crop to 16:9”).".to_owned(), Action::Offer(offers));
    }
    // Fixing what was noticed, and making the photo look good without saying how.
    let fixes = has(&["fix", "repair", "correct", "improve", "enhance", "polish"]) && (has(&["all", "everything", "it", "them", "up", "photo", "this", "image", "picture"]) || said.len() <= 2);
    let looks_good = has(&["make"]) && (has(&["better", "nicer"]) || (has(&["look"]) && has(&["good", "better", "nice", "nicer", "great", "best", "right"])));
    if (fixes || looks_good) && !question {
        if requests::everyone(&said) && scene.photo_names.len() > 1 {
            return Reply::Say("I fix one photo at a time, since each has to be measured as it shows. Say “fix it” for this one, then “next photo”.".to_owned());
        }
        // "Fix them all" is the suggestions and nothing else; "fix it" is whatever it takes.
        let only_the_suggestions = has(&["all", "everything", "them"]) && !looks_good;
        return Reply::Do(String::new(), if only_the_suggestions { Action::FixAll } else { Action::Improve });
    }
    // "Fix the light" is Auto, which sets the light and nothing else.
    if has(&["fix", "repair", "correct", "sort"]) && has(&["light", "lighting", "exposure", "brightness", "tone"]) && !question {
        return Reply::Do("Auto: the light set from the photo.".to_owned(), Action::Auto);
    }
    // The other open photos: showing one, and doing something to all of them.
    if !question && let Some(reply) = requests::photos(trimmed, &said, scene) {
        return reply;
    }
    // "Brighten all the photos" names an edit, which is made on this photo before it is repeated.
    if !question && requests::everyone(&said) && scene.photo_names.len() > 1 && edits::one(trimmed, develop).is_some() {
        return Reply::Say("To change every open photo, make the change on this one first, then say “same for all the others”.".to_owned());
    }
    // The white balance set from the photo: the color-cast rule's own fix, when it finds one.
    // (Auto sets the light only, so "auto white balance" belongs here too.)
    let about_balance = has(&["cast", "wb"]) || (has(&["white"]) && has(&["balance"]));
    if about_balance && has(&["fix", "remove", "correct", "neutralize", "neutralise", "auto", "automatic", "set"]) && !question && edits::number(trimmed).is_none() {
        let cast = scene.suggestions.iter().find(|suggestion| suggestion.check == Check::ColorCast).and_then(|suggestion| suggestion.fix.clone());
        return match cast {
            Some(fix) => Reply::Do(format!("White balance set from the photo's grays: {}.", listed_changes(&fix)), Action::Adjust(Box::new(fix))),
            None => Reply::Say("I don't measure a color cast in this photo, so I've left the white balance alone. To set it yourself, press W and click something that should be neutral gray, or say “warmer” or “cooler”.".to_owned()),
        };
    }
    // "Fix the colors": whatever the color rules measured (a cast, colors too strong or too
    // dull), each with its own fix, as one change.
    if has(&["fix", "repair", "correct", "sort"]) && has(&["color", "colour"]) && !question {
        let mut now = develop.clone();
        let mut made: Vec<String> = Vec::new();
        let mut changes: Vec<(DevelopSetting, f32)> = Vec::new();
        for (title, fix) in scene.suggestions.iter().filter(|suggestion| suggestion.check.area() == Area::Color).filter_map(|suggestion| Some((&suggestion.title, suggestion.fix.as_ref()?))) {
            now = fix.apply_to(&now);
            made.push(format!("• {title}: {}", listed_changes(fix)));
            changes.extend(fix.changes.iter().map(|(setting, _)| (*setting, setting.get(&now))));
        }
        return if made.is_empty() {
            Reply::Say("I don't measure a color cast, or colors that are too strong or too dull, so I've left the colors alone. Tell me which way to take them: “warmer”, “cooler”, “more color”, “less color”.".to_owned())
        } else {
            Reply::Do(format!("Colors fixed from what I measured:\n{}", made.join("\n")), Action::Adjust(Box::new(Fix::settings("Fix Colors", changes))))
        };
    }
    if has(&["auto", "automatic", "autotone"]) && !question && !has(&["straighten", "level", "horizon", "crop"]) {
        return Reply::Do("Auto: the light set from the photo.".to_owned(), Action::Auto);
    }
    if !question && let Some(reply) = requests::export(trimmed, &said) {
        return reply;
    }
    if !question && let Some(reply) = requests::geometry(trimmed, &said) {
        return reply;
    }
    if let Some(reply) = requests::keeping(trimmed, &said, scene, question) {
        return reply;
    }
    // A reset of everything; one of a slider, a color or the crop is read as that instead, and
    // one of the window's own things (the zoom, the workspace) is a menu command.
    let resets = has(&["reset"]) || said.windows(2).any(|pair| pair[0] == "start" && pair[1] == "over");
    if resets && !question && edits::named_setting(&said).is_none() && !colors::mentions(trimmed) && !has(&["zoom", "view", "workspace", "layout", "panel", "window"]) {
        if has(&["all", "everything"]) || (has(&["crop"]) && has(&["mask"])) {
            return Reply::Do("Everything is back as it came: the settings, the crop and the masks. Undo brings them back.".to_owned(), Action::ResetAll);
        }
        return Reply::Do("Every Develop setting is back to zero (the crop and masks stay). Undo brings them back.".to_owned(), Action::Reset);
    }
    // Undo, turning and comparing.
    if !question && let Some(reply) = command_request(&said) {
        return reply;
    }
    const OPENING_SETTINGS: &[&str] = &["open", "show", "go", "to", "the", "app", "setting", "preference", "please", "me", "my"];
    if has(&["setting", "preference"]) && !question && said.iter().all(|word| OPENING_SETTINGS.contains(&word.as_str())) {
        return Reply::Do("Opening Settings.".to_owned(), Action::OpenSettings);
    }
    // Presets and looks by name.
    if has(&["preset"]) && has(&["list", "which", "what", "show", "all", "any"]) {
        return Reply::Say(preset_list(scene));
    }
    if has(&["look"]) && has(&["list", "which", "what", "all", "any"]) && question {
        return Reply::Say(look_list());
    }
    // A preset can't be scaled, so "a bit moody" asks for the look of that name instead.
    if !question && strength(&said) == 1.0 && let Some(name) = preset_request(trimmed, &said, scene) {
        return Reply::Do(format!("Applied {name}."), Action::ApplyPreset(name));
    }
    // A slider named with a number, or "reset" ("set exposure to 0.5", "shadows +30").
    if !question {
        let typed = edits::number(trimmed);
        if edits::named_setting(&said) == Some(DevelopSetting::Temperature) && typed.is_some_and(|number| number.kelvin || number.value.abs() >= 1000.0) {
            return Reply::Say("Temperature here isn't in kelvin: it runs from -100 (cooler, bluer) to +100 (warmer, more amber), with 0 as the photo came. Say “warmer”, “cooler”, or a value like “temperature 20”.".to_owned());
        }
        if let Some((text, fix)) = edits::exact(trimmed, &said, develop) {
            return Reply::Do(text, Action::Adjust(Box::new(fix)));
        }
    }
    // Questions about a problem are answered from what was measured…
    let area = if has(&["exposed", "overexposed", "underexposed", "bright", "dark", "exposure", "blown", "clipped", "clipping", "highlight", "shadow", "light"]) {
        Some(Area::Light)
    } else if has(&["contrast", "contrasty", "harsh", "flat", "washed"]) {
        Some(Area::Contrast)
    } else if has(&["color", "colour", "cast", "saturated", "saturation", "warm", "cool", "tint", "white", "balance", "vivid", "dull"]) {
        Some(Area::Color)
    } else if has(&["sharp", "sharpness", "halo", "clarity", "dehaze", "noise", "noisy", "detail"]) {
        Some(Area::Detail)
    } else {
        None
    };
    if question {
        let about_a_problem = has(&["wrong", "problem", "fix", "too", "clipped", "clipping", "clip", "blown", "bright", "dark", "overexposed", "underexposed", "histogram", "cast"]);
        if (has(&["white"]) && has(&["balance"]) || has(&["wb"])) && !has(&["wrong", "problem", "fix"]) {
            return Reply::Say(white_balance(scene));
        }
        // "What's the contrast at?", "how much vignette is there?"
        if has(&["what", "much", "where", "which"]) && !about_a_problem && let Some(setting) = edits::named_setting(&said) {
            return Reply::Say(value_of(setting, develop));
        }
        if has(&["photo", "image", "picture", "tab"]) && has(&["open"]) && has(&["how", "which", "what", "many"]) {
            let count = scene.photo_names.len();
            return Reply::Say(if count == 1 { format!("Just this one: {}.", scene.photo.name) } else { format!("{count} photos are open: {}. This is {}.", scene.photo_names.join(", "), scene.photo.name) });
        }
        if has(&["histogram", "bright", "dark", "exposure", "exposed", "overexposed", "underexposed", "clipped", "clipping", "blown"]) && !has(&["wrong", "problem", "fix"]) {
            let mut text = brightness(scene);
            if let Reply::Do(found, _) = problems(scene, Some(Area::Light)) {
                text.push_str(&format!("\n\n{found}"));
            }
            return Reply::Say(text);
        }
        if let Some(area) = area {
            return problems(scene, Some(area));
        }
    }
    // …while the same words said as a request make the edit.
    if let Some((text, fix)) = edits::worded(trimmed, &said, develop, question) {
        return Reply::Do(text, Action::Adjust(Box::new(fix)));
    }
    if has(&["wrong", "problem", "issue", "check", "review", "feedback", "suggestion", "suggest", "notice", "look", "better", "good"]) {
        return problems(scene, area);
    }
    if has(&["histogram"]) {
        return Reply::Say(brightness(scene));
    }
    if has(&["size", "resolution", "dimension", "megapixel", "pixel", "file", "info", "big", "crop", "cropped"]) {
        return Reply::Say(info(scene));
    }
    if has(&["changed", "change", "edit", "edits", "setting", "done"]) {
        return Reply::Say(edits(scene));
    }
    if has(&["tip", "hint", "trick", "advice", "teach", "learn"]) {
        return Reply::Do(String::new(), Action::Tip);
    }
    if has(&["ai", "robot", "chatgpt", "gpt", "llm", "human", "real"]) {
        return Reply::Say("No, I'm not an AI. I'm a set of plain rules that measure your photo, and a list of phrases I know how to answer. Everything I tell you comes from the photo itself.".to_owned());
    }
    if has(&["who", "name"]) && has(&["you", "your"]) {
        return Reply::Say(format!("I'm {}. I look over your photo for problems and help fix them. You can rename me with /name.", scene.name));
    }
    if has(&["help", "can", "what"]) && (has(&["you", "do", "help"])) {
        return Reply::Do(help(scene), Action::Help);
    }
    if has(&["thank", "thanks", "thx", "cheers", "great", "awesome", "nice", "perfect"]) {
        return Reply::Say("You're welcome!".to_owned());
    }
    if has(&["hello", "hi", "hey", "hiya", "howdy", "morning", "afternoon", "evening"]) {
        return Reply::Say(format!("Hello! Ask me what I notice in {}, or tell me what to change, like \"a bit brighter\".", scene.photo.name));
    }
    if has(&["bye", "goodbye", "later", "night"]) {
        return Reply::Say("Bye for now. I'll keep an eye on the photo.".to_owned());
    }
    Reply::Unknown
}

/// What Iris says when neither the photo nor her help answers a message.
pub const NOT_FOLLOWED: &str = "I didn't follow that. I can tell you what I notice (\"what's wrong?\"), describe the photo (\"how bright is it?\"), make edits (\"less contrast\", \"shadows +30\"), or explain how to do something (\"how do I crop?\"). Type /help for everything.";

/// Every word the readers here listen for, for putting a mistyped message right before it is
/// read a second time.
pub fn vocabulary() -> Vec<String> {
    let own = [
        "wrong", "problem", "histogram", "resolution", "megapixel", "changed", "brightness", "white", "balance", "before", "after", "split", "rotate", "mirror", "automatic", "reset", "everything", "preset", "snapshot", "export", "suggestion", "review",
        "overexposed", "underexposed", "clipping", "clipped", "photo", "picture", "image",
    ];
    let mut found: Vec<String> = edits::vocabulary().into_iter().chain(colors::vocabulary()).chain(looks::vocabulary()).chain(requests::vocabulary()).chain(own).map(str::to_owned).collect();
    found.sort_unstable();
    found.dedup();
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assistant::{Measure, Options, suggestions};
    use crate::bitmap::Bitmap;
    use crate::color::Rgba;
    use crate::image_io::ExportFormat;
    use crate::photo::{Snapshot, Source};

    fn photo() -> Photo {
        Photo::new(Source::new("Beach.jpg", Vec::new(), Bitmap::filled(40, 30, Rgba::rgb(0.9, 0.9, 0.9))))
    }

    /// Asks about `photo` as one of `names`' open photos (the first of them).
    fn ask_among(message: &str, photo: &Photo, names: &[&str]) -> Reply {
        let bright = Bitmap::filled(40, 30, Rgba::rgb(0.95, 0.95, 0.95));
        let measure = Measure::of(&bright);
        let found = suggestions(&measure, None, &photo.edit, &Options::default());
        let presets = vec!["Vivid".to_owned(), "High Contrast".to_owned(), "Teal & Orange".to_owned(), "Moody".to_owned()];
        let names: Vec<String> = names.iter().map(|name| (*name).to_owned()).collect();
        reply(message, &Scene { photo, measure: Some(&measure), suggestions: &found, name: "Iris", photo_names: &names, active: 0, presets: &presets, unsaved: true })
    }

    fn ask(message: &str, photo: &Photo) -> Reply {
        ask_among(message, photo, &["Beach"])
    }

    fn adjusted(reply: Reply) -> Fix {
        match reply {
            Reply::Do(_, Action::Adjust(fix)) => *fix,
            other => panic!("expected an edit, got {other:?}"),
        }
    }

    fn done(reply: Reply) -> Action {
        match reply {
            Reply::Do(_, action) => action,
            other => panic!("expected something to do, got {other:?}"),
        }
    }

    fn said(reply: Reply) -> String {
        match reply {
            Reply::Say(text) | Reply::Do(text, _) => text,
            other => panic!("expected words, got {other:?}"),
        }
    }

    #[test]
    fn questions_about_problems_are_answered_from_the_measurements() {
        let photo = photo();
        let Reply::Say(text) = ask("Is it overexposed?", &photo) else { panic!("a question gets words") };
        assert!(text.contains("bright overall"), "{text}");
        assert!(text.contains("overexposed"), "{text}");
        assert!(matches!(ask("what's wrong with it", &photo), Reply::Do(_, Action::ShowSuggestions)));
        assert!(matches!(ask("/check", &photo), Reply::Do(_, Action::ShowSuggestions)));
    }

    #[test]
    fn plain_words_make_edits_of_the_right_size_and_direction() {
        let photo = photo();
        let darker = adjusted(ask("it's too bright", &photo));
        assert_eq!(darker.changes[0], (DevelopSetting::Exposure, -0.3));
        assert_eq!(darker.changes[1].0, DevelopSetting::Highlights);
        assert_eq!(adjusted(ask("a bit brighter please", &photo)).changes, vec![(DevelopSetting::Exposure, 0.15)]);
        assert_eq!(adjusted(ask("much warmer", &photo)).changes, vec![(DevelopSetting::Temperature, 30.0)]);
        let softer = adjusted(ask("too much contrast", &photo));
        assert!(softer.changes.contains(&(DevelopSetting::Contrast, -20.0)), "{softer:?}");
        assert_eq!(adjusted(ask("make it black and white", &photo)).changes, vec![(DevelopSetting::Saturation, -100.0)]);
        assert_eq!(adjusted(ask("more contrast", &photo)).changes[0], (DevelopSetting::Contrast, 20.0));
        // Asked politely, a request is still a request.
        assert_eq!(adjusted(ask("Can you make it a bit brighter?", &photo)).changes, vec![(DevelopSetting::Exposure, 0.15)]);
        assert_eq!(adjusted(ask("brighter by half a stop", &photo)).changes, vec![(DevelopSetting::Exposure, 0.5)]);
        assert_eq!(adjusted(ask("two stops darker", &photo)).changes, vec![(DevelopSetting::Exposure, -2.0)]);
    }

    #[test]
    fn edits_build_on_the_current_settings_and_stay_in_range() {
        let mut photo = photo();
        photo.edit.develop.exposure = 4.9;
        assert_eq!(adjusted(ask("a lot brighter", &photo)).changes, vec![(DevelopSetting::Exposure, 5.0)]);
    }

    #[test]
    fn sliders_are_set_and_nudged_by_name() {
        let mut photo = photo();
        photo.edit.develop.shadows = 10.0;
        assert_eq!(adjusted(ask("set exposure to -0.5", &photo)).changes, vec![(DevelopSetting::Exposure, -0.5)]);
        assert_eq!(adjusted(ask("shadows +30", &photo)).changes, vec![(DevelopSetting::Shadows, 40.0)]);
        assert_eq!(adjusted(ask("bring the shadows down 5", &photo)).changes, vec![(DevelopSetting::Shadows, 5.0)]);
        assert_eq!(adjusted(ask("highlights -40", &photo)).changes, vec![(DevelopSetting::Highlights, -40.0)]);
        assert_eq!(adjusted(ask("luminance noise 30", &photo)).changes, vec![(DevelopSetting::NoiseLuminance, 30.0)]);
        assert_eq!(adjusted(ask("reset shadows", &photo)).changes, vec![(DevelopSetting::Shadows, 0.0)]);
        assert_eq!(adjusted(ask("more clarity", &photo)).changes, vec![(DevelopSetting::Clarity, 10.0)]);
        // Out of range is kept in range, and says so.
        let Reply::Do(text, Action::Adjust(fix)) = ask("contrast 500", &photo) else { panic!("an edit") };
        assert_eq!(fix.changes, vec![(DevelopSetting::Contrast, 100.0)]);
        assert!(text.contains("as far as it goes"), "{text}");
    }

    #[test]
    fn a_value_after_to_is_the_value_itself_whatever_its_sign() {
        let mut photo = photo();
        photo.edit.develop.exposure = 1.0;
        photo.edit.develop.shadows = 10.0;
        // "To -0.5" is where it should end up, not a step down from where it is.
        assert_eq!(adjusted(ask("set exposure to -0.5", &photo)).changes, vec![(DevelopSetting::Exposure, -0.5)]);
        assert_eq!(adjusted(ask("set exposure to +0.5", &photo)).changes, vec![(DevelopSetting::Exposure, 0.5)]);
        assert_eq!(adjusted(ask("take the shadows down to -20", &photo)).changes, vec![(DevelopSetting::Shadows, -20.0)]);
        // "By" and a bare sign are changes from where it is.
        assert_eq!(adjusted(ask("exposure up by 0.3", &photo)).changes, vec![(DevelopSetting::Exposure, 1.3)]);
        assert_eq!(adjusted(ask("add 10 to shadows", &photo)).changes, vec![(DevelopSetting::Shadows, 20.0)]);
        assert_eq!(adjusted(ask("exposure minus 1", &photo)).changes, vec![(DevelopSetting::Exposure, 0.0)]);
    }

    #[test]
    fn a_slider_named_with_a_direction_moves_that_slider_not_the_whole_photo() {
        let photo = photo();
        assert_eq!(adjusted(ask("brighten the shadows", &photo)).changes, vec![(DevelopSetting::Shadows, 10.0)]);
        assert_eq!(adjusted(ask("raise the shadows a lot", &photo)).changes, vec![(DevelopSetting::Shadows, 20.0)]);
        assert_eq!(adjusted(ask("recover the highlights", &photo)).changes, vec![(DevelopSetting::Highlights, -10.0)]);
        assert_eq!(adjusted(ask("the shadows are too dark", &photo)).changes, vec![(DevelopSetting::Shadows, 10.0)]);
        assert_eq!(adjusted(ask("crush the blacks a bit", &photo)).changes, vec![(DevelopSetting::Blacks, -5.0)]);
        // "Turn" with a slider is about the slider, not the photo.
        assert_eq!(adjusted(ask("turn the saturation down 10", &photo)).changes, vec![(DevelopSetting::Saturation, -10.0)]);
    }

    #[test]
    fn kelvin_is_explained_rather_than_guessed() {
        let photo = photo();
        for message in ["temperature 5600 K", "set the temperature to 5600k", "temperature 6500"] {
            let Reply::Say(text) = ask(message, &photo) else { panic!("{message} should be explained") };
            assert!(text.contains("isn't in kelvin"), "{text}");
        }
        assert_eq!(adjusted(ask("temperature 20", &photo)).changes, vec![(DevelopSetting::Temperature, 20.0)]);
    }

    #[test]
    fn several_edits_in_one_request_become_one_fix_that_says_each() {
        let mut photo = photo();
        photo.edit.develop.shadows = 10.0;
        let Reply::Do(text, Action::Adjust(fix)) = ask("shadows +30, highlights -20 and a bit warmer", &photo) else { panic!("one edit") };
        assert_eq!(fix.changes, vec![(DevelopSetting::Shadows, 40.0), (DevelopSetting::Highlights, -20.0), (DevelopSetting::Temperature, 8.0)]);
        assert!(text.starts_with("3 changes") && text.contains("Shadows 40") && text.contains("Highlights -20") && text.contains("Warmer"), "{text}");
        // A slider named alone takes the amount said for its neighbor, before or after.
        assert_eq!(adjusted(ask("shadows and highlights up 20", &photo)).changes, vec![(DevelopSetting::Shadows, 30.0), (DevelopSetting::Highlights, 20.0)]);
        assert_eq!(adjusted(ask("lower the contrast and the clarity", &photo)).changes, vec![(DevelopSetting::Contrast, -10.0), (DevelopSetting::Clarity, -10.0)]);
        // Two sliders each with a number are two edits, even with only a space between them.
        assert_eq!(adjusted(ask("exposure 0.5 contrast 20", &photo)).changes, vec![(DevelopSetting::Exposure, 0.5), (DevelopSetting::Contrast, 20.0)]);
        assert_eq!(adjusted(ask("luminance noise 30", &photo)).changes, vec![(DevelopSetting::NoiseLuminance, 30.0)]);
        // Later edits build on earlier ones in the same request.
        assert_eq!(adjusted(ask("exposure +0.5, exposure +0.25", &photo)).changes, vec![(DevelopSetting::Exposure, 0.75)]);
        // "Black and white" is one look, not two requests; "blacks and whites" are two sliders.
        assert_eq!(adjusted(ask("black and white, and add grain", &photo)).changes, vec![(DevelopSetting::Saturation, -100.0), (DevelopSetting::Grain, 25.0)]);
        assert_eq!(adjusted(ask("blacks and whites up 10", &photo)).changes, vec![(DevelopSetting::Blacks, 10.0), (DevelopSetting::Whites, 10.0)]);
        // The same thing said twice is done once.
        assert_eq!(adjusted(ask("it's too dark, make it brighter", &photo)).changes, vec![(DevelopSetting::Exposure, 0.3)]);
    }

    #[test]
    fn a_list_with_a_part_that_isnt_followed_changes_nothing_and_says_which() {
        let photo = photo();
        let Reply::Partly { followed, missed } = ask("shadows +30 and frobnicate the widget", &photo) else { panic!("partly followed") };
        assert_eq!((followed, missed), (vec!["shadows +30".to_owned()], vec!["frobnicate the widget".to_owned()]));
        // A name with an "and" in it is put in quotes for whatever splits requests at "and".
        assert_eq!(quote_name("sharpen and save a snapshot called Warm and Soft"), "sharpen and save a snapshot called “Warm and Soft”");
        assert_eq!(quote_name("save a snapshot called “Warm and Soft”"), "save a snapshot called “Warm and Soft”");
        assert_eq!(quote_name("warmer and softer"), "warmer and softer");
        // A question, a remark and a name with an "and" in it are each one thing.
        assert!(matches!(ask("is it too dark and too warm?", &photo), Reply::Say(_)));
        assert_eq!(ask("thanks, that's great", &photo), Reply::Say("You're welcome!".to_owned()));
        assert!(matches!(ask("save a snapshot called Warm and Brighter", &photo), Reply::Do(_, Action::Snapshot(Some(name))) if name == "Warm and Brighter"));
    }

    #[test]
    fn looks_are_nudges_scaled_by_how_they_are_asked_for() {
        let mut photo = photo();
        photo.edit.develop.exposure = 0.5;
        let Reply::Do(text, Action::Adjust(moody)) = ask("make it moody", &photo) else { panic!("a look") };
        assert!(moody.changes.contains(&(DevelopSetting::Exposure, 0.25)) && moody.changes.contains(&(DevelopSetting::Vignette, -20.0)), "{moody:?}");
        // The reply names every slider that moved.
        assert!(text.contains("Exposure +0.25") && text.contains("Vignette -20") && text.contains("Saturation -18"), "{text}");
        let gentle = adjusted(ask("a bit moody", &photo));
        assert!(gentle.changes.contains(&(DevelopSetting::Vignette, -10.0)), "{gentle:?}");
        let strong = adjusted(ask("really cinematic", &photo));
        assert!(strong.changes.contains(&(DevelopSetting::Grade(0, 0), 200.0)) && strong.changes.contains(&(DevelopSetting::Grade(0, 1), 44.0)), "{strong:?}");
        for (message, look) in [
            ("bright and airy", "Bright & Airy"),
            ("punchy look", "Punchy"),
            ("give it a soft look", "Soft"),
            ("matte", "Matte"),
            ("a faded look", "Matte"),
            ("vintage feel", "Vintage"),
            ("warm sunset", "Warm Sunset"),
            ("golden hour", "Warm Sunset"),
            ("make it cool", "Cool"),
            ("high-key", "High-Key"),
            ("low key", "Low-Key"),
            ("dramatic black and white", "Dramatic Black & White"),
        ] {
            assert_eq!(adjusted(ask(message, &photo)).label, format!("{look} Look"), "{message}");
        }
        // Less of a look moves the same sliders back.
        photo.edit.develop.vignette = -20.0;
        assert!(adjusted(ask("less moody", &photo)).changes.contains(&(DevelopSetting::Vignette, -10.0)));
        // Everyday words that happen to name a look mean what they always did.
        assert_eq!(adjusted(ask("too cool", &photo)).label, "Warmer");
        assert_eq!(adjusted(ask("too soft", &photo)).label, "Sharper");
        assert_eq!(adjusted(ask("softer", &photo)).label, "Less Contrast");
        assert_eq!(ask("cool", &photo), Reply::Unknown);
        // A preset's exact name is still the preset; a scaled one is the look.
        assert!(matches!(ask("try moody", &photo), Reply::Do(_, Action::ApplyPreset(name)) if name == "Moody"));
        assert!(matches!(ask("/looks", &photo), Reply::Say(text) if text.contains("Cinematic")));
    }

    #[test]
    fn one_color_is_changed_through_the_mixer() {
        let photo = photo();
        let Reply::Do(text, Action::Adjust(sky)) = ask("make the sky bluer", &photo) else { panic!("an edit") };
        assert_eq!(sky.changes, vec![(DevelopSetting::MixerSaturation(5), 20.0)]);
        assert!(text.contains("Blue Saturation +20"), "{text}");
        assert_eq!(adjusted(ask("deeper blues", &photo)).changes, vec![(DevelopSetting::MixerLuminance(5), -15.0), (DevelopSetting::MixerSaturation(5), 10.0)]);
        assert_eq!(adjusted(ask("greener grass", &photo)).changes, vec![(DevelopSetting::MixerSaturation(3), 20.0), (DevelopSetting::MixerSaturation(2), 20.0)]);
        assert_eq!(adjusted(ask("desaturate the oranges", &photo)).changes, vec![(DevelopSetting::MixerSaturation(1), -20.0)]);
        assert_eq!(adjusted(ask("desaturate the oranges by 35", &photo)).changes, vec![(DevelopSetting::MixerSaturation(1), -35.0)]);
        assert_eq!(adjusted(ask("darken the blues a bit", &photo)).changes, vec![(DevelopSetting::MixerLuminance(5), -10.0)]);
        assert_eq!(adjusted(ask("softer skin tones", &photo)).changes, vec![(DevelopSetting::MixerSaturation(1), -10.0), (DevelopSetting::MixerLuminance(1), 10.0)]);
        assert_eq!(adjusted(ask("blue saturation -20", &photo)).changes, vec![(DevelopSetting::MixerSaturation(5), -20.0)]);
        assert_eq!(adjusted(ask("set the orange luminance to 15", &photo)).changes, vec![(DevelopSetting::MixerLuminance(1), 15.0)]);
        assert_eq!(adjusted(ask("reset the blues", &photo)).changes.len(), 3);
        assert_eq!(adjusted(ask("reset the color mixer", &photo)).changes.len(), 24);
        // Darkening a sky this way only reaches what is blue, and the reply says so.
        assert!(said(ask("darken the sky", &photo)).contains("Linear Gradient"));
        // A color's bare name is about the white balance, and a slider of the whole photo stays one.
        assert_eq!(adjusted(ask("it's too blue", &photo)).label, "Warmer");
        assert_eq!(adjusted(ask("more contrast in the sky", &photo)).label, "More Contrast");
    }

    #[test]
    fn detail_and_finishing_are_asked_for_in_words() {
        let mut photo = photo();
        assert_eq!(adjusted(ask("sharpen it", &photo)).changes, vec![(DevelopSetting::SharpenAmount, 30.0)]);
        assert_eq!(adjusted(ask("sharpening 60", &photo)).changes, vec![(DevelopSetting::SharpenAmount, 60.0)]);
        assert_eq!(adjusted(ask("reduce the noise", &photo)).changes, vec![(DevelopSetting::NoiseLuminance, 25.0), (DevelopSetting::NoiseColor, 25.0)]);
        assert_eq!(adjusted(ask("noise reduction 40", &photo)).changes, vec![(DevelopSetting::NoiseLuminance, 40.0)]);
        assert_eq!(adjusted(ask("dehaze a bit", &photo)).changes, vec![(DevelopSetting::Dehaze, 10.0)]);
        assert_eq!(adjusted(ask("it's hazy", &photo)).changes, vec![(DevelopSetting::Dehaze, 20.0)]);
        assert_eq!(adjusted(ask("more texture", &photo)).changes, vec![(DevelopSetting::Texture, 10.0)]);
        // A vignette darkens the corners, which is a negative amount.
        assert_eq!(adjusted(ask("add a vignette", &photo)).changes, vec![(DevelopSetting::Vignette, -25.0)]);
        assert_eq!(adjusted(ask("add a subtle vignette", &photo)).changes, vec![(DevelopSetting::Vignette, -13.0)]);
        assert_eq!(adjusted(ask("darken the corners", &photo)).changes, vec![(DevelopSetting::Vignette, -25.0)]);
        assert!(said(ask("vignette 30", &photo)).contains("lighter corners"));
        assert_eq!(adjusted(ask("add some film grain", &photo)).changes, vec![(DevelopSetting::Grain, 25.0)]);
        // "Less grainy" lowers the grain that was added, or else smooths the photo's noise.
        assert_eq!(adjusted(ask("less grainy", &photo)).label, "Noise Reduction");
        photo.edit.develop.grain = 40.0;
        photo.edit.develop.vignette = -40.0;
        assert_eq!(adjusted(ask("less grainy", &photo)).changes, vec![(DevelopSetting::Grain, 15.0)]);
        assert_eq!(adjusted(ask("less vignette", &photo)).changes, vec![(DevelopSetting::Vignette, -25.0)]);
        assert_eq!(adjusted(ask("remove the vignette", &photo)).changes, vec![(DevelopSetting::Vignette, 0.0)]);
        assert_eq!(adjusted(ask("no grain", &photo)).changes, vec![(DevelopSetting::Grain, 0.0)]);
    }

    #[test]
    fn the_crop_is_shaped_straightened_and_reset_by_word() {
        let photo = photo();
        assert_eq!(done(ask("crop to 16:9", &photo)), Action::Crop { aspect: 7, upright: Some(false) });
        assert_eq!(done(ask("crop it 9x16", &photo)), Action::Crop { aspect: 7, upright: Some(true) });
        assert_eq!(done(ask("crop to 4 by 5", &photo)), Action::Crop { aspect: 3, upright: Some(true) });
        assert_eq!(done(ask("crop to 3:2", &photo)), Action::Crop { aspect: 5, upright: Some(false) });
        assert_eq!(done(ask("crop it square", &photo)), Action::Crop { aspect: 2, upright: None });
        assert_eq!(done(ask("crop to the original ratio", &photo)), Action::Crop { aspect: 1, upright: None });
        assert_eq!(done(ask("crop to 4:3, portrait", &photo)), Action::Crop { aspect: 6, upright: Some(true) });
        assert!(said(ask("crop to 21:9", &photo)).contains("can't hold the crop to 21:9"));
        assert_eq!(done(ask("reset the crop", &photo)), Action::ResetCrop);
        assert_eq!(done(ask("swap the crop", &photo)), Action::TurnCrop(None));
        assert_eq!(done(ask("straighten 1.5 degrees", &photo)), Action::Straighten(1.5));
        assert_eq!(done(ask("tilt it 2 degrees left", &photo)), Action::Straighten(-2.0));
        assert_eq!(done(ask("crop", &photo)), Action::OpenCrop);
        assert_eq!(done(ask("rotate 90", &photo)), Action::Rotate { clockwise: true, turns: 1 });
        assert_eq!(done(ask("rotate 180", &photo)), Action::Rotate { clockwise: true, turns: 2 });
        assert_eq!(done(ask("rotate 90 degrees left", &photo)), Action::Rotate { clockwise: false, turns: 1 });
        assert!(said(ask("rotate 60 degrees", &photo)).contains("quarter turns"));
    }

    #[test]
    fn the_other_open_photos_are_reached_by_word() {
        let photo = photo();
        let open = ["Beach", "Dunes", "Harbor"];
        assert_eq!(done(ask_among("next photo", &photo, &open)), Action::Go(Where::Next));
        assert_eq!(done(ask_among("go to the previous one", &photo, &open)), Action::Go(Where::Previous));
        assert_eq!(done(ask_among("photo 3", &photo, &open)), Action::Go(Where::Index(2)));
        assert_eq!(done(ask_among("switch to Harbor", &photo, &open)), Action::Go(Where::Index(2)));
        assert!(said(ask_among("go to photo 9", &photo, &open)).contains("no photo 9"));
        assert_eq!(done(ask_among("sync to the others", &photo, &open)), Action::Sync(SyncParts::EVERYTHING));
        assert_eq!(done(ask_among("apply this to all open photos", &photo, &open)), Action::Sync(SyncParts::EVERYTHING));
        assert_eq!(SyncParts::EVERYTHING.described(), "every Develop setting, the masks and the crop");
        let Action::Sync(parts) = done(ask_among("apply this to all photos, everything but the crop", &photo, &open)) else { panic!("a sync") };
        assert!(parts.develop.tone && parts.masks && !parts.crop);
        assert_eq!(parts.described(), "everything but Crop");
        let Action::Sync(parts) = done(ask_among("sync only the white balance", &photo, &open)) else { panic!("a sync") };
        assert!(parts.develop.white_balance && !parts.develop.tone && !parts.masks && !parts.crop);
        assert_eq!(parts.described(), "only White Balance");
        let Action::Sync(parts) = done(ask_among("sync everything but the masks and the tone curve", &photo, &open)) else { panic!("a sync") };
        assert!(parts.develop.tone && !parts.develop.tone_curve && !parts.masks && parts.crop);
        assert_eq!(parts.described(), "everything but Tone Curve, Masks");
        let Action::Sync(parts) = done(ask_among("sync only the crop", &photo, &open)) else { panic!("a sync") };
        assert!(parts.crop && !parts.masks && parts.develop == crate::presets::PresetParts::NONE);
        // A preset or an edit is made here first.
        assert!(said(ask_among("apply Vivid to all photos", &photo, &open)).contains("same for all the others"));
        assert!(said(ask_among("make all the photos brighter", &photo, &open)).contains("same for all the others"));
        assert_eq!(done(ask_among("auto all photos", &photo, &open)), Action::EachPhoto(Each::Auto));
        assert_eq!(done(ask_among("reset every photo", &photo, &open)), Action::EachPhoto(Each::Reset));
        // With one photo open there is nothing to do it to.
        assert!(said(ask("sync to the others", &photo)).contains("only photo open"));
        assert!(said(ask("next photo", &photo)).contains("only photo open"));
    }

    #[test]
    fn syncing_copies_only_the_parts_asked_for_and_carries_the_crop_in_proportion() {
        let sized = |width: u32, height: u32| Photo::new(Source::new("Photo.jpg", Vec::new(), Bitmap::new(width, height)));
        let mut from = sized(600, 400);
        from.edit.develop.temperature = 20.0;
        from.edit.develop.exposure = 1.0;
        from.edit.crop.angle = 3.0;
        from.edit.crop.rect = Some(crate::geom::Rect::new(150.0, 100.0, 300.0, 200.0));
        let mut to = sized(1200, 800);
        to.edit.develop.contrast = 30.0;
        let white_balance = SyncParts { develop: crate::presets::PresetParts { white_balance: true, ..crate::presets::PresetParts::NONE }, masks: false, crop: false };
        assert!(!white_balance.copy(&from, &mut to));
        assert_eq!((to.edit.develop.temperature, to.edit.develop.exposure, to.edit.develop.contrast, to.edit.crop.angle), (20.0, 0.0, 30.0, 0.0));
        // The whole edit: the settings as they are, the crop twice the size on a photo twice as big.
        assert!(!SyncParts::EVERYTHING.copy(&from, &mut to));
        assert_eq!((to.edit.develop.exposure, to.edit.develop.contrast, to.edit.crop.angle), (1.0, 0.0, 3.0));
        assert_eq!(to.edit.crop.rect, Some(crate::geom::Rect::new(300.0, 200.0, 600.0, 400.0)));
        // An upright photo gets the crop turned with it, and says so.
        let mut upright = sized(400, 600);
        assert!(SyncParts::EVERYTHING.copy(&from, &mut upright));
        let rect = upright.edit.crop.rect.unwrap();
        assert!(rect.height > rect.width, "{rect:?}");
    }

    #[test]
    fn an_export_is_asked_for_with_its_choices() {
        let photo = photo();
        let asked = |message: &str| match done(ask(message, &photo)) {
            Action::Export(asked) => asked,
            other => panic!("{message}: expected an export, got {other:?}"),
        };
        let wide = asked("export as JPEG at 80% quality, 2048 pixels wide");
        assert_eq!(wide, ExportAsk { formats: vec![ExportFormat::Jpeg], quality: Some(80), size: Some(SizeAsk::Wide(2048)), all: false, now: false });
        assert!(wide.is_direct(), "naming a format says enough to write the file");
        // A plain "export" opens the sheet, as the menu command does.
        assert_eq!(asked("export"), ExportAsk::default());
        assert!(!asked("export").is_direct() && !asked("export at 2048 px").is_direct());
        assert!(asked("export now").is_direct() && asked("export it straight away").is_direct());
        assert_eq!(asked("export as png at full size"), ExportAsk { formats: vec![ExportFormat::Png], size: Some(SizeAsk::Full), ..ExportAsk::default() });
        assert_eq!(asked("export jpeg quality 80 long edge 2048"), ExportAsk { formats: vec![ExportFormat::Jpeg], quality: Some(80), size: Some(SizeAsk::Long(2048)), ..ExportAsk::default() });
        assert_eq!(asked("export at 1080 px tall").size, Some(SizeAsk::Tall(1080)));
        assert_eq!(asked("save as a tiff").formats, vec![ExportFormat::Tiff]);
        // Several formats, and every open photo.
        assert_eq!(asked("export as JPEG and TIFF").formats, vec![ExportFormat::Jpeg, ExportFormat::Tiff]);
        assert!(asked("export all").all && asked("export every open photo as png").all && asked("export all").is_direct());
        // Asking how is a question for the help, not an export.
        assert!(!matches!(ask("how do I export?", &photo), Reply::Do(_, Action::Export(_))));
    }

    #[test]
    fn a_folder_of_photos_is_read_with_what_to_do_to_each() {
        let photo = photo();
        let job = |message: &str| match done(ask(message, &photo)) {
            Action::Folder(job) => job,
            other => panic!("{message}: expected a folder batch, got {other:?}"),
        };
        let finished = job("apply portrait finish to every photo in this folder and export");
        assert_eq!((finished.place, finished.requests), (FolderPlace::This, vec!["portrait finish".to_owned()]));
        assert!(finished.export.is_some_and(|export| export.now && export.formats.is_empty()));
        let trip = job("auto every photo in ~/Pictures/Trip");
        assert_eq!((trip.place, trip.requests, trip.export), (FolderPlace::Named("~/Pictures/Trip".to_owned()), vec!["auto".to_owned()], None));
        let jpegs = job("export everything in this folder as JPEG");
        assert!(jpegs.requests.is_empty() && jpegs.export.is_some_and(|export| export.formats == vec![ExportFormat::Jpeg]));
        let quoted = job("make it black and white and warmer for all the photos in “/Volumes/Card One/DCIM”");
        assert_eq!(quoted.place, FolderPlace::Named("/Volumes/Card One/DCIM".to_owned()));
        assert_eq!(quoted.requests, vec!["make it black and white".to_owned(), "warmer".to_owned()]);
        // Without a folder named, "all" is still the open photos.
        assert!(matches!(done(ask("export all", &photo)), Action::Export(_)));
    }

    #[test]
    fn a_photo_is_sent_to_the_other_apps_by_name() {
        let photo = photo();
        for (message, to) in [("open this photo in Edit", OtherApp::Edit), ("send this to Omina Edit", OtherApp::Edit), ("put this photo in Motion", OtherApp::Motion), ("send it to Omina Motion", OtherApp::Motion)] {
            assert_eq!(done(ask(message, &photo)), Action::SendTo(to), "{message}");
        }
        // Edits that only mention motion or editing are what they always were.
        assert!(!matches!(ask("edit the photo", &photo), Reply::Do(_, Action::SendTo(_))));
        assert!(!matches!(ask("how do I send this to Motion?", &photo), Reply::Do(_, Action::SendTo(_))));
    }

    #[test]
    fn vague_requests_get_a_stated_default_or_a_question() {
        let mut photo = photo();
        // Each of these has one meaning, and the reply says what it came to.
        let Reply::Do(text, Action::Adjust(pop)) = ask("make it pop", &photo) else { panic!("an edit") };
        assert_eq!(pop.changes, vec![(DevelopSetting::Contrast, 15.0), (DevelopSetting::Clarity, 10.0), (DevelopSetting::Vibrance, 15.0)]);
        assert!(text.contains("which I take as") && text.contains("Contrast +15, Clarity +10, Vibrance +15"), "{text}");
        assert_eq!(adjusted(ask("more dramatic", &photo)).changes, vec![(DevelopSetting::Contrast, 20.0), (DevelopSetting::Clarity, 12.0), (DevelopSetting::Highlights, -10.0), (DevelopSetting::Blacks, -10.0)]);
        assert_eq!(adjusted(ask("flatter", &photo)).changes, vec![(DevelopSetting::Contrast, -20.0), (DevelopSetting::Clarity, -8.0)]);
        assert!(said(ask("softer", &photo)).starts_with("Softer: Contrast -20, Highlights -15, Shadows +15"));
        assert_eq!(adjusted(ask("clean it up", &photo)).changes, vec![(DevelopSetting::NoiseLuminance, 25.0), (DevelopSetting::NoiseColor, 25.0), (DevelopSetting::SharpenAmount, 20.0)]);
        // Complaints lead to the fix for what they complain of.
        for (complaint, label) in [("it's too dark", "Brighter"), ("too yellow", "Cooler"), ("it's too flat", "More Contrast"), ("too noisy", "Noise Reduction"), ("it's too harsh", "Less Contrast")] {
            assert_eq!(adjusted(ask(complaint, &photo)).label, label, "{complaint}");
        }
        // "More subtle" takes the whole edit part of the way back, and says so when there is none.
        assert!(said(ask("more subtle", &photo)).starts_with("No slider has been moved yet"));
        photo.edit.develop.contrast = 40.0;
        photo.edit.develop.vignette = -20.0;
        let subtle = adjusted(ask("more subtle", &photo));
        assert_eq!(subtle.changes, vec![(DevelopSetting::Contrast, 30.0), (DevelopSetting::Vignette, -15.0)]);
        // Making it look good, fixing the light and fixing the colors are told apart.
        assert_eq!(done(ask("make it look good", &photo)), Action::Improve);
        assert_eq!(done(ask("improve this", &photo)), Action::Improve);
        assert_eq!(done(ask("fix it", &photo)), Action::Improve);
        assert_eq!(done(ask("fix them all", &photo)), Action::FixAll);
        assert_eq!(done(ask("fix the light", &photo)), Action::Auto);
        // The test's measurements find dull colors, whose fix is more Vibrance.
        let Reply::Do(text, Action::Adjust(colors)) = ask("fix the colors", &photo) else { panic!("an edit") };
        assert_eq!(colors.changes, vec![(DevelopSetting::Vibrance, 30.0)]);
        assert!(text.contains("Colors look dull"), "{text}");
        // The crop tightens by a stated share, and levelling measures the photo.
        assert_eq!(done(ask("crop it tighter", &photo)), Action::TightenCrop(0.1));
        assert_eq!(done(ask("crop a bit off the edges", &photo)), Action::TightenCrop(0.05));
        assert_eq!(done(ask("crop in by 20%", &photo)), Action::TightenCrop(0.2));
        assert_eq!(done(ask("crop it a bit looser", &photo)), Action::TightenCrop(-0.05));
        for message in ["straighten it", "level the horizon", "auto straighten", "level"] {
            assert_eq!(done(ask(message, &photo)), Action::Level, "{message}");
        }
        // Not saying what to change gets a question with choices, never a guess.
        for message in ["change it", "do something", "edit the photo", "make it different"] {
            let Reply::Do(text, Action::Offer(offers)) = ask(message, &photo) else { panic!("{message} should be asked about") };
            assert!(text.starts_with("What would you like changed?") && offers.contains(&"Make it look good".to_owned()), "{message}");
        }
        assert!(matches!(ask("edits", &photo), Reply::Say(text) if text.starts_with("So far")));
    }

    #[test]
    fn snapshots_presets_and_copied_settings_are_kept_by_word() {
        let mut photo = photo();
        assert_eq!(done(ask("save a snapshot", &photo)), Action::Snapshot(None));
        assert_eq!(done(ask("save a snapshot called Before crop", &photo)), Action::Snapshot(Some("Before crop".to_owned())));
        assert_eq!(done(ask("snapshot “Warm version”", &photo)), Action::Snapshot(Some("Warm version".to_owned())));
        assert!(said(ask("go back to the snapshot Before crop", &photo)).contains("no snapshots yet"));
        photo.snapshots.push(Snapshot { name: "Before crop".to_owned(), edit: photo.edit.clone() });
        photo.snapshots.push(Snapshot { name: "Warm".to_owned(), edit: photo.edit.clone() });
        assert_eq!(done(ask("go back to the snapshot Before crop", &photo)), Action::RestoreSnapshot(0));
        assert_eq!(done(ask("restore snapshot 2", &photo)), Action::RestoreSnapshot(1));
        assert_eq!(done(ask("restore the last snapshot", &photo)), Action::RestoreSnapshot(1));
        assert!(said(ask("which snapshots are there?", &photo)).contains("“Before crop”, “Warm”"));
        // What differs from a snapshot is listed slider by slider.
        photo.edit.develop.exposure = 0.5;
        photo.edit.crop.angle = 2.0;
        let different = said(ask("what's different from the last snapshot?", &photo));
        assert!(different.contains("Exposure: 0.00 → 0.50") && different.contains("The crop changed"), "{different}");
        photo.snapshots[1].edit = photo.edit.clone();
        assert!(said(ask("what's different from the snapshot Warm?", &photo)).starts_with("Nothing"));
        assert_eq!(done(ask("save this as a preset called Golden", &photo)), Action::SavePreset(Some("Golden".to_owned())));
        assert_eq!(done(ask("make a new preset", &photo)), Action::SavePreset(None));
        assert_eq!(done(ask("copy these settings", &photo)), Action::CopySettings);
        assert_eq!(done(ask("paste settings", &photo)), Action::PasteSettings { look: true, crop: false });
        assert_eq!(done(ask("paste settings with the crop", &photo)), Action::PasteSettings { look: true, crop: true });
        assert_eq!(done(ask("paste only the crop", &photo)), Action::PasteSettings { look: false, crop: true });
    }

    #[test]
    fn the_ready_check_gives_one_verdict_from_what_was_measured() {
        let photo = photo();
        // The test's measurements are of a nearly white photo: overexposed.
        let Reply::Do(text, Action::Offer(offers)) = ask("is this ready?", &photo) else { panic!("a verdict") };
        assert!(text.contains("Light:") && text.contains("Frame: Not cropped: 40 × 30 pixels") && text.contains("aren't saved yet"), "{text}");
        assert!(text.contains("overexposed") && text.contains("Verdict: n"), "{text}");
        assert_eq!(offers, vec!["Fix it".to_owned(), "Export".to_owned(), "Save".to_owned()]);
        assert!(matches!(ask("/ready", &photo), Reply::Do(_, Action::Offer(_))));
        // With nothing noticed and nothing unsaved, it says so plainly.
        let measure = Measure::of(&Bitmap::filled(40, 30, Rgba::rgb(0.5, 0.45, 0.4)));
        let names = vec!["Beach".to_owned()];
        let clean = Scene { photo: &photo, measure: Some(&measure), suggestions: &[], name: "Iris", photo_names: &names, active: 0, presets: &[], unsaved: false };
        let Reply::Do(text, Action::Offer(offers)) = reply("ready to export?", &clean) else { panic!("a verdict") };
        assert!(text.contains("Verdict: ready to export") && text.contains("File: Saved."), "{text}");
        assert_eq!(offers, vec!["Export".to_owned()]);
    }

    #[test]
    fn questions_are_answered_with_the_photos_own_numbers() {
        let mut photo = photo();
        photo.edit.develop.temperature = 15.0;
        photo.edit.develop.contrast = 20.0;
        photo.edit.develop.mixer_saturation[5] = 20.0;
        let balance = said(ask("what's the white balance?", &photo));
        assert!(balance.contains("Temperature is at +15 and Tint at +0"), "{balance}");
        assert!(said(ask("what is the contrast at?", &photo)).starts_with("Contrast is at 20."));
        let changed = said(ask("what did I change?", &photo));
        assert!(changed.contains("Temperature +15") && changed.contains("Contrast +20") && changed.contains("Blue Saturation +20"), "{changed}");
        // Clipping is a measurement, not a slider's value.
        assert!(said(ask("how much are the highlights clipping?", &photo)).contains("clipped to white"));
        assert!(said(ask_among("which photos are open?", &photo, &["Beach", "Dunes"])).contains("2 photos are open: Beach, Dunes"));
    }

    #[test]
    fn presets_undo_turning_and_comparing_are_understood() {
        let photo = photo();
        assert!(matches!(ask("try vivid", &photo), Reply::Do(_, Action::ApplyPreset(name)) if name == "Vivid"));
        assert!(matches!(ask("apply the teal & orange preset", &photo), Reply::Do(_, Action::ApplyPreset(name)) if name == "Teal & Orange"));
        // Asking about contrast isn't asking for the High Contrast preset.
        assert!(!matches!(ask("is the contrast too high?", &photo), Reply::Do(_, Action::ApplyPreset(_))));
        assert!(matches!(ask("undo", &photo), Reply::Do(_, Action::Undo)));
        assert!(matches!(ask("rotate left", &photo), Reply::Do(_, Action::Rotate { clockwise: false, turns: 1 })));
        assert!(matches!(ask("turn it upside down", &photo), Reply::Do(_, Action::Rotate { turns: 2, .. })));
        assert!(matches!(ask("flip it vertically", &photo), Reply::Do(_, Action::Flip { horizontal: false })));
        assert!(matches!(ask("show me the before", &photo), Reply::Do(_, Action::Compare(Some(false)))));
        assert!(matches!(ask("compare with the original", &photo), Reply::Do(_, Action::Compare(Some(false)))));
        assert!(matches!(ask("side by side", &photo), Reply::Do(_, Action::Compare(Some(true)))));
        assert!(matches!(ask("show after", &photo), Reply::Do(_, Action::Compare(None))));
        assert!(matches!(ask("save a snapshot", &photo), Reply::Do(_, Action::Snapshot(None))));
        let Reply::Say(list) = ask("/presets", &photo) else { panic!("words") };
        assert!(list.contains("Teal & Orange"));
    }

    #[test]
    fn words_that_only_sound_like_a_request_are_left_alone() {
        let photo = photo();
        // A remark about how it looks, a bare topic and a slip of the tongue change nothing.
        for message in ["it looks soft", "noise", "grain", "vignette", "cool it", "first", "red eye"] {
            assert!(!matches!(ask(message, &photo), Reply::Do(_, Action::Adjust(_) | Action::Go(_))), "{message}");
        }
        // Snapshots are only deleted by their own button, never by a message that names one.
        assert!(said(ask("delete the snapshot Warm", &photo)).contains("its buttons"));
        // Auto sets the light; the white balance is set from a cast that was measured, or not at all.
        assert!(said(ask("auto white balance", &photo)).contains("don't measure a color cast"));
        assert_eq!(adjusted(ask("less saturated", &photo)).label, "Less Color");
        assert_eq!(adjusted(ask("cool it down", &photo)).label, "Cooler");
        assert!(matches!(ask("compare", &photo), Reply::Do(_, Action::Compare(Some(true)))));
        assert_eq!(done(ask_among("last photo", &photo, &["Beach", "Dunes", "Harbor"])), Action::Go(Where::Index(2)));
        // None of these is about turning, comparing or resetting the photo.
        for message in ["hide clipping", "zoom in", "the left side", "reset the workspace", "open", "save", "white balance selector", "frobnicate"] {
            assert_eq!(ask(message, &photo), Reply::Unknown, "{message}");
        }
        assert!(matches!(ask("open settings", &photo), Reply::Do(_, Action::OpenSettings)));
        assert!(matches!(ask("reset", &photo), Reply::Do(_, Action::Reset)));
        assert!(matches!(ask("reset everything", &photo), Reply::Do(_, Action::ResetAll)));
        assert!(matches!(ask("fix all photos", &photo), Reply::Do(_, Action::FixAll)), "with one photo open, all of them is this one");
        assert!(said(ask_among("fix all photos", &photo, &["Beach", "Dunes"])).contains("one photo at a time"));
    }

    #[test]
    fn commands_and_small_talk_answer() {
        let photo = photo();
        assert!(matches!(ask("/fix", &photo), Reply::Do(_, Action::FixAll)));
        assert!(matches!(ask("fix everything", &photo), Reply::Do(_, Action::FixAll)));
        assert!(matches!(ask("/name Pixel", &photo), Reply::Do(_, Action::Rename(name)) if name == "Pixel"));
        assert!(matches!(ask("/nothing", &photo), Reply::Say(text) if text.contains("/help")));
        assert!(matches!(ask("/help", &photo), Reply::Do(text, Action::Help) if text.contains("/ready") && text.contains("/recipes")));
        assert!(matches!(ask("what can you do?", &photo), Reply::Do(_, Action::Help)));
        assert!(matches!(ask("/forget tidy", &photo), Reply::Say(text) if text.contains("“tidy”")));
        let Reply::Say(info) = ask("/info", &photo) else { panic!("info is words") };
        assert!(info.contains("40 × 30"), "{info}");
        let Reply::Say(none) = ask("/edits", &photo) else { panic!("edits are words") };
        assert!(none.contains("as it came"));
        let Reply::Say(ai) = ask("are you an AI?", &photo) else { panic!("words") };
        assert!(ai.contains("not an AI"));
    }

    #[test]
    fn the_overexposed_rule_is_what_the_answers_rest_on() {
        let bright = Measure::of(&Bitmap::filled(40, 30, Rgba::rgb(1.0, 1.0, 1.0)));
        // A field of pure white is overexposed and blown: the light area answers with both.
        let found = suggestions(&bright, None, &crate::edit::Edit::default(), &Options::default());
        assert!(found.iter().any(|found| found.check == Check::BlownHighlights), "{found:?}");
        assert!(found.iter().any(|found| found.check == Check::Overexposed), "{found:?}");
    }

    #[test]
    fn the_vocabulary_holds_the_words_the_readers_listen_for() {
        let words = vocabulary();
        for word in ["brighter", "vignette", "moody", "blues", "straighten", "snapshot", "export"] {
            assert!(words.iter().any(|known| known == word), "{word}");
        }
    }
}

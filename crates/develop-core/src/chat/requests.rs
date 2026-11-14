//! Requests that are not slider edits: the crop's shape and angle, exporting, moving between
//! the open photos and doing the same thing to all of them, snapshots, presets, and copying
//! and pasting settings. Each reader here only says what was asked for (an `Action`); the app
//! carries it out, since only it knows the open photos, the files and the switches in Settings.

use super::{Action, Reply, Scene, edits, raw_words};
use crate::crop::{ASPECTS, MAX_ANGLE};
use crate::image_io::ExportFormat;
use crate::photo::Photo;
use crate::presets::{Preset, PresetParts};

/// Another of the Omina apps a photo can be sent to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OtherApp {
    /// Omina Edit, for design and layout.
    Edit,
    /// Omina Motion, for video and motion.
    Motion,
}

impl OtherApp {
    /// The app's name as people see it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Edit => "Omina Edit",
            Self::Motion => "Omina Motion",
        }
    }
}

/// Which folder a folder batch works on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FolderPlace {
    /// The folder the photo showing came from.
    This,
    /// A folder named in the message, as typed ("~" not yet expanded).
    Named(String),
}

/// Something to do to every photo in a folder: the requests to make on each (a recipe, a look,
/// a preset, an edit, as typed), then an export if one was asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FolderJob {
    pub place: FolderPlace,
    pub requests: Vec<String>,
    pub export: Option<ExportAsk>,
}

/// Which open photo to show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Where {
    Next,
    Previous,
    /// The photo at this place among the tabs.
    Index(usize),
}

/// Something done to every open photo, each from its own pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Each {
    /// Lightroom's Auto.
    Auto,
    /// Every Develop setting back to its default.
    Reset,
}

/// Which parts of a photo's edit are copied to the others.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SyncParts {
    /// The groups of Develop settings, as a preset divides them.
    pub develop: PresetParts,
    pub masks: bool,
    /// The crop and straightening. A crop is kept in each photo's own pixels, so it is carried
    /// across in proportion (see `Crop::carried_to`), not copied as it stands.
    pub crop: bool,
}

impl SyncParts {
    /// The whole edit: every Develop setting, the masks and the crop.
    pub const EVERYTHING: SyncParts = SyncParts { develop: PresetParts { white_balance: true, tone: true, presence: true, tone_curve: true, color_mixer: true, color_grading: true, detail: true, effects: true }, masks: true, crop: true };
    const NOTHING: SyncParts = SyncParts { develop: PresetParts::NONE, masks: false, crop: false };

    /// The names of the parts copied, in the Develop panel's order.
    fn names(&self) -> Vec<&'static str> {
        let mut parts = self.develop;
        let mut names: Vec<&'static str> = PresetParts::named().into_iter().filter(|(_, flag)| *flag(&mut parts)).map(|(name, _)| name).collect();
        if self.masks {
            names.push("Masks");
        }
        if self.crop {
            names.push("Crop");
        }
        names
    }

    /// What is copied, in words: "every Develop setting, the masks and the crop", "only White
    /// Balance".
    pub fn described(&self) -> String {
        if *self == Self::EVERYTHING {
            return "every Develop setting, the masks and the crop".to_owned();
        }
        let copied = self.names();
        let left_out: Vec<&str> = Self::EVERYTHING.names().into_iter().filter(|name| !copied.contains(name)).collect();
        if left_out.len() < copied.len() { format!("everything but {}", left_out.join(", ")) } else { format!("only {}", copied.join(", ")) }
    }

    /// Copies these parts of `from`'s edit onto `to`, leaving the rest of `to` alone. Returns
    /// whether the crop had to be turned a quarter because `to` stands the other way.
    pub fn copy(&self, from: &Photo, to: &mut Photo) -> bool {
        to.edit.develop = Preset::new("", "", from.edit.develop.clone(), self.develop).apply_to(&to.edit.develop);
        if self.masks {
            to.edit.masks = from.edit.masks.clone();
        }
        if !self.crop {
            return false;
        }
        let (crop, turned) = from.edit.crop.carried_to(&to.edit.crop, from.image_size(), to.image_size());
        to.edit.crop = crop;
        turned
    }
}

/// How big an export is asked to be.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SizeAsk {
    /// The finished photo's full size.
    Full,
    /// This many pixels on its longer side.
    Long(u32),
    /// This many pixels wide, or tall.
    Wide(u32),
    Tall(u32),
}

/// The choices an export request names; anything it leaves out stays as Export last had it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExportAsk {
    /// The formats named, in the order said ("as JPEG and TIFF"). None named means the format
    /// Export last used.
    pub formats: Vec<ExportFormat>,
    /// JPEG's quality, 1–100.
    pub quality: Option<u8>,
    pub size: Option<SizeAsk>,
    /// Whether every open photo was asked for, rather than the one showing.
    pub all: bool,
    /// Whether it was asked for at once ("export now"), without the sheet.
    pub now: bool,
}

impl ExportAsk {
    /// Whether the request says enough to write the file without asking anything more: a
    /// format, every photo, or "now". A plain "export" opens the sheet, as the menu does.
    pub fn is_direct(&self) -> bool {
        !self.formats.is_empty() || self.all || self.now
    }
}

/// Whether a request is about every open photo: "all photos", "the others", "the rest".
pub(super) fn everyone(said: &[String]) -> bool {
    let has = |list: &[&str]| said.iter().any(|word| list.contains(&word.as_str()));
    has(&["other", "rest"]) || (has(&["all", "every", "each"]) && has(&["photo", "image", "picture", "tab", "open", "them", "one"]))
}

/// Reaches one part's flag in a `SyncParts`.
type PartFlag = fn(&mut SyncParts) -> &mut bool;

/// The groups a sync request names, with the words that name each.
const SYNC_GROUPS: &[(&[&str], PartFlag)] = &[
    (&["wb", "balance", "temperature", "tint"], |parts| &mut parts.develop.white_balance),
    (&["tone", "exposure"], |parts| &mut parts.develop.tone),
    (&["presence", "clarity", "texture", "vibrance", "saturation"], |parts| &mut parts.develop.presence),
    (&["curve"], |parts| &mut parts.develop.tone_curve),
    (&["mixer", "hsl"], |parts| &mut parts.develop.color_mixer),
    (&["grading", "grade"], |parts| &mut parts.develop.color_grading),
    (&["detail", "sharpening", "noise"], |parts| &mut parts.develop.detail),
    (&["effect", "vignette", "grain"], |parts| &mut parts.develop.effects),
    (&["mask"], |parts| &mut parts.masks),
    (&["crop", "cropping", "framing", "straightening"], |parts| &mut parts.crop),
];

/// Which parts a sync request asks for: all of them, "only" the ones it names, or everything
/// "but" them.
fn sync_parts(said: &[String]) -> SyncParts {
    // "The tone curve" is the curve alone, not the tone sliders as well.
    let tone_curve = said.windows(2).any(|pair| pair[0] == "tone" && pair[1] == "curve");
    let words: Vec<&str> = said.iter().map(String::as_str).filter(|word| !(tone_curve && *word == "tone")).collect();
    let has = |list: &[&str]| words.iter().any(|word| list.contains(word));
    let named: Vec<PartFlag> = SYNC_GROUPS.iter().filter(|(names, _)| has(names)).map(|(_, flag)| *flag).collect();
    if named.is_empty() {
        return SyncParts::EVERYTHING;
    }
    let all_but = has(&["but", "except", "without", "excluding", "not"]);
    let mut parts = if all_but { SyncParts::EVERYTHING } else { SyncParts::NOTHING };
    for flag in named {
        *flag(&mut parts) = !all_but;
    }
    parts
}

/// The open photo a message names, by the longest name written in it.
fn named_photo(message: &str, scene: &Scene) -> Option<usize> {
    let lower = message.to_lowercase();
    scene.photo_names.iter().enumerate().filter(|(_, name)| name.chars().count() >= 3 && lower.contains(&name.to_lowercase())).max_by_key(|(_, name)| name.len()).map(|(index, _)| index)
}

/// Requests about the open photos: showing another one, copying this one's settings to the
/// others, and Auto or a reset for all of them.
pub(super) fn photos(message: &str, said: &[String], scene: &Scene) -> Option<Reply> {
    let has = |list: &[&str]| said.iter().any(|word| list.contains(&word.as_str()));
    let only = |list: &[&str]| said.iter().all(|word| list.contains(&word.as_str()));
    let count = scene.photo_names.len();
    let alone = |what: &str| Reply::Say(format!("This is the only photo open, so there is {what}. Open more with ⌘O and each gets a tab of its own."));
    let everyone = everyone(said);

    if everyone && has(&["auto", "automatic", "autotone"]) {
        return Some(if count < 2 { alone("no other to set") } else { Reply::Do(String::new(), Action::EachPhoto(Each::Auto)) });
    }
    if everyone && has(&["reset"]) {
        return Some(if count < 2 { alone("no other to reset") } else { Reply::Do(String::new(), Action::EachPhoto(Each::Reset)) });
    }
    let copies = has(&["paste"]) || (has(&["apply", "copy", "give", "put", "match", "same", "use"]) && has(&["this", "these", "setting", "look", "edit", "adjustment", "same", "it"]));
    if has(&["sync", "synchronize", "synchronise"]) || (everyone && copies) {
        // A preset's name would be lost here: it is applied to this photo first.
        if scene.presets.iter().any(|name| message.to_lowercase().contains(&name.to_lowercase())) {
            return Some(Reply::Say("One step at a time: apply the preset to this photo (“try Vivid”), then say “same for all the others”.".to_owned()));
        }
        return Some(if count < 2 { alone("nothing to sync to") } else { Reply::Do(String::new(), Action::Sync(sync_parts(said))) });
    }
    // Showing another photo.
    const MOVING: &[&str] = &["go", "to", "the", "show", "me", "switch", "move", "on", "photo", "image", "picture", "one", "tab", "please", "jump", "open", "select", "number"];
    let step = if has(&["next"]) && only(&[MOVING, &["next"]].concat()) {
        Some(Where::Next)
    } else if has(&["previous", "prev"]) && only(&[MOVING, &["previous", "prev"]].concat()) {
        Some(Where::Previous)
    } else if has(&["first"]) && has(&["photo", "image", "picture", "tab"]) && only(&[MOVING, &["first"]].concat()) {
        Some(Where::Index(0))
    } else if has(&["last"]) && has(&["photo", "image", "picture", "tab"]) && only(&[MOVING, &["last"]].concat()) {
        Some(Where::Index(count.saturating_sub(1)))
    } else {
        None
    };
    if let Some(step) = step {
        return Some(if count < 2 { alone("no other to show") } else { Reply::Do(String::new(), Action::Go(step)) });
    }
    // "Photo 3", "go to photo 3".
    if has(&["photo", "image", "picture", "tab"]) && said.iter().all(|word| MOVING.contains(&word.as_str()) || word.parse::<usize>().is_ok()) {
        let number = said.iter().find_map(|word| word.parse::<usize>().ok())?;
        return Some(if (1..=count).contains(&number) { Reply::Do(String::new(), Action::Go(Where::Index(number - 1))) } else { Reply::Say(format!("There {} open, so there is no photo {number}.", counted_photos(count))) });
    }
    if has(&["go", "switch", "jump", "show", "select"]) && !has(&["before", "after", "original", "snapshot", "preset"]) {
        let index = named_photo(message, scene)?;
        return Some(if index == scene.active { Reply::Say(format!("{} is the photo showing.", scene.photo_names[index])) } else { Reply::Do(String::new(), Action::Go(Where::Index(index))) });
    }
    None
}

/// "1 photo is" or "3 photos are".
fn counted_photos(count: usize) -> String {
    if count == 1 { "is 1 photo".to_owned() } else { format!("are {count} photos") }
}

/// A crop shape written in a message.
enum Shape {
    /// One of `ASPECTS`, and whether it was asked for upright (taller than wide).
    Listed(usize, Option<bool>),
    /// A shape the crop can't be held to, as it was written.
    Other(String),
}

/// The ratios in `ASPECTS` as their two numbers, smaller first.
const RATIOS: [((u32, u32), usize); 6] = [((1, 1), 2), ((4, 5), 3), ((5, 7), 4), ((2, 3), 5), ((3, 4), 6), ((9, 16), 7)];

/// The crop shape a message names: "16:9", "4x5", "4 by 5", "square", "the original ratio".
/// The first number is the width, so "4:5" is upright and "5:4" lies on its side.
fn shape(message: &str, raw: &[String]) -> Option<Shape> {
    let said = |list: &[&str]| raw.iter().any(|word| list.contains(&word.as_str()));
    let turned = if said(&["portrait", "vertical", "upright", "tall"]) {
        Some(true)
    } else if said(&["landscape", "horizontal", "wide", "widescreen"]) {
        Some(false)
    } else {
        None
    };
    if said(&["square"]) {
        return Some(Shape::Listed(2, None));
    }
    if said(&["original"]) && said(&["ratio", "aspect", "shape", "proportion", "proportions"]) {
        return Some(Shape::Listed(1, turned));
    }
    // "16:9", "16x9", "16×9", "16/9" in one word, or "16 by 9" in three.
    let lower = message.to_lowercase().replace('×', "x");
    let tokens: Vec<&str> = lower.split(|character: char| character.is_whitespace() || matches!(character, ',' | '?' | '!')).filter(|token| !token.is_empty()).collect();
    let pair = tokens
        .iter()
        .find_map(|token| {
            let (width, height) = token.trim_end_matches('.').split_once([':', 'x', '/'])?;
            Some((width.parse::<u32>().ok()?, height.parse::<u32>().ok()?))
        })
        .or_else(|| tokens.windows(3).find_map(|three| (three[1] == "by").then(|| Some((three[0].parse::<u32>().ok()?, three[2].trim_end_matches('.').parse::<u32>().ok()?))).flatten()))?;
    let (width, height) = pair;
    let key = (width.min(height), width.max(height));
    match RATIOS.iter().find(|(ratio, _)| *ratio == key) {
        Some((_, aspect)) => Some(Shape::Listed(*aspect, turned.or((width != height).then_some(height > width)))),
        None => Some(Shape::Other(format!("{width}:{height}"))),
    }
}

/// Requests about the crop, the straighten angle and turning the photo by degrees.
pub(super) fn geometry(message: &str, said: &[String]) -> Option<Reply> {
    let raw = raw_words(message);
    let has = |list: &[&str]| said.iter().any(|word| list.contains(&word.as_str()));
    // A snapshot or preset may have "crop" in its name, and "paste the crop" is about what
    // was copied; those are read as what they are.
    if has(&["snapshot", "preset", "paste"]) {
        return None;
    }
    let cropping = has(&["crop", "cropping", "cropped", "uncrop"]);
    if has(&["uncrop"]) || ((cropping || has(&["straighten", "straightening", "angle"])) && has(&["reset", "remove", "undo", "clear", "no"])) {
        return Some(Reply::Do("The crop and straightening are back to none (turns and flips stay).".to_owned(), Action::ResetCrop));
    }
    if cropping || has(&["ratio", "aspect"]) || (has(&["square"]) && has(&["make", "it", "photo", "format"])) {
        match shape(message, &raw) {
            Some(Shape::Listed(aspect, upright)) => return Some(Reply::Do(String::new(), Action::Crop { aspect, upright })),
            Some(Shape::Other(written)) => {
                let offered: Vec<&str> = ASPECTS.iter().skip(1).map(|(name, _)| *name).collect();
                return Some(Reply::Say(format!("I can't hold the crop to {written}. The shapes it can be held to are {}. For any other, press R and drag the crop's corners.", offered.join(", "))));
            }
            None => {}
        }
    }
    if cropping && has(&["swap", "portrait", "landscape", "vertical", "horizontal", "upright", "sideway"]) {
        let upright = if has(&["portrait", "vertical", "upright"]) {
            Some(true)
        } else if has(&["landscape", "horizontal", "sideway"]) {
            Some(false)
        } else {
            None
        };
        return Some(Reply::Do(String::new(), Action::TurnCrop(upright)));
    }
    // An angle in degrees: quarter turns turn the photo, small angles straighten it. "Turn" is
    // said of sliders too ("turn the contrast down 10"), so it needs a direction or "degrees".
    let turning = has(&["rotate", "tilt"]) || (has(&["turn"]) && has(&["degree", "left", "right", "clockwise", "counterclockwise", "anticlockwise", "ccw", "upside"]));
    let straightening = has(&["straighten", "level", "angle"]);
    if (turning || straightening)
        && edits::named_setting(said).is_none()
        && let Some(number) = edits::number(message)
    {
        let leftward = has(&["left", "counterclockwise", "anticlockwise", "ccw"]);
        let degrees = if leftward { -number.value.abs() } else { number.value };
        let quarters = (degrees / 90.0).round();
        if turning && degrees != 0.0 && (degrees - quarters * 90.0).abs() < 0.01 && quarters.abs() <= 3.0 {
            let turns = quarters.abs() as u8;
            let clockwise = quarters > 0.0;
            let text = match turns {
                2 => "Turned upside down (two quarter turns).".to_owned(),
                _ => format!("Turned {} quarter turn{} {}.", if turns == 1 { "a" } else { "three" }, if turns == 1 { "" } else { "s" }, if clockwise { "right" } else { "left" }),
            };
            return Some(Reply::Do(text, Action::Rotate { clockwise, turns }));
        }
        if degrees.abs() > MAX_ANGLE {
            return Some(Reply::Say(format!("I can turn the photo in quarter turns (“rotate right”, “rotate 180”), or straighten it by up to {MAX_ANGLE:.0}° either way (“straighten 2 degrees”).")));
        }
        return Some(Reply::Do(String::new(), Action::Straighten(degrees)));
    }
    // No angle given: the photo's own lines are measured for it (see `straighten`).
    if has(&["straighten", "horizon", "crooked", "tilted", "wonky", "lopsided"]) || (has(&["level"]) && (has(&["it", "photo", "horizon", "image", "picture", "auto", "this"]) || said.len() == 1)) {
        return Some(Reply::Do(String::new(), Action::Level));
    }
    // "Crop it tighter", "crop a bit off the edges", "crop in by 20%": the same shape and
    // middle, a tenth smaller each way unless the request says how much. "Looser" goes back out.
    if cropping || has(&["trim"]) {
        let looser = has(&["looser", "loose", "wider", "less"]);
        let tighter = has(&["tighter", "tight", "closer", "in", "edge", "border", "off", "more", "bit", "little", "slightly", "lot", "trim"]);
        if looser || tighter {
            let share = match edits::number(message) {
                Some(number) if (1.0..=50.0).contains(&number.value.abs()) => number.value.abs() / 100.0,
                Some(_) => return Some(Reply::Say("I can crop in by 1% to 50% at a time: “crop in by 20%”.".to_owned())),
                None => 0.1 * super::strength(said),
            };
            return Some(Reply::Do(String::new(), Action::TightenCrop(if looser { -share } else { share })));
        }
    }
    // "Crop", "crop it", "open the crop tool": no shape named, so the tool itself.
    const OPENING: &[&str] = &["crop", "cropping", "it", "this", "the", "photo", "image", "picture", "open", "start", "use", "tool", "please", "show", "me"];
    let asks_first = raw.first().is_some_and(|first| matches!(first.as_str(), "crop" | "open" | "start" | "use" | "show" | "please"));
    if cropping && asks_first && said.iter().all(|word| OPENING.contains(&word.as_str())) {
        return Some(Reply::Do("Crop & Straighten is open: drag a corner or edge to crop, and press Enter when you're done. Or name a shape, like “crop to 16:9” or “crop square”.".to_owned(), Action::OpenCrop));
    }
    None
}

/// An export request: "export", "export as JPEG at 80% quality, 2048 pixels wide", "export all".
pub(super) fn export(message: &str, said: &[String]) -> Option<Reply> {
    let has = |list: &[&str]| said.iter().any(|word| list.contains(&word.as_str()));
    let raw = raw_words(message);
    let mut formats: Vec<ExportFormat> = Vec::new();
    for word in &raw {
        let format = match word.as_str() {
            "jpeg" | "jpg" | "jpegs" | "jpgs" => ExportFormat::Jpeg,
            "png" | "pngs" => ExportFormat::Png,
            "tiff" | "tif" | "tiffs" => ExportFormat::Tiff,
            "webp" => ExportFormat::Webp,
            "bmp" => ExportFormat::Bmp,
            _ => continue,
        };
        if !formats.contains(&format) {
            formats.push(format);
        }
    }
    // "Save as a JPEG" is an export too; plain "save" is the project's Save.
    if !(has(&["export", "exporting"]) || (has(&["save", "write", "output"]) && !formats.is_empty())) {
        return None;
    }
    let lower = message.to_lowercase();
    let now = has(&["now", "directly", "immediately", "instantly"]) || ["straight away", "right away", "straightaway", "at once"].iter().any(|phrase| lower.contains(phrase));
    let mut ask = ExportAsk { formats, all: has(&["all", "every", "each", "everything", "other"]) || everyone(said), now, ..ExportAsk::default() };
    if has(&["full", "original"]) && has(&["size", "resolution", "res"]) {
        ask.size = Some(SizeAsk::Full);
    }
    if has(&["best", "maximum", "max", "highest"]) && has(&["quality"]) {
        ask.quality = Some(100);
    }
    // Each number is read by what stands beside it: "80%" and "quality 80" are the quality;
    // "2048 px", "2048 pixels wide" and "long edge 2048" the size. A number with nothing
    // beside it is the quality if it could be one, else the size.
    let tokens: Vec<&str> = lower.split(|character: char| character.is_whitespace() || matches!(character, ',' | ';' | '(' | ')')).filter(|token| !token.is_empty()).collect();
    for (index, token) in tokens.iter().enumerate() {
        let token = token.trim_end_matches('.');
        if token == "4k" {
            ask.size = Some(SizeAsk::Long(3840));
            continue;
        }
        let Ok(value) = token.trim_end_matches('%').trim_end_matches("px").parse::<u32>() else { continue };
        let beside = |offset: isize| index.checked_add_signed(offset).and_then(|at| tokens.get(at)).map(|word| word.trim_end_matches(['.', ':']));
        let before = |list: &[&str]| beside(-1).is_some_and(|word| list.contains(&word));
        let after = |list: &[&str]| [beside(1), beside(2)].into_iter().flatten().any(|word| list.contains(&word));
        let pixels = token.ends_with("px");
        let is_quality = token.ends_with('%') || (!pixels && (value <= 100 && before(&["quality", "q"]) || beside(1) == Some("quality")));
        let is_size = pixels || before(&["edge", "side", "width", "height"]) || after(&["px", "pixel", "pixels", "wide", "tall", "high", "long", "longest"]);
        if is_quality || (!is_size && value <= 100 && ask.quality.is_none()) {
            if (1..=100).contains(&value) {
                ask.quality = Some(value as u8);
            }
        } else if value >= 16 {
            ask.size = Some(if after(&["wide", "across"]) || before(&["width"]) {
                SizeAsk::Wide(value)
            } else if after(&["tall", "high"]) || before(&["height"]) {
                SizeAsk::Tall(value)
            } else {
                SizeAsk::Long(value)
            });
        }
    }
    Some(Reply::Do(String::new(), Action::Export(ask)))
}

/// The name given in a message: what follows "called" or "named", or what is in quotes, as it
/// was typed.
fn given_name(message: &str) -> Option<String> {
    let tidy = |name: &str| {
        let name = name.trim().trim_matches(|character: char| matches!(character, '"' | '“' | '”' | '.' | '!' | '?' | ',')).trim();
        (!name.is_empty()).then(|| name.chars().take(100).collect::<String>())
    };
    if let Some(start) = message.find(['“', '"']) {
        let rest = &message[start..];
        let rest = &rest[rest.chars().next().map_or(0, char::len_utf8)..];
        let end = rest.find(['”', '"']).unwrap_or(rest.len());
        return tidy(&rest[..end]);
    }
    let lower = message.to_lowercase();
    let (at, word) = [" called ", " named ", " titled "].iter().filter_map(|word| lower.find(word).map(|at| (at, word))).min_by_key(|(at, _)| *at)?;
    tidy(message.get(at + word.len()..)?)
}

/// Whether a message gives something a name, so an "and" in it may be part of the name.
pub(super) fn names_something(message: &str) -> bool {
    given_name(message).is_some()
}

/// `message` with the name it gives after "called" or "named" put in quotes, so whatever
/// splits a message at "and" leaves the name whole ("save a snapshot called Warm and Soft").
/// A message that gives no name that way, or quotes it already, comes back as it is.
pub fn quote_name(message: &str) -> String {
    if message.contains(['“', '"']) {
        return message.to_owned();
    }
    let lower = message.to_lowercase();
    let Some((at, word)) = [" called ", " named ", " titled "].iter().filter_map(|word| lower.find(word).map(|at| (at, word))).min_by_key(|(at, _)| *at) else { return message.to_owned() };
    let (Some(head), Some(name)) = (message.get(..at + word.len()), given_name(message)) else { return message.to_owned() };
    format!("{head}“{name}”")
}

/// The snapshot a message means: by the longest name written in it, by number ("snapshot 2"),
/// or the latest or first. With one snapshot and nothing said, that one.
fn meant_snapshot(message: &str, said: &[String], scene: &Scene) -> Option<usize> {
    let snapshots = &scene.photo.snapshots;
    let lower = message.to_lowercase();
    let by_name = snapshots.iter().enumerate().filter(|(_, snapshot)| !snapshot.name.is_empty() && lower.contains(&snapshot.name.to_lowercase())).max_by_key(|(_, snapshot)| snapshot.name.len()).map(|(index, _)| index);
    let has = |list: &[&str]| said.iter().any(|word| list.contains(&word.as_str()));
    by_name
        .or_else(|| said.iter().find_map(|word| word.parse::<usize>().ok()).filter(|number| (1..=snapshots.len()).contains(number)).map(|number| number - 1))
        .or_else(|| has(&["last", "latest", "newest", "recent"]).then(|| snapshots.len().checked_sub(1)).flatten())
        .or_else(|| (has(&["first", "oldest"]) && !snapshots.is_empty()).then_some(0))
        .or_else(|| (snapshots.len() == 1).then_some(0))
}

/// The photo's snapshots by name, as a sentence.
fn snapshot_list(scene: &Scene) -> String {
    let names: Vec<String> = scene.photo.snapshots.iter().map(|snapshot| format!("“{}”", snapshot.name)).collect();
    match names.len() {
        0 => "This photo has no snapshots yet. Say “save a snapshot called Before” to keep the edit as it is now.".to_owned(),
        1 => format!("This photo has one snapshot: {}.", names[0]),
        count => format!("This photo has {count} snapshots: {}.", names.join(", ")),
    }
}

/// Requests about keeping and bringing back a look: snapshots, presets of your own, and
/// copying and pasting settings. `asked` is whether the message is a question.
pub(super) fn keeping(message: &str, said: &[String], scene: &Scene, asked: bool) -> Option<Reply> {
    let has = |list: &[&str]| said.iter().any(|word| list.contains(&word.as_str()));
    if has(&["snapshot"]) {
        // "What changed?" asks; "before I change it" in a snapshot's request does not.
        let compares = has(&["different", "difference", "differ", "diff", "since", "compare", "versu", "vs"]) || (asked && has(&["changed", "change"]));
        if compares {
            let Some(index) = meant_snapshot(message, said, scene) else { return Some(Reply::Say(snapshot_list(scene))) };
            let snapshot = &scene.photo.snapshots[index];
            let changed = super::differences(&snapshot.edit, &scene.photo.edit);
            return Some(Reply::Say(if changed.is_empty() {
                format!("Nothing: the edit is exactly as the snapshot “{}” has it.", snapshot.name)
            } else {
                format!("Since the snapshot “{}”:\n• {}", snapshot.name, changed.join("\n• "))
            }));
        }
        if has(&["list", "which", "what", "many", "any"]) {
            return Some(Reply::Say(snapshot_list(scene)));
        }
        if asked {
            return None;
        }
        // Throwing a snapshot away or renaming it is left to its own buttons: a message that
        // names one must never be taken for "save another".
        if has(&["delete", "remove", "trash", "rename", "update", "replace", "overwrite"]) {
            return Some(Reply::Say("I don't delete, rename or update snapshots myself. Open Snapshots on the rail and use its buttons: double-click one to rename it, and its arrow and trash buttons update or delete it.".to_owned()));
        }
        if has(&["restore", "back", "apply", "load", "return", "revert", "use", "open", "go"]) {
            return Some(match meant_snapshot(message, said, scene) {
                Some(index) => Reply::Do(format!("Back to the snapshot “{}”.", scene.photo.snapshots[index].name), Action::RestoreSnapshot(index)),
                None => Reply::Say(format!("I'm not sure which snapshot you mean. {}", snapshot_list(scene))),
            });
        }
        let name = given_name(message);
        let text = match &name {
            Some(name) => format!("Saved the edit as it is now as the snapshot “{name}” (Snapshots, on the rail)."),
            None => "Saved a snapshot of the edit as it is now (Snapshots, on the rail).".to_owned(),
        };
        return Some(Reply::Do(text, Action::Snapshot(name)));
    }
    if asked {
        return None;
    }
    if has(&["preset"]) && has(&["save", "make", "create", "new", "keep", "store"]) {
        return Some(Reply::Do(String::new(), Action::SavePreset(given_name(message))));
    }
    let settings = has(&["setting", "look", "edit", "adjustment", "develop"]);
    if has(&["copy"]) && settings && !everyone(said) {
        return Some(Reply::Do(String::new(), Action::CopySettings));
    }
    // Paste Settings leaves the crop to each photo; asked for by name, it is carried over too.
    let crop = has(&["crop", "cropping", "framing"]) && !has(&["without", "but", "except", "not"]);
    if has(&["paste"]) && (settings || crop || said.iter().all(|word| ["paste", "it", "that", "them", "here", "please", "the", "this", "on", "onto", "photo", "everything", "all"].contains(&word.as_str()))) {
        let look = !(crop && (has(&["only", "just"]) || !(settings || has(&["everything", "all", "with", "too", "including"]))));
        return Some(Reply::Do(String::new(), Action::PasteSettings { look, crop }));
    }
    None
}

/// Whether a message asks if the photo is finished: "is this ready?", "ready to export?".
pub(super) fn asks_ready(message: &str, said: &[String]) -> bool {
    let has = |list: &[&str]| said.iter().any(|word| list.contains(&word.as_str()));
    let lower = message.to_lowercase();
    has(&["preflight"]) || has(&["ready", "finished"]) && !has(&["not"]) || lower.contains("final check") || lower.contains("am i done") || lower.contains("good to go") || lower.contains("is it done") || lower.contains("is this done")
}

/// Words that say a request is about every photo in a folder, before the folder is named:
/// "every photo in", "everything in", "all the pictures in".
const EVERY: &[&str] = &["every photo in", "every picture in", "every image in", "every file in", "everything in", "all photos in", "all the photos in", "all pictures in", "all the pictures in", "all images in", "all the images in", "all files in", "the photos in", "the pictures in", "each photo in", "photos in", "pictures in"];

/// Words that join the request to the folder phrase and add nothing: "apply portrait finish
/// *to* every photo…", "run *on* everything in…".
const LINKS: &[&str] = &["to", "on", "for", "with", "across", "over"];

/// A folder batch: "apply portrait finish to every photo in this folder and export", "auto
/// every photo in ~/Pictures/Trip", "export everything in this folder as JPEG". The folder is
/// "this folder" (the photo showing's) or a path starting "~" or "/", quoted when it has words
/// of its own after it.
pub(super) fn folder(message: &str) -> Option<Reply> {
    let lower = message.to_lowercase();
    let (at, every) = EVERY.iter().filter_map(|phrase| lower.find(&format!("{phrase} ")).map(|at| (at, phrase))).min_by_key(|(at, _)| *at)?;
    let after = &message[at + every.len() + 1..];
    let after_lower = after.to_lowercase();
    // The folder, and what is said after it.
    let (place, rest) = if let Some(rest) = after_lower.strip_prefix("this folder").or_else(|| after_lower.strip_prefix("the folder")).or_else(|| after_lower.strip_prefix("this photo's folder")) {
        (FolderPlace::This, &after[after.len() - rest.len()..])
    } else if let Some(quoted) = after.strip_prefix(['"', '“']) {
        let end = quoted.find(['"', '”'])?;
        (FolderPlace::Named(quoted[..end].to_owned()), &quoted[end + quoted[end..].chars().next().map_or(0, char::len_utf8)..])
    } else if after.starts_with(['~', '/']) {
        // The path runs to where the request goes on: " and ", " then ", " as ", or the end.
        let end = [" and ", " then ", " as ", ", ", "; "].iter().filter_map(|word| after_lower.find(word)).min().unwrap_or(after.len());
        (FolderPlace::Named(after[..end].trim().trim_end_matches(['.', '!']).to_owned()), &after[end..])
    } else {
        return None;
    };
    // What is asked: the words before the folder phrase, and any after it.
    let before = message[..at].trim();
    let before = before.split_whitespace().collect::<Vec<_>>();
    let before = match before.split_last() {
        Some((last, start)) if LINKS.contains(&last.to_lowercase().as_str()) => start.to_vec(),
        _ => before,
    };
    let mut asked = before.join(" ");
    let rest = rest.trim().trim_start_matches(',').trim();
    if !rest.is_empty() {
        asked = format!("{asked} {rest}");
    }
    // "Apply portrait finish" is "portrait finish": the recipe's own name runs it.
    let asked = asked.trim();
    let asked = ["apply ", "run ", "do ", "use "].iter().find_map(|verb| asked.to_lowercase().starts_with(verb).then(|| &asked[verb.len()..])).unwrap_or(asked);
    let mut requests: Vec<String> = Vec::new();
    let mut export = None;
    // Split as a list of edits is ("black and white" stays one), and at "then".
    for part in asked.split(" then ").flat_map(super::edits::clauses) {
        let said = super::words(&part);
        match self::export(&part, &said) {
            Some(Reply::Do(_, Action::Export(ask))) => export = Some(ExportAsk { all: false, now: true, ..ask }),
            _ => requests.push(part),
        }
    }
    Some(Reply::Do(String::new(), Action::Folder(FolderJob { place, requests, export })))
}

/// A request to send the photo to another app: "open this photo in Edit", "send this to Omina
/// Motion", "put this photo in Motion".
pub(super) fn send_to(said: &[String]) -> Option<Reply> {
    let has = |list: &[&str]| said.iter().any(|word| list.contains(&word.as_str()));
    if !has(&["send", "open", "put", "take", "move", "bring", "hand", "edit", "use"]) || has(&["how", "where"]) {
        return None;
    }
    let to = if has(&["canva", "canvas"]) || said.windows(2).any(|pair| pair[0] == "in" && pair[1] == "edit") || said.windows(2).any(|pair| pair[0] == "to" && pair[1] == "edit") || said.windows(2).any(|pair| pair[0] == "omina" && pair[1] == "edit") {
        OtherApp::Edit
    } else if has(&["motion", "studio"]) {
        OtherApp::Motion
    } else {
        return None;
    };
    Some(Reply::Do(String::new(), Action::SendTo(to)))
}

/// Every word these readers listen for, for putting a mistyped message right.
pub(super) fn vocabulary() -> Vec<&'static str> {
    let groups = SYNC_GROUPS.iter().flat_map(|(names, _)| names.iter().copied());
    let own = [
        "export", "quality", "pixels", "width", "height", "square", "original", "ratio", "aspect", "portrait", "landscape", "vertical", "horizontal", "straighten", "horizon", "degrees", "rotate", "counterclockwise", "clockwise", "snapshot", "snapshots",
        "restore", "preset", "presets", "called", "named", "paste", "settings", "previous", "others", "photos", "every", "synchronize", "ready", "finished", "preflight", "different", "difference", "compare", "uncrop", "swap",
    ];
    groups.chain(own).collect()
}

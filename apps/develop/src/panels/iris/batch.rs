//! Doing the same thing to several open photos at once: copying one photo's settings onto the
//! others ("sync to the others"), Auto or a reset for all of them, and repeating the last
//! request on every other photo ("same for all the others").
//!
//! Each open photo has a history of its own, so there is no single Undo for a change to many.
//! Iris is therefore careful here: she says what will change and on how many photos, and
//! waits for a yes. What she then does is one named step in each photo's own history, and she
//! remembers the photos as she left them, so "undo that" straight afterwards takes the change
//! back on every photo that hasn't been touched since.

use develop_core::chat::{Action, Each, Reply, SyncParts};
use develop_core::crop::ASPECTS;
use develop_core::develop::Develop;
use develop_core::{Edit, Photo};
use ::iris::calc::Units;
use ::iris::front::{self, Turn};

use super::Message;
use super::conversation::{Outcome, allowed_part};
use crate::workspace::Workspace;

/// What is done to the photos.
#[derive(Clone, Debug, PartialEq)]
pub enum What {
    /// Copy these parts of the photo showing onto every other open photo.
    Sync(SyncParts),
    /// Do this to every open photo, the one showing included.
    Each(Each),
    /// Make this request again on every other open photo, read afresh for each.
    Repeat(String),
}

/// A change to several photos that is waiting for a yes or no.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    what: What,
    /// The message that asked for it, which "again" means once it is done.
    asked: String,
}

/// A change made to several photos: its name, and each photo as it was left (by the open
/// photo's session), so it can be taken back where nothing has changed since.
pub struct Done {
    label: String,
    photos: Vec<(u64, Photo)>,
}

/// The step's name in each photo's History.
fn label(what: &What) -> &'static str {
    match what {
        What::Sync(_) => "Iris: Sync Settings",
        What::Each(Each::Auto) => "Iris: Auto",
        What::Each(Each::Reset) => "Iris: Reset Develop",
        What::Repeat(_) => "Iris: Same Again",
    }
}

/// The open photos a change is made to, by their place among the tabs.
fn targets(workspace: &Workspace, what: &What) -> Vec<usize> {
    let active = workspace.state.active;
    (0..workspace.state.sessions.len()).filter(|index| matches!(what, What::Each(_)) || *index != active).collect()
}

/// Photo names as a short list: all of them up to five, then the first few and how many more.
fn listed(names: &[String]) -> String {
    const SHOWN: usize = 5;
    if names.len() <= SHOWN { names.join(", ") } else { format!("{} and {} more", names[..SHOWN - 1].join(", "), names.len() - (SHOWN - 1)) }
}

/// "1 photo" or "3 photos".
fn photos(count: usize) -> String {
    if count == 1 { "1 photo".to_owned() } else { format!("{count} photos") }
}

/// Says what a change to several photos will do and waits for a yes or no. `asked` is the
/// message that asked for it.
pub fn propose(workspace: &mut Workspace, mut what: What, asked: &str) -> Outcome {
    // A photo that isn't cropped has no framing to give the others: syncing its look leaves
    // their crops alone (only "sync only the crop" says to clear them). So does Crop &
    // Straighten being turned off.
    let mut crop_note = "";
    if let What::Sync(parts) = &mut what
        && parts.crop
    {
        let framed = workspace.state.edit_settings().is_some_and(|edit| edit.crop.rect.is_some() || edit.crop.angle.abs() > 0.005);
        let only_the_crop = !parts.masks && parts.develop == develop_core::presets::PresetParts::NONE;
        if !workspace.ui.settings.feature_on(crate::features::Feature::Crop) {
            parts.crop = false;
            crop_note = " Crop & Straighten is turned off, so each keeps its own crop.";
        } else if !framed && !only_the_crop {
            parts.crop = false;
            crop_note = " This photo isn't cropped or straightened, so each keeps its own crop.";
        }
    }
    let found = targets(workspace, &what);
    if found.is_empty() {
        workspace.ui.iris.messages.push(Message::from_iris("This is the only photo open, so there is no other to do it to."));
        return Outcome::failed();
    }
    let names: Vec<String> = found.iter().map(|index| workspace.state.sessions[*index].photo.name.clone()).collect();
    let shown = workspace.state.project_name();
    let (does, yes) = match &what {
        What::Sync(parts) => {
            let crop = if parts.crop {
                " The crop is carried over in proportion to each photo's size, and turned for a photo that stands the other way."
            } else if crop_note.is_empty() {
                " Each keeps its own crop."
            } else {
                crop_note
            };
            (format!("This copies {} from {shown} onto the {} open: {}.{crop}", parts.described(), others(found.len()), listed(&names)), "Yes, sync them")
        }
        What::Each(Each::Auto) => (format!("This sets the light with Auto on all {}: {}. Each is measured on its own.", photos(found.len()), listed(&names)), "Yes, auto them all"),
        What::Each(Each::Reset) => (format!("This puts every Develop setting back to zero on all {}: {}. Their crops and masks stay.", photos(found.len()), listed(&names)), "Yes, reset them all"),
        What::Repeat(request) => (format!("This does “{request}” again on the {} open: {}. It is read afresh for each, so “a bit brighter” is a bit brighter than that photo was.", others(found.len()), listed(&names)), "Yes, do them all"),
    };
    let undo = "Each photo keeps its own history, so ⌘Z only takes it back on the one showing. Say “undo that” straight afterwards and I'll take it back on all of them.";
    workspace.ui.iris.messages.push(Message { replies: vec![yes.to_owned(), "No, leave them".to_owned()], ..Message::from_iris(format!("{does}\n{undo}\nGo ahead?")) });
    workspace.ui.iris.awaiting = Some(Plan { what, asked: asked.to_owned() });
    Outcome::waiting()
}

/// "The other photo" or "3 other photos".
fn others(count: usize) -> String {
    if count == 1 { "other photo".to_owned() } else { format!("{count} other photos") }
}

/// Whether a message says yes or no to a change that is waiting. `None` when it says neither,
/// which lets the question drop.
pub fn answer_to(message: &str) -> Option<bool> {
    let lower = message.trim().to_lowercase().replace('’', "'");
    let starts = |list: &[&str]| list.iter().any(|start| lower == *start || lower.strip_prefix(start).is_some_and(|rest| rest.starts_with([' ', ',', '.', '!'])));
    if starts(&["yes", "y", "yeah", "yep", "ok", "okay", "sure", "go ahead", "do it", "confirm", "please do"]) {
        Some(true)
    } else if starts(&["no", "n", "nope", "cancel", "stop", "don't", "dont", "never mind", "nevermind", "leave"]) {
        Some(false)
    } else {
        None
    }
}

/// Carries out a change that was said yes to, one named history step in each photo, and says
/// how many photos changed and which were left as they were.
pub fn carry_out(workspace: &mut Workspace, plan: Plan) -> Outcome {
    let Plan { what, asked } = plan;
    let Some(from) = workspace.state.photo().cloned() else { return Outcome::failed() };
    let step = label(&what);
    // The requests to repeat, as their steps: a recipe is unfolded and "then" split, without
    // disturbing what "again" means.
    let requests: Vec<String> = match &what {
        What::Repeat(request) => unfolded(workspace, request),
        _ => Vec::new(),
    };
    let kit = Kit::of(workspace);
    let mut changed: Vec<(u64, Photo)> = Vec::new();
    let mut names: Vec<String> = Vec::new();
    let mut left: Vec<String> = Vec::new();
    // Photos that stand the other way, whose crop was turned a quarter to match.
    let mut turned: Vec<String> = Vec::new();
    for index in targets(workspace, &what) {
        let Some(session) = workspace.state.sessions.get(index) else { continue };
        let (id, photo_name) = (session.id, session.photo.name.clone());
        // The photo as the change leaves it, worked out before anything is touched.
        let result: Result<Photo, String> = match &what {
            What::Sync(parts) => {
                let mut photo = session.photo.clone();
                if parts.copy(&from, &mut photo) {
                    turned.push(photo_name.clone());
                }
                Ok(photo)
            }
            What::Each(Each::Auto) => {
                let mut photo = session.photo.clone();
                photo.edit.develop.auto_tone(&workspace.auto_image(&session.photo));
                Ok(photo)
            }
            What::Each(Each::Reset) => {
                let mut photo = session.photo.clone();
                photo.edit.develop = Develop::default();
                Ok(photo)
            }
            What::Repeat(_) => kit.repeated(&session.photo, &requests),
        };
        match result {
            Err(why) => left.push(format!("{photo_name} ({why})")),
            Ok(photo) => {
                if workspace.state.edit_at(index, step, |open| *open = photo.clone()) {
                    changed.push((id, photo));
                    names.push(photo_name);
                } else {
                    left.push(format!("{photo_name} (already that way)"));
                }
            }
        }
    }
    let mut text = if names.is_empty() { "Nothing changed on any photo.".to_owned() } else { format!("Done on {}: {}.", photos(names.len()), listed(&names)) };
    if !left.is_empty() {
        text.push_str(&format!("\nLeft as they were: {}.", left.join(", ")));
    }
    if names.is_empty() {
        workspace.ui.iris.messages.push(Message::from_iris(text));
        return Outcome::default();
    }
    turned.retain(|name| names.contains(name));
    if !turned.is_empty() {
        text.push_str(&format!("\n{} {} the other way up from this photo, so the crop was turned a quarter to match.", listed(&turned), if turned.len() == 1 { "stands" } else { "stand" }));
    }
    text.push_str("\nSay “undo that” now to take it back on all of them.");
    workspace.ui.iris.messages.push(Message::from_iris(text));
    workspace.ui.iris.last_batch = Some(Done { label: step.to_owned(), photos: changed });
    super::conversation::forget_tunable(workspace);
    // "Again" now means this, except for a repeat, which stays what it repeated.
    if !matches!(what, What::Repeat(_)) {
        workspace.ui.iris.memory.last_change = Some(asked);
    }
    Outcome { batch: true, ..Outcome::changed(&["Undo that"]) }
}

/// What making a request on a photo needs to know besides the photo: the presets, the
/// switches in Settings and Iris's name. Copied out of the workspace, so photos can be edited
/// away from it (off the UI thread, for a folder of photos that aren't open).
#[derive(Clone)]
pub struct Kit {
    presets: Vec<develop_core::presets::Preset>,
    settings: crate::app_settings::AppSettings,
    name: String,
}

impl Kit {
    /// What the workspace knows now.
    pub fn of(workspace: &Workspace) -> Self {
        Self { presets: crate::panels::presets::all(workspace), settings: workspace.ui.settings.clone(), name: super::name(workspace) }
    }

    /// Iris's reading of `request` about `photo` alone, as if it were the only photo open.
    fn read(&self, photo: &Photo, request: &str) -> Reply {
        let names = vec![photo.name.clone()];
        let presets: Vec<String> = self.presets.iter().map(|preset| preset.name.clone()).collect();
        let scene = develop_core::chat::Scene { photo, measure: None, suggestions: &[], name: &self.name, photo_names: &names, active: 0, presets: &presets, unsaved: false };
        develop_core::chat::reply(request, &scene)
    }

    /// `photo` with `requests` made on it one after another, each read against the photo as
    /// the one before left it. `Err` says why, when none of them could be made.
    pub fn repeated(&self, photo: &Photo, requests: &[String]) -> Result<Photo, String> {
        let mut photo = photo.clone();
        let mut made = 0;
        let mut problem = None;
        for request in requests {
            let reply = self.read(&photo, request);
            match self.apply(&mut photo, reply) {
                Ok(()) => made += 1,
                Err(why) => problem = Some(why),
            }
        }
        match (made, problem) {
            (0, Some(why)) => Err(why),
            (0, None) => Err("nothing to repeat".to_owned()),
            _ => Ok(photo),
        }
    }

    /// Whether `request` is something that can be made on any photo, judged on `photo`
    /// without changing it.
    pub fn can_repeat(&self, photo: &Photo, request: &str) -> bool {
        let mut scratch = photo.clone();
        self.apply(&mut scratch, self.read(photo, request)).is_ok()
    }

    /// Makes what `reply` asks for on `photo`, for the requests that mean the same on any
    /// photo: slider changes, looks, presets, Auto, resets, turns, flips and the crop.
    fn apply(&self, photo: &mut Photo, reply: Reply) -> Result<(), String> {
        let size = photo.image_size();
        match reply {
            Reply::Do(_, Action::Adjust(fix)) => {
                let (allowed, off) = allowed_part(&self.settings, &fix);
                if allowed.changes.is_empty() && !off.is_empty() {
                    return Err("its card is turned off in Settings".to_owned());
                }
                photo.edit = allowed.apply_to_edit(&photo.edit);
            }
            Reply::Do(_, Action::Reset) => photo.edit.develop = Develop::default(),
            Reply::Do(_, Action::ResetAll) => photo.edit = Edit::default(),
            Reply::Do(_, Action::Auto) => {
                let image = crate::commands::auto_image_of(photo);
                photo.edit.develop.auto_tone(&image);
            }
            Reply::Do(_, Action::ApplyPreset(preset_name)) => {
                let preset = self.presets.iter().find(|preset| preset.name == preset_name).ok_or_else(|| format!("no preset called {preset_name}"))?;
                photo.edit.develop = preset.apply_to(&photo.edit.develop);
            }
            Reply::Do(_, Action::Rotate { clockwise, turns }) => {
                for _ in 0..turns {
                    let size = photo.image_size();
                    photo.edit.turn(clockwise, size);
                }
            }
            Reply::Do(_, Action::Flip { horizontal }) => photo.edit.flip(horizontal, size),
            Reply::Do(_, Action::Crop { aspect, upright }) => {
                if let Some(ratio) = ASPECTS[aspect.min(ASPECTS.len() - 1)].1 {
                    photo.edit.crop.rect = Some(develop_core::photo::crop_to_ratio(&photo.edit.crop, size, ratio));
                }
                turn_crop(photo, upright.map(Some));
            }
            Reply::Do(_, Action::TurnCrop(upright)) => turn_crop(photo, Some(upright)),
            Reply::Do(_, Action::ResetCrop) => {
                photo.edit.crop.rect = None;
                photo.edit.crop.angle = 0.0;
            }
            Reply::Do(_, Action::Straighten(degrees)) => photo.edit.crop.set_angle(degrees, size),
            Reply::Do(_, Action::TightenCrop(share)) => crate::state::tighten_crop(photo, share),
            Reply::Do(_, Action::Level) => {
                let angle = crate::commands::level_angle_from_original(photo).ok_or_else(|| "no clear line to level by".to_owned())?;
                photo.edit.crop.set_angle(angle, size);
            }
            _ => return Err("that isn't something I can do to a photo that isn't showing".to_owned()),
        }
        Ok(())
    }
}

/// The requests a message stands for, as their steps: a recipe is unfolded (with any values
/// it takes) and "then" split, without disturbing what "again" means.
pub fn unfolded(workspace: &Workspace, request: &str) -> Vec<String> {
    if super::conversation::holds_stray_braces(&workspace.ui.iris.memory, request) {
        return vec![request.to_owned()];
    }
    let mut scratch = workspace.ui.iris.memory.clone();
    front::read(request, &mut scratch, &Units::default(), &[]).turns.into_iter().filter_map(|turn| if let Turn::Ask(text) = turn { Some(text) } else { None }).collect()
}

/// Stands `photo`'s crop upright or lays it on its side: `Some(None)` swaps it, `Some(Some(…))`
/// asks for one way, and `None` leaves it.
fn turn_crop(photo: &mut Photo, upright: Option<Option<bool>>) {
    let Some(upright) = upright else { return };
    let size = photo.image_size();
    let kept = photo.edit.crop.kept(size);
    let swaps = upright.is_none_or(|upright| (kept.height > kept.width) != upright && (kept.height - kept.width).abs() > 0.5);
    if swaps {
        photo.edit.crop.rect = Some(develop_core::photo::crop_swapped(&photo.edit.crop, size));
    }
}

/// Takes a change to several photos back on each photo that is still exactly as the change
/// left it, and says how many that was.
pub fn undo(workspace: &mut Workspace, done: Done) -> Outcome {
    workspace.state.record_history(false);
    let mut undone = 0;
    let mut touched: Vec<String> = Vec::new();
    for (id, as_left) in &done.photos {
        let Some(index) = workspace.state.index_of(*id) else { continue };
        if workspace.state.sessions[index].photo == *as_left && workspace.state.undo_at(index) {
            undone += 1;
        } else {
            touched.push(workspace.state.sessions[index].photo.name.clone());
        }
    }
    let what = done.label.trim_start_matches("Iris: ");
    let mut text = if undone == 0 { format!("I couldn't take “{what}” back on any photo: each has been changed since.") } else { format!("Undone on {}: {what}.", photos(undone)) };
    if undone > 0 && !touched.is_empty() {
        text.push_str(&format!(" {} changed since, so I left {}: undo there by hand (⌘Z).", listed(&touched), if touched.len() == 1 { "it" } else { "them" }));
    }
    workspace.ui.iris.messages.push(Message::from_iris(text));
    super::conversation::forget_tunable(workspace);
    if undone == 0 { Outcome::failed() } else { Outcome { batch: true, ..Outcome::changed(&[]) } }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_yes_or_no_is_told_from_anything_else() {
        for yes in ["yes", "Yes, sync them", "ok", "go ahead", "Do it!", "sure"] {
            assert_eq!(answer_to(yes), Some(true), "{yes}");
        }
        for no in ["no", "No, leave them", "cancel", "never mind", "don’t"] {
            assert_eq!(answer_to(no), Some(false), "{no}");
        }
        for other in ["a bit brighter", "next photo", "yesterday's look", "nothing", "okra"] {
            assert_eq!(answer_to(other), None, "{other}");
        }
    }

    #[test]
    fn long_lists_of_photos_are_cut_short() {
        let names: Vec<String> = ["A", "B", "C", "D", "E", "F", "G"].iter().map(|name| (*name).to_owned()).collect();
        assert_eq!(listed(&names[..3]), "A, B, C");
        assert_eq!(listed(&names), "A, B, C, D and 3 more");
        assert_eq!((photos(1), photos(4), others(1), others(2)), ("1 photo".to_owned(), "4 photos".to_owned(), "other photo".to_owned(), "2 other photos".to_owned()));
    }
}

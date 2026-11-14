//! Every editing action, without a window: the open photos (each with its own file and undo
//! history; one of them is shown and edited at a time), the tool in use, the selected mask,
//! copied settings, before/after, and drags in progress on the photo. Panels read this and
//! forward input to it; it is unit-tested on its own.
//!
//! All changes to the shown photo go through `edit`, which bumps `revision` so the viewer knows
//! to render again, and names the change for the History panel.

pub mod history;
pub mod tools;

use std::path::PathBuf;

use develop_core::crop::ASPECTS;
use develop_core::develop::{Develop, DevelopSetting};
use develop_core::geom::Vec2;
use develop_core::masks::{Mask, MaskShape};
use develop_core::presets::Preset;
use develop_core::{Edit, Photo, Snapshot};

use history::History;
pub use tools::{Interaction, Tool};

/// What a drag on an empty part of the photo makes while the Masks tool is in use.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MaskKind {
    #[default]
    Linear,
    Radial,
}

/// An open photo: what it is, where it is saved, and its undo history.
pub struct Session {
    /// Stays the same while the photo is open, however the open photos are reordered or closed.
    pub id: u64,
    pub photo: Photo,
    /// The project file it was opened from or last saved to; `None` until first saved.
    pub file: Option<PathBuf>,
    pub history: History<Photo>,
    /// The file it was opened from (a photo or a project), which says what folder "this
    /// folder" is. `None` for a photo brought back after Develop closed without saving it.
    pub origin: Option<PathBuf>,
}

/// How the viewer compares the edit with the photo as it came.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Compare {
    /// Just the edited photo.
    #[default]
    Off,
    /// The photo as it came, in place of the edit (Lightroom's \ key).
    Before,
    /// Before on the left of a divider, after on the right.
    Split,
}

pub struct EditorState {
    /// Every open photo, in the order of their tabs.
    pub sessions: Vec<Session>,
    /// Which of them is shown and edited.
    pub active: usize,
    /// The id the next opened photo gets.
    next_session: u64,
    /// Bumped by every change, so the viewer and panels know to catch up.
    pub revision: u64,
    /// The name of the change in progress, for its step in the History panel.
    pub label: Option<String>,
    /// Things that couldn't be done, and why, for the window to show.
    pub notices: Vec<String>,
    pub tool: Tool,
    pub mask_kind: MaskKind,
    /// The mask whose settings and handles show.
    pub selected_mask: Option<usize>,
    /// The crop's ratio, by its place in `ASPECTS` (0 is free).
    pub crop_aspect: usize,
    pub interaction: Option<Interaction>,
    /// Settings copied with Copy Settings, ready to paste onto this photo or another.
    pub copied: Option<Edit>,
    /// The crop of the photo the settings were copied from, with that photo's size (turned as
    /// it was shown), for pasting the framing too when it is asked for by name.
    pub copied_crop: Option<(develop_core::crop::Crop, (u32, u32))>,
    pub compare: Compare,
    /// Where the split's divider is, as a fraction of the photo's width.
    pub split_at: f32,
    /// How many steps undo keeps.
    pub history_limit: usize,
}

impl Default for EditorState {
    fn default() -> Self {
        Self::new()
    }
}

impl EditorState {
    pub fn new() -> Self {
        Self {
            sessions: Vec::new(),
            active: 0,
            next_session: 1,
            revision: 0,
            label: None,
            notices: Vec::new(),
            tool: Tool::View,
            mask_kind: MaskKind::default(),
            selected_mask: None,
            crop_aspect: 0,
            interaction: None,
            copied: None,
            copied_crop: None,
            compare: Compare::Off,
            split_at: 0.5,
            history_limit: 100,
        }
    }

    // ----- The open photos -----

    /// Opens `photo` in a tab of its own after the others and shows it. The photos already open
    /// stay as they are. `first_step` names its first history state.
    pub fn open(&mut self, photo: Photo, file: Option<PathBuf>, first_step: &str) {
        let mut history = History::new(&photo, first_step);
        history.limit = self.history_limit;
        let id = self.next_session;
        self.next_session += 1;
        let origin = file.clone();
        self.sessions.push(Session { id, photo, file, history, origin });
        self.activate(self.sessions.len() - 1);
    }

    /// Opens `photo`, brought back after Develop closed without saving it, as unsaved changes on
    /// `file` (the project it came from, if it had one).
    pub fn open_recovered(&mut self, photo: Photo, file: Option<PathBuf>) {
        self.open(photo, file, "Recovered");
        if let Some(session) = self.sessions.last_mut() {
            session.history.mark_unsaved();
        }
    }

    /// Shows open photo `index`. What belongs to the photo that was showing (the tool's drag,
    /// the selected mask, a comparison) is let go.
    pub fn activate(&mut self, index: usize) {
        if index >= self.sessions.len() {
            return;
        }
        self.active = index;
        self.tool = Tool::View;
        self.selected_mask = None;
        self.interaction = None;
        self.compare = Compare::Off;
        self.label = None;
        self.revision += 1;
    }

    /// Shows the next open photo (or the previous one), wrapping around.
    pub fn activate_next(&mut self, forward: bool) {
        let count = self.sessions.len();
        if count > 1 {
            let next = if forward { (self.active + 1) % count } else { (self.active + count - 1) % count };
            self.activate(next);
        }
    }

    /// Where the open photo with session `id` is among the open photos, if it is still open.
    pub fn index_of(&self, id: u64) -> Option<usize> {
        self.sessions.iter().position(|session| session.id == id)
    }

    /// The open photo saved as `path`, if it is open.
    pub fn find_file(&self, path: &std::path::Path) -> Option<usize> {
        self.sessions.iter().position(|session| session.file.as_deref() == Some(path))
    }

    /// Closes open photo `index`, showing its neighbor (the one after it, or else before it).
    pub fn close(&mut self, index: usize) -> Option<Session> {
        if index >= self.sessions.len() {
            return None;
        }
        let closed = self.sessions.remove(index);
        let next = if self.active > index { self.active - 1 } else { self.active.min(self.sessions.len().saturating_sub(1)) };
        self.activate(next);
        Some(closed)
    }

    /// The shown photo's session.
    pub fn session(&self) -> Option<&Session> {
        self.sessions.get(self.active)
    }

    fn session_mut(&mut self) -> Option<&mut Session> {
        self.sessions.get_mut(self.active)
    }

    pub fn photo(&self) -> Option<&Photo> {
        self.session().map(|session| &session.photo)
    }

    pub fn file(&self) -> Option<&PathBuf> {
        self.session().and_then(|session| session.file.as_ref())
    }

    /// The open photo's edit.
    pub fn edit_settings(&self) -> Option<&Edit> {
        self.photo().map(|photo| &photo.edit)
    }

    /// Changes the shown photo, naming the change `label` for the History panel.
    pub fn edit(&mut self, label: impl Into<String>, change: impl FnOnce(&mut Photo)) {
        let Some(session) = self.session_mut() else { return };
        change(&mut session.photo);
        self.label = Some(label.into());
        self.revision += 1;
    }

    /// Changes the open photo's Develop settings.
    pub fn edit_develop(&mut self, label: impl Into<String>, change: impl FnOnce(&mut Develop)) {
        self.edit(label, |photo| change(&mut photo.edit.develop));
    }

    pub fn notice(&mut self, text: impl Into<String>) {
        self.notices.push(text.into());
    }

    /// The name shown in the title bar: the photo's, or nothing open.
    pub fn project_name(&self) -> String {
        self.photo().map_or_else(|| "No Photo".to_owned(), |photo| photo.name.clone())
    }

    pub fn rename(&mut self, name: &str) {
        let name = name.trim();
        if !name.is_empty() && self.photo().is_some_and(|photo| photo.name != name) {
            let name = name.to_owned();
            self.edit("Rename", |photo| photo.name = name);
        }
    }

    // ----- History -----

    /// Records a history step for the latest change, unless a drag is still going (a whole
    /// slider drag is one step).
    pub fn record_history(&mut self, dragging: bool) {
        if dragging {
            return;
        }
        let label = self.label.take().unwrap_or_else(|| "Edit".to_owned());
        if let Some(session) = self.session_mut() {
            session.history.record(&session.photo, &label);
        }
    }

    pub fn can_undo(&self) -> bool {
        self.session().is_some_and(|session| session.history.can_undo(&session.photo))
    }

    pub fn can_redo(&self) -> bool {
        self.session().is_some_and(|session| session.history.can_redo())
    }

    pub fn undo(&mut self) -> bool {
        let changed = self.session_mut().is_some_and(|session| session.history.undo(&mut session.photo));
        self.after_history_jump(changed)
    }

    pub fn redo(&mut self) -> bool {
        let changed = self.session_mut().is_some_and(|session| session.history.redo(&mut session.photo));
        self.after_history_jump(changed)
    }

    /// Goes back or forward to step `index` in the History panel's list.
    pub fn go_to_history(&mut self, index: usize) -> bool {
        let changed = self.session_mut().is_some_and(|session| session.history.go_to(index, &mut session.photo));
        self.after_history_jump(changed)
    }

    fn after_history_jump(&mut self, changed: bool) -> bool {
        if changed {
            self.label = None;
            self.revision += 1;
            // The selected mask may be gone in the state jumped to.
            let masks = self.edit_settings().map_or(0, |edit| edit.masks.len());
            self.selected_mask = self.selected_mask.filter(|index| *index < masks);
        }
        changed
    }

    /// Changes open photo `index`, whether or not it is the one showing, and records the change
    /// at once as a step of that photo's own history named `label`. Returns whether anything
    /// changed. This is how something is done to several photos in a row: each keeps its own
    /// undo, so the change can be taken back photo by photo.
    pub fn edit_at(&mut self, index: usize, label: &str, change: impl FnOnce(&mut Photo)) -> bool {
        // Anything changed but not yet recorded stays a step of its own.
        if index == self.active {
            self.record_history(false);
        }
        let Some(session) = self.sessions.get_mut(index) else { return false };
        session.history.record(&session.photo, "Edit");
        change(&mut session.photo);
        let changed = session.history.record(&session.photo, label);
        self.after_history_jump(changed)
    }

    /// Undoes the latest step of open photo `index`, whether or not it is the one showing.
    pub fn undo_at(&mut self, index: usize) -> bool {
        let changed = self.sessions.get_mut(index).is_some_and(|session| session.history.undo(&mut session.photo));
        self.after_history_jump(changed)
    }

    /// The name of the step undo would take back, and of the one redo would put back, if any.
    pub fn undo_and_redo_names(&self) -> (Option<String>, Option<String>) {
        let Some(session) = self.session() else { return (None, None) };
        let (names, current) = session.history.states();
        let undo = (current > 0).then(|| names[current].clone());
        (undo, names.get(current + 1).cloned())
    }

    /// Whether the shown photo has changes not yet saved.
    pub fn has_unsaved_changes(&self) -> bool {
        self.is_unsaved(self.active)
    }

    /// Whether open photo `index` has changes not yet saved.
    pub fn is_unsaved(&self, index: usize) -> bool {
        self.sessions.get(index).is_some_and(|session| session.history.has_unsaved_changes(&session.photo))
    }

    /// The first open photo with unsaved changes, if any.
    pub fn first_unsaved(&self) -> Option<usize> {
        (0..self.sessions.len()).find(|index| self.is_unsaved(*index))
    }

    /// Notes that open photo `index` was saved as `path`.
    pub fn mark_saved(&mut self, index: usize, path: PathBuf) {
        if let Some(session) = self.sessions.get_mut(index) {
            session.history.mark_saved(&session.photo);
            session.file = Some(path);
        }
    }

    // ----- Develop -----

    /// Sets one Develop setting (kept within its range).
    pub fn set_develop(&mut self, setting: DevelopSetting, value: f32) {
        self.edit_develop(setting.slider().name, |develop| setting.set(develop, value));
    }

    /// Lightroom's Auto: light and contrast set from `image` (a small copy of the photo).
    pub fn auto_tone(&mut self, image: &develop_core::Bitmap) {
        self.edit_develop("Auto", |develop| develop.auto_tone(image));
    }

    /// The White Balance Selector: makes `color` (picked from the photo as it came) neutral.
    pub fn pick_white_balance(&mut self, color: [f32; 3]) {
        let Some(mut develop) = self.edit_settings().map(|edit| edit.develop.clone()) else { return };
        if develop.neutralize(color) {
            self.edit_develop("White Balance", |current| {
                current.temperature = develop.temperature;
                current.tint = develop.tint;
            });
        } else {
            self.notice("That spot is too dark to judge its color. Pick something light and neutral, like a white wall or a cloud.");
        }
    }

    /// Every Develop setting back to its default (the crop and masks stay).
    pub fn reset_develop(&mut self) {
        self.edit_develop("Reset Develop", |develop| *develop = Develop::default());
    }

    /// Everything back as it came: settings, crop and masks.
    pub fn reset_all(&mut self) {
        self.selected_mask = None;
        self.edit("Reset All", |photo| photo.edit = Edit::default());
    }

    /// Makes an Iris fix (its settings, and any mask it adds), as one undo step named after it.
    pub fn apply_fix(&mut self, fix: &develop_core::assistant::Fix) {
        let fix = fix.clone();
        self.edit(fix.label.clone(), move |photo| photo.edit = fix.apply_to_edit(&photo.edit));
    }

    pub fn apply_preset(&mut self, preset: &Preset) {
        let preset = preset.clone();
        self.edit("Preset", move |photo| photo.edit.develop = preset.apply_to(&photo.edit.develop));
    }

    // ----- Copy and paste settings -----

    /// Copies the photo's look (Develop settings and masks, not its crop).
    pub fn copy_settings(&mut self) {
        self.copied = self.edit_settings().map(Edit::look);
        self.copied_crop = self.photo().map(|photo| (photo.edit.crop.clone(), photo.image_size()));
    }

    /// Pastes the copied photo's crop onto this one in proportion (see `Crop::carried_to`):
    /// the same shape, place and straighten angle, on this photo's own pixels. Returns whether
    /// the crop was turned a quarter because this photo stands the other way, or `None` when
    /// nothing is copied.
    pub fn paste_crop(&mut self) -> Option<bool> {
        let (crop, from) = self.copied_crop.clone()?;
        let photo = self.photo()?;
        let (carried, turned) = crop.carried_to(&photo.edit.crop, from, photo.image_size());
        self.edit("Paste Crop", |photo| photo.edit.crop = carried);
        Some(turned)
    }

    /// Pastes copied settings onto this photo, keeping its crop.
    pub fn paste_settings(&mut self) -> bool {
        let Some(look) = self.copied.clone() else { return false };
        if self.photo().is_none() {
            return false;
        }
        self.selected_mask = None;
        self.edit("Paste Settings", move |photo| {
            photo.edit.develop = look.develop;
            photo.edit.masks = look.masks;
        });
        true
    }

    // ----- Snapshots -----

    /// Saves the current edit as a snapshot, named `name` (or numbered).
    pub fn add_snapshot(&mut self, name: Option<String>) {
        let Some(photo) = self.photo() else { return };
        let name = name.filter(|name| !name.trim().is_empty()).unwrap_or_else(|| format!("Snapshot {}", photo.snapshots.len() + 1));
        let snapshot = Snapshot { name, edit: photo.edit.clone() };
        self.edit("New Snapshot", |photo| photo.snapshots.push(snapshot));
    }

    /// Brings back snapshot `index`'s edit.
    pub fn apply_snapshot(&mut self, index: usize) {
        let Some(snapshot) = self.photo().and_then(|photo| photo.snapshots.get(index)).cloned() else { return };
        self.selected_mask = None;
        self.edit("Snapshot", |photo| photo.edit = snapshot.edit);
    }

    /// Replaces snapshot `index` with the current edit.
    pub fn update_snapshot(&mut self, index: usize) {
        self.edit("Update Snapshot", |photo| {
            let edit = photo.edit.clone();
            if let Some(snapshot) = photo.snapshots.get_mut(index) {
                snapshot.edit = edit;
            }
        });
    }

    pub fn rename_snapshot(&mut self, index: usize, name: &str) {
        let name = name.trim().to_owned();
        if name.is_empty() {
            return;
        }
        self.edit("Rename Snapshot", |photo| {
            if let Some(snapshot) = photo.snapshots.get_mut(index) {
                snapshot.name = name;
            }
        });
    }

    pub fn delete_snapshot(&mut self, index: usize) {
        self.edit("Delete Snapshot", |photo| {
            if index < photo.snapshots.len() {
                photo.snapshots.remove(index);
            }
        });
    }

    // ----- Masks -----

    /// Adds a mask of `shape` and selects it.
    pub fn add_mask(&mut self, shape: MaskShape) -> Option<usize> {
        let count = self.edit_settings()?.masks.len();
        if count >= develop_core::photo::MAX_MASKS {
            self.notice(format!("A photo can have up to {} masks.", develop_core::photo::MAX_MASKS));
            return None;
        }
        let name = format!("Mask {}", count + 1);
        self.edit("New Mask", |photo| photo.edit.masks.push(Mask::new(name, shape)));
        self.selected_mask = Some(count);
        Some(count)
    }

    /// Changes mask `index`.
    pub fn edit_mask(&mut self, index: usize, label: impl Into<String>, change: impl FnOnce(&mut Mask)) {
        self.edit(label, |photo| {
            if let Some(mask) = photo.edit.masks.get_mut(index) {
                change(mask);
            }
        });
    }

    pub fn delete_mask(&mut self, index: usize) {
        self.edit("Delete Mask", |photo| {
            if index < photo.edit.masks.len() {
                photo.edit.masks.remove(index);
            }
        });
        self.selected_mask = match self.selected_mask {
            Some(selected) if selected == index => None,
            Some(selected) if selected > index => Some(selected - 1),
            other => other,
        };
    }

    pub fn duplicate_mask(&mut self, index: usize) {
        let Some(mut mask) = self.edit_settings().and_then(|edit| edit.masks.get(index)).cloned() else { return };
        mask.name = format!("{} copy", mask.name);
        let count = self.edit_settings().map_or(0, |edit| edit.masks.len());
        if count >= develop_core::photo::MAX_MASKS {
            return;
        }
        self.edit("Duplicate Mask", |photo| photo.edit.masks.push(mask));
        self.selected_mask = Some(count);
    }

    // ----- Crop -----

    /// The image's size (the original turned as the crop says).
    pub fn image_size(&self) -> Option<(u32, u32)> {
        self.photo().map(Photo::image_size)
    }

    /// Turns the photo a quarter turn, with its crop and masks.
    pub fn rotate(&mut self, clockwise: bool) {
        let Some(size) = self.image_size() else { return };
        self.edit(if clockwise { "Rotate Right" } else { "Rotate Left" }, |photo| photo.edit.turn(clockwise, size));
    }

    /// Mirrors the photo, with its crop and masks.
    pub fn flip(&mut self, horizontal: bool) {
        let Some(size) = self.image_size() else { return };
        self.edit(if horizontal { "Flip Horizontal" } else { "Flip Vertical" }, |photo| photo.edit.flip(horizontal, size));
    }

    /// Sets the straighten angle.
    pub fn set_angle(&mut self, degrees: f32) {
        let Some(size) = self.image_size() else { return };
        self.edit("Straighten", |photo| photo.edit.crop.set_angle(degrees, size));
    }

    /// Holds the crop to ratio `index` in `ASPECTS`, reshaping it now.
    pub fn set_crop_aspect(&mut self, index: usize) {
        let index = index.min(ASPECTS.len() - 1);
        self.crop_aspect = index;
        let (Some(ratio), Some(size)) = (ASPECTS[index].1, self.image_size()) else { return };
        self.edit("Crop Ratio", |photo| {
            let rect = develop_core::photo::crop_to_ratio(&photo.edit.crop, size, ratio);
            photo.edit.crop.rect = Some(rect);
        });
    }

    /// Swaps a held crop ratio between landscape and portrait.
    pub fn swap_crop_orientation(&mut self) {
        self.edit("Crop Ratio", |photo| photo.edit.crop.rect = Some(develop_core::photo::crop_swapped(&photo.edit.crop, photo.image_size())));
    }

    /// Holds the crop to ratio `index` in `ASPECTS` and, when `upright` says so, stands it
    /// taller than wide (or lays it wider than tall), all as one change.
    pub fn shape_crop(&mut self, index: usize, upright: Option<bool>) {
        self.set_crop_aspect(index);
        self.turn_crop(upright);
    }

    /// Stands the crop upright or lays it on its side, as `upright` says; `None` swaps
    /// whichever it is now. A crop already lying the way asked for is left alone.
    pub fn turn_crop(&mut self, upright: Option<bool>) {
        let Some(photo) = self.photo() else { return };
        let kept = photo.edit.crop.kept(photo.image_size());
        let swaps = upright.is_none_or(|upright| (kept.height > kept.width) != upright && (kept.height - kept.width).abs() > 0.5);
        if swaps {
            self.swap_crop_orientation();
        }
    }

    /// Crops in by `share` of the width and height (0.1 takes a tenth off each), keeping the
    /// crop's shape and middle. A negative share crops back out, as far as the photo allows.
    pub fn tighten_crop(&mut self, share: f32) {
        self.edit(if share >= 0.0 { "Crop Tighter" } else { "Crop Looser" }, |photo| tighten_crop(photo, share));
    }

    /// Straightens the photo to `degrees`, as measured from its own lines (see
    /// `develop_core::straighten`).
    pub fn level(&mut self, degrees: f32) {
        self.set_angle(degrees);
        if self.label.is_some() {
            self.label = Some("Level".to_owned());
        }
    }

    /// The crop and straightening back to none (turns and flips stay).
    pub fn reset_crop(&mut self) {
        self.edit("Reset Crop", |photo| {
            photo.edit.crop.rect = None;
            photo.edit.crop.angle = 0.0;
        });
    }

    /// The ratio the crop is held to while dragging its handles, width over height, if any.
    pub fn held_ratio(&self) -> Option<f32> {
        let (_, ratio) = ASPECTS[self.crop_aspect.min(ASPECTS.len() - 1)];
        let ratio = ratio?;
        let size = self.image_size()?;
        // "Original" holds the current crop's own shape.
        let kept = self.edit_settings()?.crop.kept(size);
        Some(if ratio > 0.0 { if kept.width >= kept.height { ratio.max(1.0 / ratio) } else { ratio.min(1.0 / ratio) } } else { kept.width / kept.height.max(1e-3) })
    }

    // ----- Tools -----

    pub fn set_tool(&mut self, tool: Tool) {
        if self.tool != tool {
            self.tool = tool;
            self.interaction = None;
        }
    }

    /// Picks the Masks tool, set to make `kind` on a drag.
    pub fn use_mask_tool(&mut self, kind: MaskKind) {
        self.mask_kind = kind;
        self.set_tool(Tool::Masks);
    }

    /// Toggles showing the photo as it came.
    pub fn toggle_before(&mut self) {
        self.compare = if self.compare == Compare::Before { Compare::Off } else { Compare::Before };
    }

    /// Toggles the before / after split.
    pub fn toggle_split(&mut self) {
        self.compare = if self.compare == Compare::Split { Compare::Off } else { Compare::Split };
    }

    /// The point `fraction` across and down the image, in image pixels.
    pub fn image_point(&self, fraction: Vec2) -> Option<Vec2> {
        let (width, height) = self.image_size()?;
        Some(Vec2::new(fraction.x * width as f32, fraction.y * height as f32))
    }
}

/// Crops `photo` in by `share` of its crop's width and height, about the crop's middle and
/// keeping its shape; a negative share crops back out. The result always fits on the photo.
pub fn tighten_crop(photo: &mut Photo, share: f32) {
    let size = photo.image_size();
    let crop = &photo.edit.crop;
    let kept = crop.kept(size);
    let factor = (1.0 - share).clamp(0.1, 10.0);
    let center = kept.center();
    let (width, height) = (kept.width * factor, kept.height * factor);
    let fitted = crop.constrain(develop_core::Rect::new(center.x - width * 0.5, center.y - height * 0.5, width, height), size);
    photo.edit.crop.rect = Some(fitted);
}

#[cfg(test)]
pub mod test_support {
    use develop_core::{Bitmap, Photo, Rgba, Source};

    /// A small photo, 200 × 100, a gradient so edits show.
    pub fn photo() -> Photo {
        let mut bitmap = Bitmap::filled(200, 100, Rgba::rgb(0.5, 0.5, 0.5));
        for x in 0..200 {
            for y in 0..100 {
                bitmap.set_pixel(x, y, [x as u8, (y * 2) as u8, 128, 255]);
            }
        }
        Photo::new(Source::new("Test.jpg", Vec::new(), bitmap))
    }

    /// An editor with `photo` open.
    pub fn state() -> super::EditorState {
        let mut state = super::EditorState::new();
        state.open(photo(), None, "Import");
        state
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::state;
    use super::*;

    #[test]
    fn opening_another_photo_keeps_the_first_with_its_edits_and_history() {
        let mut state = state();
        state.set_develop(DevelopSetting::Exposure, 1.0);
        state.record_history(false);
        let mut second = super::test_support::photo();
        second.name = "Second".into();
        state.open(second, None, "Import");
        assert_eq!(state.sessions.len(), 2);
        assert_eq!(state.active, 1);
        assert_eq!(state.photo().unwrap().name, "Second");
        assert_eq!(state.edit_settings().unwrap().develop.exposure, 0.0, "the new photo starts fresh");
        // Edits go to the shown photo only, and each keeps its own undo.
        state.set_develop(DevelopSetting::Contrast, 40.0);
        state.record_history(false);
        state.activate(0);
        assert_eq!(state.edit_settings().unwrap().develop.exposure, 1.0);
        assert_eq!(state.edit_settings().unwrap().develop.contrast, 0.0);
        assert!(state.undo());
        assert_eq!(state.edit_settings().unwrap().develop.exposure, 0.0);
        state.activate(1);
        assert_eq!(state.edit_settings().unwrap().develop.contrast, 40.0);
        // The first photo's only change was undone, so only the second has unsaved changes.
        assert_eq!(state.first_unsaved(), Some(1));
    }

    #[test]
    fn closing_a_photo_shows_its_neighbor() {
        let mut state = state();
        for name in ["B", "C"] {
            let mut photo = super::test_support::photo();
            photo.name = name.into();
            state.open(photo, None, "Import");
        }
        state.activate(1);
        state.add_mask(MaskShape::radial(Vec2::new(0.5, 0.5), Vec2::new(0.1, 0.1)));
        state.close(1);
        assert_eq!(state.photo().unwrap().name, "C");
        assert_eq!(state.selected_mask, None, "the mask belonged to the closed photo");
        state.activate_next(true);
        assert_eq!(state.active, 0);
        state.close(1);
        state.close(0);
        assert!(state.photo().is_none() && state.sessions.is_empty());
        assert!(state.close(0).is_none());
    }

    #[test]
    fn edits_are_undone_as_named_steps() {
        let mut state = state();
        state.set_develop(DevelopSetting::Exposure, 1.0);
        state.record_history(false);
        state.set_develop(DevelopSetting::Contrast, 30.0);
        state.record_history(false);
        let names = state.session().unwrap().history.states().0;
        assert_eq!(names, vec!["Import", "Exposure", "Contrast"]);
        assert!(state.has_unsaved_changes());
        assert!(state.undo());
        assert_eq!(state.edit_settings().unwrap().develop.contrast, 0.0);
        assert!(state.redo());
        assert_eq!(state.edit_settings().unwrap().develop.contrast, 30.0);
    }

    #[test]
    fn a_drag_is_one_step() {
        let mut state = state();
        for value in 1..10 {
            state.set_develop(DevelopSetting::Exposure, value as f32 * 0.1);
            state.record_history(true);
        }
        state.record_history(false);
        assert_eq!(state.session().unwrap().history.states().0.len(), 2);
    }

    #[test]
    fn copied_settings_paste_without_the_crop() {
        let mut state = state();
        state.set_develop(DevelopSetting::Saturation, -100.0);
        state.reset_crop();
        state.set_angle(5.0);
        state.copy_settings();
        let mut other = super::test_support::state();
        assert!(!other.paste_settings(), "nothing copied in this editor yet");
        other.copied = state.copied.clone();
        assert!(other.paste_settings());
        let edit = other.edit_settings().unwrap();
        assert_eq!(edit.develop.saturation, -100.0);
        assert_eq!(edit.crop.angle, 0.0);
        // The crop comes across only when it is pasted by name.
        assert_eq!(other.paste_crop(), None, "no crop was copied in this editor");
        other.copied_crop = state.copied_crop.clone();
        assert_eq!(other.paste_crop(), Some(false));
        assert_eq!(other.edit_settings().unwrap().crop.angle, 5.0);
    }

    #[test]
    fn snapshots_bring_back_an_edit() {
        let mut state = state();
        state.set_develop(DevelopSetting::Exposure, 0.5);
        state.add_snapshot(None);
        state.set_develop(DevelopSetting::Exposure, -1.0);
        state.apply_snapshot(0);
        let photo = state.photo().unwrap();
        assert_eq!(photo.snapshots[0].name, "Snapshot 1");
        assert_eq!(photo.edit.develop.exposure, 0.5);
        state.rename_snapshot(0, "  Warm  ");
        assert_eq!(state.photo().unwrap().snapshots[0].name, "Warm");
        state.delete_snapshot(0);
        assert!(state.photo().unwrap().snapshots.is_empty());
    }

    #[test]
    fn deleting_a_mask_keeps_the_selection_on_the_same_mask() {
        let mut state = state();
        for _ in 0..3 {
            state.add_mask(MaskShape::radial(Vec2::new(0.5, 0.5), Vec2::new(0.1, 0.1)));
        }
        assert_eq!(state.selected_mask, Some(2));
        state.delete_mask(0);
        assert_eq!(state.selected_mask, Some(1));
        state.delete_mask(1);
        assert_eq!(state.selected_mask, None);
        state.duplicate_mask(0);
        assert_eq!(state.edit_settings().unwrap().masks[1].name, "Mask 2 copy");
    }

    #[test]
    fn rotating_and_flipping_take_the_masks_along() {
        let mut state = state(); // 200 × 100
        state.add_mask(MaskShape::radial(Vec2::new(0.2, 0.5), Vec2::new(0.1, 0.2)));
        state.rotate(true);
        let MaskShape::Radial { center, radius, .. } = state.edit_settings().unwrap().masks[0].shape else { panic!("radial") };
        // The photo's point (40, 50) is at (50, 40) of the turned 100 × 200 photo.
        assert!(center.distance(Vec2::new(0.5, 0.2)) < 1e-5, "{center:?}");
        assert_eq!(radius, Vec2::new(0.2, 0.1));
        state.flip(false);
        let MaskShape::Radial { center, .. } = state.edit_settings().unwrap().masks[0].shape else { panic!("radial") };
        assert!(center.distance(Vec2::new(0.5, 0.8)) < 1e-5, "{center:?}");
    }

    #[test]
    fn crop_ratios_reshape_the_crop() {
        let mut state = state();
        state.set_crop_aspect(2); // 1 × 1
        let rect = state.edit_settings().unwrap().crop.rect.unwrap();
        assert!((rect.width - rect.height).abs() < 0.5, "{rect:?}");
        assert_eq!(state.held_ratio(), Some(1.0));
        state.set_crop_aspect(5); // 2 × 3
        state.swap_crop_orientation();
        let rect = state.edit_settings().unwrap().crop.rect.unwrap();
        assert!(rect.height > rect.width, "{rect:?}");
        state.reset_crop();
        assert!(state.edit_settings().unwrap().crop.rect.is_none());
    }

    #[test]
    fn a_crop_is_shaped_and_stood_upright_in_one_step() {
        let mut state = state(); // 200 × 100
        state.shape_crop(3, Some(true)); // 4 × 5, upright
        let rect = state.edit_settings().unwrap().crop.rect.unwrap();
        assert!(rect.height > rect.width && (rect.height / rect.width - 1.25).abs() < 0.02, "{rect:?}");
        // Already upright, so asking again leaves it; asking for it on its side swaps it.
        state.turn_crop(Some(true));
        assert_eq!(state.edit_settings().unwrap().crop.rect.unwrap(), rect);
        state.turn_crop(Some(false));
        let lying = state.edit_settings().unwrap().crop.rect.unwrap();
        assert!(lying.width > lying.height, "{lying:?}");
        state.record_history(false);
        assert_eq!(state.session().unwrap().history.states().0, vec!["Import", "Crop Ratio"]);
    }

    #[test]
    fn the_crop_tightens_about_its_middle_and_never_leaves_the_photo() {
        let mut state = state(); // 200 × 100
        state.tighten_crop(0.1);
        let rect = state.edit_settings().unwrap().crop.rect.unwrap();
        assert!((rect.width - 180.0).abs() < 0.01 && (rect.height - 90.0).abs() < 0.01 && (rect.x - 10.0).abs() < 0.01, "{rect:?}");
        // Loosening by more than there is room for stops at the whole photo, in the same shape.
        state.tighten_crop(-0.5);
        let rect = state.edit_settings().unwrap().crop.rect.unwrap();
        assert!((rect.width - 200.0).abs() < 0.5 && (rect.height - 100.0).abs() < 0.5, "{rect:?}");
        state.record_history(false);
        assert_eq!(state.session().unwrap().history.states().0.last().map(String::as_str), Some("Crop Looser"));
    }

    #[test]
    fn a_photo_that_isnt_showing_is_changed_and_undone_in_its_own_history() {
        let mut state = state();
        let mut second = super::test_support::photo();
        second.name = "Second".into();
        state.open(second, None, "Import");
        // A change to the photo showing that hasn't been recorded yet stays its own step.
        state.set_develop(DevelopSetting::Contrast, 10.0);
        assert!(state.edit_at(1, "Iris: Sync", |photo| photo.edit.develop.exposure = 1.0));
        assert!(state.edit_at(0, "Iris: Sync", |photo| photo.edit.develop.exposure = 1.0));
        assert!(!state.edit_at(0, "Iris: Sync", |_| {}), "nothing changed, so no step");
        assert_eq!(state.sessions[0].history.states().0, vec!["Import", "Iris: Sync"]);
        assert_eq!(state.sessions[1].history.states().0, vec!["Import", "Contrast", "Iris: Sync"]);
        assert_eq!(state.undo_and_redo_names(), (Some("Iris: Sync".to_owned()), None));
        assert!(state.undo_at(0));
        assert_eq!(state.sessions[0].photo.edit.develop.exposure, 0.0);
        assert_eq!(state.sessions[1].photo.edit.develop.exposure, 1.0, "each photo is undone on its own");
        assert!(!state.edit_at(7, "Nothing", |_| {}) && !state.undo_at(7), "no such photo");
    }

    #[test]
    fn the_white_balance_selector_refuses_black() {
        let mut state = state();
        state.pick_white_balance([0.0, 0.0, 0.0]);
        assert_eq!(state.notices.len(), 1);
        state.pick_white_balance([0.7, 0.6, 0.5]);
        assert!(state.edit_settings().unwrap().develop.temperature < 0.0);
    }
}

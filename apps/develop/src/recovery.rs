//! Crash recovery. While open photos have unsaved changes, a copy of each is kept in
//! Develop's settings folder as often as Settings ▸ General says (every half minute to start
//! with), and once more if the app quits without having asked about them. If Develop closes
//! without saving (a crash, a power cut), the next launch offers them back: Restore or
//! Discard. Saving a photo, or closing it without saving, takes it out of the copy.
//!
//! The copy is a folder holding a `.develop` file for each unsaved photo and an index naming
//! them, with the project file each is unsaved work on. A `.develop` file holds the original
//! photo, so the copies are written on a background thread rather than holding up the window,
//! and a photo unchanged since its last copy isn't written again. Each time the copy is
//! removed (or written at quit) a generation number moves on, and a background copy started
//! before that is dropped instead of bringing back work that was saved or thrown away meanwhile.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use develop_core::Photo;
use gpui_kit::component::WindowExt as _;
use gpui_kit::{AppContext as _, Context, PromptButton, PromptLevel, Window};
use serde::{Deserialize, Serialize};

use crate::app_settings;
use crate::workspace::Workspace;

/// How often unsaved work is copied when Settings doesn't say, and how long the timer waits
/// before looking again while the copy is turned off.
#[cfg_attr(test, allow(dead_code))]
pub const RECOVERY_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);

/// The largest index read back. It names a few photos, so anything bigger isn't Develop's.
const MAX_INDEX_BYTES: u64 = 1024 * 1024;

/// The most photos offered back, however many the index names.
const MAX_PHOTOS: usize = 100;

/// One photo in the copy: its file in the recovery folder, its name (to ask about it), and
/// the project file it is unsaved work on, if it had one yet.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Entry {
    project: String,
    name: String,
    of: Option<PathBuf>,
}

/// The copy's index, listing its photos in the order of their tabs.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Index {
    photos: Vec<Entry>,
}

/// Where the copy lives. Worked out on the window's thread and handed to the background
/// writer, since tests keep settings in a folder per thread.
#[derive(Clone, Debug)]
struct Files {
    folder: PathBuf,
    index: PathBuf,
}

impl Files {
    fn here() -> Self {
        let folder = app_settings::folder().join("recovery");
        Self { index: folder.join("index.json"), folder }
    }
}

/// The file name a photo's copy is kept under, by its session (tab) id.
fn project_name(id: u64) -> String {
    format!("photo-{id}.develop")
}

/// Whether `name`, read from the index, is one this module writes: `photo-<number>.develop`,
/// with no folders in it. The index is untrusted, so nothing else is opened.
fn is_copy_name(name: &str) -> bool {
    name.strip_prefix("photo-").and_then(|rest| rest.strip_suffix(".develop")).is_some_and(|id| !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit()))
}

/// The recovery folder, for tests to look in.
#[cfg(test)]
pub fn folder() -> PathBuf {
    Files::here().folder
}

/// One open photo with unsaved changes, as it is to be copied. `photo` is `None` when it is
/// unchanged since its last copy, whose file is kept as it is.
struct Copy {
    id: u64,
    name: String,
    photo: Option<Photo>,
    of: Option<PathBuf>,
}

/// An open photo as last copied: its tab, the photo and its project file. Photos compare
/// quickly (their originals are compared by identity), so this tells what changed.
type Copied = (u64, Photo, Option<PathBuf>);

/// The window's part in keeping the copy: whether it is being offered back or written, and
/// what was copied last.
#[derive(Default)]
pub struct Recovery {
    /// Whether the launch is still asking whether to restore last time's copy. Until it is
    /// answered, the copy is left alone, so quitting first doesn't lose it.
    pub offered: bool,
    /// Whether the last copy couldn't be written (said once, until it works again).
    pub failing: bool,
    /// The generation of a copy being written in the background, so the timer doesn't start
    /// another of the same. One left over from an older generation is being dropped anyway.
    writing: Option<u64>,
    /// The photos as last copied, so unchanged ones aren't written again.
    copied: Vec<Copied>,
    /// Moves on each time the copy is removed or written at quit (see the module notes).
    generation: Arc<Mutex<u64>>,
}

/// Locks the generation number. A writer that panicked holding it left nothing half done that
/// matters (the number itself is always whole), so a poisoned lock is used as it is.
fn lock(generation: &Mutex<u64>) -> MutexGuard<'_, u64> {
    generation.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Writes the photos in `copies` and an index of them, unless the generation has moved on from
/// `expected` meanwhile. Each photo is packed and written beside its place first; only then is
/// the lock taken to move them into place, so removing the copy never waits for the slow part.
/// Files of photos no longer listed are removed. Returns whether the copy was put in place.
fn write_copies(copies: &[Copy], files: &Files, generation: &Mutex<u64>, expected: u64) -> Result<bool, String> {
    std::fs::create_dir_all(&files.folder).map_err(|error| format!("{} couldn't be made: {error}", files.folder.display()))?;
    // The preview inside a project file is only for the start screen, which never lists the
    // copies, so a single pixel stands in for it.
    let preview = develop_core::Bitmap::filled(1, 1, develop_core::Rgba::rgb(0.0, 0.0, 0.0));
    let mut staged = Vec::new();
    let packed = copies.iter().try_for_each(|copy| {
        let Some(photo) = &copy.photo else { return Ok(()) };
        let bytes = develop_core::file::save_to_bytes(photo, &preview)?;
        let path = files.folder.join(format!("photo-{}-{expected}.part", copy.id));
        develop_core::image_io::write_atomically(&path, &bytes)?;
        staged.push((path, files.folder.join(project_name(copy.id))));
        Ok::<(), String>(())
    });
    let remove_staged = |staged: &[(PathBuf, PathBuf)]| {
        for (path, _) in staged {
            let _ = std::fs::remove_file(path);
        }
    };
    if let Err(error) = packed {
        remove_staged(&staged);
        return Err(error);
    }
    let current = lock(generation);
    if *current != expected {
        remove_staged(&staged);
        return Ok(false);
    }
    for (index, (from, to)) in staged.iter().enumerate() {
        if let Err(error) = std::fs::rename(from, to) {
            remove_staged(&staged[index..]);
            return Err(format!("{} couldn't be saved: {error}", to.display()));
        }
    }
    let index = Index { photos: copies.iter().map(|copy| Entry { project: project_name(copy.id), name: copy.name.clone(), of: copy.of.clone() }).collect() };
    let text = serde_json::to_vec_pretty(&index).map_err(|error| error.to_string())?;
    develop_core::image_io::write_atomically(&files.index, &text)?;
    // Photos saved or closed since the last copy.
    if let Ok(entries) = std::fs::read_dir(&files.folder) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if is_copy_name(&name) && !index.photos.iter().any(|listed| listed.project == name) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    Ok(true)
}

/// Removes the copy, and moves the generation on so a background copy still being written is
/// dropped.
fn remove_copy(files: &Files, generation: &Mutex<u64>) {
    let mut current = lock(generation);
    *current += 1;
    if let Err(error) = std::fs::remove_dir_all(&files.folder)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        eprintln!("could not remove the recovery copy {}: {error}", files.folder.display());
    }
}

/// Reads the copy's index, or `None` if there is no copy (or nothing in it can be used). Names
/// that aren't this module's own are left out, and so are names given twice.
fn read_index(files: &Files) -> Option<Vec<Entry>> {
    let size = std::fs::metadata(&files.index).ok()?.len();
    if size > MAX_INDEX_BYTES {
        return None;
    }
    let text = std::fs::read(&files.index).ok()?;
    let mut photos = serde_json::from_slice::<Index>(&text).ok()?.photos;
    let mut seen = std::collections::HashSet::new();
    photos.retain(|entry| is_copy_name(&entry.project) && seen.insert(entry.project.clone()) && files.folder.join(&entry.project).is_file());
    photos.truncate(MAX_PHOTOS);
    for entry in &mut photos {
        entry.name = entry.name.chars().filter(|c| !c.is_control()).take(200).collect();
    }
    (!photos.is_empty()).then_some(photos)
}

impl Workspace {
    /// Runs on the timer: keeps the recovery copy up to date, unless Settings turned it off or
    /// last time's copy is still being offered back.
    pub fn keep_work_safe(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.ui.settings.recovery_seconds == 0 || self.ui.recovery.offered {
            return;
        }
        self.keep_recovery_copy(window, cx);
    }

    /// The open photos with unsaved changes, as they are now.
    fn unsaved_photos(&self) -> Vec<Copied> {
        let unsaved = (0..self.state.sessions.len()).filter(|index| self.state.is_unsaved(*index));
        unsaved.map(|index| &self.state.sessions[index]).map(|session| (session.id, session.photo.clone(), session.file.clone())).collect()
    }

    /// Copies unsaved photos aside on a background thread. With none unsaved (back to how they
    /// were saved, by undoing say), an older copy is removed instead: it would offer back work
    /// taken out on purpose. A copy that can't be written is said once, so the person knows to
    /// save.
    pub fn keep_recovery_copy(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let now = self.unsaved_photos();
        if now.is_empty() {
            self.ui.recovery.failing = false;
            self.remove_stale_recovery_copy();
            return;
        }
        let recovery = &mut self.ui.recovery;
        let generation = recovery.generation.clone();
        let expected = *lock(&generation);
        if recovery.writing == Some(expected) || recovery.copied == now {
            return;
        }
        recovery.writing = Some(expected);
        let copies: Vec<Copy> = now
            .iter()
            .map(|(id, photo, of)| {
                let unchanged = recovery.copied.iter().any(|(copied_id, copied, _)| copied_id == id && copied == photo);
                Copy { id: *id, name: photo.name.clone(), photo: (!unchanged).then(|| photo.clone()), of: of.clone() }
            })
            .collect();
        let files = Files::here();
        let writing = cx.background_spawn(async move { write_copies(&copies, &files, &generation, expected) });
        cx.spawn_in(window, async move |this, cx| {
            let written = writing.await;
            this.update_in(cx, |this, window, cx| this.recovery_copy_written(written, now, expected, window, cx)).ok();
        })
        .detach();
    }

    /// Takes in how a background copy went: remembers what was copied, or says (once) that
    /// unsaved work isn't being kept safe.
    fn recovery_copy_written(&mut self, written: Result<bool, String>, photos: Vec<Copied>, generation: u64, window: &mut Window, cx: &mut Context<Self>) {
        let recovery = &mut self.ui.recovery;
        if recovery.writing == Some(generation) {
            recovery.writing = None;
        }
        // A copy made before the last one was removed says nothing about what is there now.
        if *lock(&recovery.generation) != generation {
            return;
        }
        match written {
            Ok(true) => {
                recovery.failing = false;
                recovery.copied = photos;
            }
            Ok(false) => {}
            Err(error) if !recovery.failing => {
                recovery.failing = true;
                let message = format!("Unsaved work can't be kept safe in case Omina Develop closes: {error} Save your photos to keep them.");
                window.push_notification(message.as_str(), cx);
            }
            Err(_) => {}
        }
    }

    /// After a photo is saved or closed without saving: takes it out of the copy straight
    /// away, by removing the copy and starting a fresh one of the photos still unsaved.
    pub fn refresh_recovery_copy(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let files = Files::here();
        if !files.folder.exists() && self.ui.recovery.writing.is_none() {
            return;
        }
        self.remove_recovery_copy();
        self.keep_work_safe(window, cx);
    }

    /// When the app quits: keeps a last copy of unsaved photos, written there and then since
    /// the app is about to close. (Quitting from the window asks about each photo first and
    /// closes those not saved, so this is for quitting that doesn't ask.) Skipped when the copy
    /// is turned off or last time's copy is still being offered back.
    pub fn keep_recovery_copy_on_quit(&mut self) {
        let photos = self.unsaved_photos();
        let recovery = &self.ui.recovery;
        if self.ui.settings.recovery_seconds == 0 || recovery.offered || photos.is_empty() {
            return;
        }
        // Moving the generation on drops a background copy that is older than this one.
        let expected = {
            let mut current = lock(&recovery.generation);
            *current += 1;
            *current
        };
        let copies: Vec<Copy> = photos.into_iter().map(|(id, photo, of)| Copy { id, name: photo.name.clone(), photo: Some(photo), of }).collect();
        if let Err(error) = write_copies(&copies, &Files::here(), &recovery.generation, expected) {
            eprintln!("could not keep unsaved work safe on quitting: {error}");
        }
    }

    /// Removes the recovery copy: the work was saved, or thrown away on purpose.
    pub fn remove_recovery_copy(&mut self) {
        remove_copy(&Files::here(), &self.ui.recovery.generation);
        self.ui.recovery.copied.clear();
    }

    /// Removes a copy left from before the photos were back to how they were saved, unless it
    /// is last time's copy still being offered back.
    fn remove_stale_recovery_copy(&mut self) {
        if !self.ui.recovery.offered && Files::here().folder.exists() {
            self.remove_recovery_copy();
        }
    }

    /// At launch: if Develop closed with unsaved photos last time, asks whether to restore them.
    /// Restore opens each in a tab as unsaved work on the project it came from (with `named`,
    /// the files named on the command line, after them); Discard removes them and carries on
    /// with the usual launch. Returns whether it asked (the launch then waits for the answer).
    pub fn offer_recovery(&mut self, named: Vec<PathBuf>, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let files = Files::here();
        let Some(photos) = read_index(&files) else { return false };
        let message = match photos.as_slice() {
            [only] => format!("Omina Develop closed with unsaved changes to “{}”.", only.name),
            _ => format!("Omina Develop closed with unsaved changes to {} photos.", photos.len()),
        };
        let buttons = [PromptButton::ok("Restore"), PromptButton::new("Discard")];
        let answer = window.prompt(PromptLevel::Warning, &message, Some("Restore them to keep working where you left off."), &buttons, cx);
        self.ui.recovery.offered = true;
        cx.spawn_in(window, async move |this, cx| {
            let restore = answer.await == Ok(0);
            if !restore {
                this.update_in(cx, |this, window, cx| {
                    this.ui.recovery.offered = false;
                    this.remove_recovery_copy();
                    this.launch_as_usual(named, window, cx);
                })
                .ok();
                return;
            }
            // Read like any file (they are checked as Open checks them), off the UI thread.
            let folder = files.folder.clone();
            let read = cx
                .background_spawn(async move { photos.into_iter().map(|entry| (develop_core::file::load(&folder.join(&entry.project)), entry)).collect::<Vec<_>>() })
                .await;
            this.update_in(cx, |this, window, cx| {
                this.ui.recovery.offered = false;
                this.restore_recovered(read, named, window, cx);
            })
            .ok();
        })
        .detach();
        true
    }

    /// Opens the photos read back from the copy, each in a tab, as unsaved changes. Any that
    /// couldn't be read are named in one message; if none could, the usual launch carries on.
    fn restore_recovered(&mut self, read: Vec<(Result<Photo, String>, Entry)>, named: Vec<PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        let mut failed = Vec::new();
        for (photo, entry) in read {
            match photo {
                Ok(photo) => self.state.open_recovered(photo, entry.of),
                Err(error) => failed.push(format!("{}: {error}", entry.name)),
            }
        }
        if !failed.is_empty() {
            crate::project_files::show_error(window, cx, "Some unsaved changes couldn't be restored.", failed.join("\n"));
        }
        if self.state.sessions.is_empty() {
            self.launch_as_usual(named, window, cx);
            return;
        }
        self.state.activate(0);
        self.ui.start_screen = None;
        crate::panels::viewer::set_zoom(self, None);
        if !self.ui.settings.onboarded {
            crate::panels::onboarding::show(self, cx);
        }
        self.open_paths(named, window, cx);
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_copies_own_file_names_are_opened() {
        assert!(is_copy_name("photo-12.develop"));
        for name in ["photo-.develop", "photo-1.develop/../x", "../photo-1.develop", "photo-1a.develop", "photo-1.develop.part", "settings.json", "/etc/photo-1.develop"] {
            assert!(!is_copy_name(name), "{name}");
        }
    }
}

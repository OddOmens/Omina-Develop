//! Doing the same thing to every photo in a folder, without opening them as tabs: "apply
//! portrait finish to every photo in this folder and export", "auto every photo in
//! ~/Pictures/Trip", "export everything in this folder as JPEG".
//!
//! Develop keeps a photo's edit in a project file (`.develop`), which holds the original photo
//! byte for byte beside the settings. So an edit made to a photo that isn't open is kept the
//! same way: a photo with a project beside it of the same name ("Beach.develop") is changed in
//! that project; any other photo gets a new project beside it. The photos themselves are never
//! written to. A request that only exports writes no projects.
//!
//! A folder batch can't be taken back with Undo, since the photos have no history open, so
//! Iris says what will happen and to how many photos, and waits for a yes. It then works
//! through them one at a time off the UI thread, keeping a progress line in the chat up to
//! date, and stops between photos when told "stop".

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use develop_core::chat::{ExportAsk, FolderJob, FolderPlace};
use develop_core::file::EXTENSION;
use develop_core::image_io::is_picture;
use develop_core::{Edit, Photo, View};
use gpui_kit::{AppContext as _, Context, Window};

use super::batch::{Kit, unfolded};
use super::conversation::{Outcome, export_choices};
use super::{Message, show_newest};
use crate::panels::export::{Job, unused_path, write_one};
use crate::workspace::Workspace;

/// One photo in the folder.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Item {
    /// A photo file, with no project of its own beside it.
    Picture(PathBuf),
    /// A project file, which holds its photo and its edit.
    Project(PathBuf),
}

impl Item {
    fn path(&self) -> &Path {
        match self {
            Self::Picture(path) | Self::Project(path) => path,
        }
    }

    /// The file's name, for saying which one.
    fn name(&self) -> String {
        self.path().file_name().map_or_else(String::new, |name| name.to_string_lossy().into_owned())
    }
}

/// What a folder holds, as a batch sees it.
#[derive(Debug, Default)]
struct Contents {
    items: Vec<Item>,
    /// Files that aren't photos or projects Develop opens.
    not_photos: usize,
    /// Photos left out because their project is beside them and stands for them.
    with_projects: usize,
    /// Photos left out because they are open in a tab, by name.
    open: Vec<String>,
}

/// A folder batch waiting for a yes or no, or being worked through.
#[derive(Clone, Debug)]
pub struct Plan {
    folder: PathBuf,
    items: Vec<Item>,
    /// The requests to make on each photo, recipes unfolded.
    requests: Vec<String>,
    export: Option<ExportAsk>,
    /// Where exports go.
    exports_to: PathBuf,
    /// What the batch does, in a few words, for the progress line.
    doing: String,
}

/// A folder batch being worked through.
pub struct Running {
    /// Set to stop it between photos.
    stop: Arc<AtomicBool>,
}

/// `typed` with a leading "~" put as the home folder.
fn expanded(typed: &str) -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    match typed.strip_prefix('~') {
        Some(rest) => home.join(rest.trim_start_matches('/')),
        None => PathBuf::from(typed),
    }
}

/// The photos in `folder` (not in folders inside it), sorted by name. A file open in a tab is
/// left out, as are photos whose project is beside them (the project stands for the photo).
fn contents(folder: &Path, open: &[PathBuf]) -> std::io::Result<Contents> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(folder)?.filter_map(Result::ok).map(|entry| entry.path()).filter(|path| path.is_file()).collect();
    files.sort();
    let mut found = Contents::default();
    let project_of = |path: &Path| path.with_extension(EXTENSION);
    for path in files {
        let hidden = path.file_name().is_some_and(|name| name.to_string_lossy().starts_with('.'));
        let project = path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case(EXTENSION));
        if hidden {
            continue;
        }
        if !project && !is_picture(&path) {
            found.not_photos += 1;
            continue;
        }
        if !project && project_of(&path).is_file() {
            found.with_projects += 1;
            continue;
        }
        if open.contains(&path) {
            found.open.push(path.file_name().map_or_else(String::new, |name| name.to_string_lossy().into_owned()));
            continue;
        }
        found.items.push(if project { Item::Project(path) } else { Item::Picture(path) });
    }
    Ok(found)
}

/// "1 photo" or "3 photos".
fn photos(count: usize) -> String {
    if count == 1 { "1 photo".to_owned() } else { format!("{count} photos") }
}

/// Says what a folder batch will do and to how many photos, and waits for a yes or no.
pub fn propose(workspace: &mut Workspace, job: FolderJob) -> Outcome {
    let say = |workspace: &mut Workspace, text: String| workspace.ui.iris.messages.push(Message::from_iris(text));
    // The folder: the photo showing's, or one named.
    let folder = match &job.place {
        FolderPlace::This => match workspace.state.session().and_then(|session| session.origin.as_ref().or(session.file.as_ref())).and_then(|file| file.parent()) {
            Some(folder) => folder.to_path_buf(),
            None => {
                say(workspace, "This photo wasn't opened from a folder (it was brought back after Omina Develop closed), so I don't know which folder you mean. Name it, like “auto every photo in ~/Pictures/Trip”.".to_owned());
                return Outcome::failed();
            }
        },
        FolderPlace::Named(typed) => expanded(typed),
    };
    if !folder.is_dir() {
        say(workspace, format!("There's no folder at {}. Check the name; a folder in your home folder starts with “~/”.", folder.display()));
        return Outcome::failed();
    }
    // What to do to each photo, checked on the photo showing (or a blank one) before asking.
    let requests: Vec<String> = job.requests.iter().flat_map(|request| unfolded(workspace, request)).collect();
    if requests.is_empty() && job.export.is_none() {
        let offers = ["Auto every photo in this folder", "Export everything in this folder as JPEG"].map(str::to_owned).to_vec();
        workspace.ui.iris.messages.push(Message { replies: offers, ..Message::from_iris(format!("What should I do to the photos in {}? Name a recipe, a look, a preset or an edit, and say “and export” to export them too.", folder.display())) });
        return Outcome::waiting();
    }
    let kit = Kit::of(workspace);
    let sample = workspace.state.photo().cloned().unwrap_or_else(|| Photo::new(develop_core::Source::new("Sample.png", Vec::new(), develop_core::Bitmap::filled(32, 24, develop_core::Rgba::rgb(0.5, 0.5, 0.5)))));
    if let Some(problem) = requests.iter().find(|request| !kit.can_repeat(&sample, request)) {
        say(workspace, format!("I can't do “{problem}” to photos that aren't open. A folder takes edits, looks, presets, Auto, resets, turns and the crop, then an export."));
        return Outcome::failed();
    }
    let open: Vec<PathBuf> = workspace.state.sessions.iter().flat_map(|session| [session.origin.clone(), session.file.clone()]).flatten().collect();
    let found = match contents(&folder, &open) {
        Ok(found) => found,
        Err(error) => {
            say(workspace, format!("{} can't be read: {error}", folder.display()));
            return Outcome::failed();
        }
    };
    if found.items.is_empty() {
        say(workspace, format!("There are no photos to work on in {}{}.", folder.display(), if found.open.is_empty() { String::new() } else { format!(" that aren't open in a tab ({})", found.open.join(", ")) }));
        return Outcome::failed();
    }
    let exports_to = workspace.ui.settings.export_folder.clone().unwrap_or_else(|| folder.join("Exports"));
    // The question: what happens to each photo, to how many, and that Undo won't take it back.
    let count = found.items.len();
    let mut doing: Vec<String> = Vec::new();
    if !job.requests.is_empty() {
        doing.push(format!("make “{}” on each", job.requests.join("”, then “")));
    }
    if let Some(export) = &job.export {
        let photo = workspace.state.photo().cloned().unwrap_or_else(|| sample.clone());
        let choices = export_choices(workspace.ui.settings.export, export, export.formats.first().copied(), photo.edit.output_size(photo.source.size()));
        let formats: Vec<&str> = if export.formats.is_empty() { vec![choices.format.name()] } else { export.formats.iter().map(|format| format.name()).collect() };
        doing.push(format!("export each as {} to {}", formats.join(" and "), exports_to.display()));
    }
    let projects = found.items.iter().filter(|item| matches!(item, Item::Project(_))).count();
    let mut text = format!("This will {} for the {} in {}.", doing.join(", then "), photos(count), folder.display());
    if !requests.is_empty() {
        text.push_str(&format!(
            " The edits are kept in a project beside each photo ({} new, {} saved over with the change); the photos themselves are never written to.",
            count - projects,
            projects
        ));
    }
    let mut left = Vec::new();
    if found.not_photos > 0 {
        left.push(format!("{} file{} that {} photos", found.not_photos, if found.not_photos == 1 { "" } else { "s" }, if found.not_photos == 1 { "isn't one of the" } else { "aren't" }));
    }
    if found.with_projects > 0 {
        left.push(format!("{} that {} a project beside {} (the project is used instead)", photos(found.with_projects), if found.with_projects == 1 { "has" } else { "have" }, if found.with_projects == 1 { "it" } else { "them" }));
    }
    if !found.open.is_empty() {
        left.push(format!("{} open in a tab ({}); say “sync to the others” for those", photos(found.open.len()), found.open.join(", ")));
    }
    if !left.is_empty() {
        text.push_str(&format!(" Left out: {}.", left.join("; ")));
    }
    text.push_str(" Undo can't take this back, as the photos aren't open. You can say “stop” while it works. Go ahead?");
    workspace.ui.iris.messages.push(Message { replies: vec![format!("Yes, do all {count}"), "No, leave them".to_owned()], ..Message::from_iris(text) });
    workspace.ui.iris.awaiting_folder = Some(Plan { folder, items: found.items, requests, export: job.export, exports_to, doing: doing.join(", then ") });
    Outcome::waiting()
}

/// What came of one photo.
struct Done {
    name: String,
    /// The project the edit was kept in, and whether it was new.
    kept: Option<(PathBuf, bool)>,
    exported: Vec<PathBuf>,
    /// What went wrong, when nothing could be done.
    failed: Option<String>,
}

/// Loads one photo, makes the requests on it, keeps the edit in its project and exports it,
/// as `plan` says. Runs off the UI thread. `promised` are the files written so far, which
/// another photo's file must not be named as.
fn work_on(item: &Item, plan: &Plan, kit: &Kit, last: crate::app_settings::ExportChoices, promised: &[PathBuf]) -> Done {
    let mut done = Done { name: item.name(), kept: None, exported: Vec::new(), failed: None };
    let loaded = match item {
        Item::Project(path) => develop_core::file::load(path),
        Item::Picture(path) => crate::project_files::read_any(path).map(|(photo, _)| photo),
    };
    let photo = match loaded {
        Ok(photo) => photo,
        Err(error) => {
            done.failed = Some(error);
            return done;
        }
    };
    let photo = if plan.requests.is_empty() {
        photo
    } else {
        match kit.repeated(&photo, &plan.requests) {
            Ok(edited) => edited,
            Err(why) => {
                done.failed = Some(why);
                return done;
            }
        }
    };
    // Kept in its project: the one it came from, or a new one beside the photo.
    if !plan.requests.is_empty() {
        let (path, new) = match item {
            Item::Project(path) => (path.clone(), false),
            Item::Picture(path) => (unused_path(path.parent().unwrap_or(&plan.folder), &photo.name, EXTENSION, promised), true),
        };
        let preview = preview(&photo);
        match develop_core::file::save(&photo, &preview, &path) {
            Ok(()) => done.kept = Some((path, new)),
            Err(error) => {
                done.failed = Some(error);
                return done;
            }
        }
    }
    if let Some(export) = &plan.export {
        if let Err(error) = std::fs::create_dir_all(&plan.exports_to) {
            done.failed = Some(format!("{} can't be made: {error}", plan.exports_to.display()));
            return done;
        }
        let formats: Vec<Option<develop_core::image_io::ExportFormat>> = if export.formats.is_empty() { vec![None] } else { export.formats.iter().copied().map(Some).collect() };
        for format in formats {
            let choices = export_choices(last, export, format, photo.edit.output_size(photo.source.size()));
            let job = Job { name: photo.name.clone(), source: photo.source.clone(), edit: photo.edit.clone(), choices };
            let mut taken: Vec<PathBuf> = promised.to_vec();
            taken.extend(done.exported.iter().cloned());
            match write_one(&job, &plan.exports_to, &taken) {
                Ok(path) => done.exported.push(path),
                Err(error) => done.failed = Some(error),
            }
        }
    }
    done
}

/// A small picture of `photo` as edited, for its project's preview.
fn preview(photo: &Photo) -> develop_core::Bitmap {
    let (width, height) = photo.source.size();
    let scale = (develop_core::file::PREVIEW_SIDE as f32 / width.max(height).max(1) as f32).min(1.0);
    let small = photo.source.bitmap.resized(((width as f32 * scale).round() as u32).max(1), ((height as f32 * scale).round() as u32).max(1));
    let edit: &Edit = &photo.edit;
    edit.render(&edit.crop.orient(&small), photo.image_size(), scale, View::Result)
}

/// The progress line, kept up to date while the batch works.
fn progress(workspace: &mut Workspace, text: String, stoppable: bool) {
    let messages = &mut workspace.ui.iris.messages;
    let replies = if stoppable { vec!["Stop".to_owned()] } else { Vec::new() };
    match messages.iter_mut().rev().find(|message| message.progress) {
        Some(line) => {
            line.text = text;
            line.replies = replies;
            line.progress = stoppable;
        }
        None => messages.push(Message { progress: stoppable, replies, ..Message::from_iris(text) }),
    }
}

/// Works through a folder batch that was said yes to, off the UI thread, one photo at a time.
pub fn run(workspace: &mut Workspace, plan: Plan, window: &mut Window, cx: &mut Context<Workspace>) -> Outcome {
    let stop = Arc::new(AtomicBool::new(false));
    workspace.ui.iris.folder_run = Some(Running { stop: stop.clone() });
    let total = plan.items.len();
    progress(workspace, format!("Working on {} in {} ({}): 0 of {total} done. Say “stop” to stop after the photo I'm on.", photos(total), plan.folder.display(), plan.doing), true);
    let kit = Kit::of(workspace);
    let last = workspace.ui.settings.export;
    cx.spawn_in(window, async move |this, cx| {
        let mut finished: Vec<Done> = Vec::new();
        let mut written: Vec<PathBuf> = Vec::new();
        for (index, item) in plan.items.iter().enumerate() {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            let (item, job, kit, promised) = (item.clone(), plan.clone(), kit.clone(), written.clone());
            let done = cx.background_spawn(async move { work_on(&item, &job, &kit, last, &promised) }).await;
            written.extend(done.kept.iter().map(|(path, _)| path.clone()).chain(done.exported.iter().cloned()));
            finished.push(done);
            let alive = this.update_in(cx, |this, window, cx| {
                progress(this, format!("Working on {} in {} ({}): {} of {total} done.", photos(total), plan.folder.display(), plan.doing, index + 1), true);
                show_newest(window, cx);
            });
            if alive.is_err() {
                return;
            }
        }
        this.update_in(cx, |this, window, cx| {
            this.ui.iris.folder_run = None;
            if finished.iter().any(|done| !done.exported.is_empty()) {
                this.ui.settings.export_folder = Some(plan.exports_to.clone());
                let folder = plan.exports_to.clone();
                crate::app_settings::update(|settings| settings.export_folder = Some(folder));
            }
            progress(this, report(&plan, &finished, stop.load(Ordering::Relaxed)), false);
            super::conversation::keep_conversation(this);
            show_newest(window, cx);
        })
        .ok();
    })
    .detach();
    Outcome::default()
}

/// What a folder batch did, in words: how many photos, where their edits and files went, and
/// each photo that couldn't be done, with why.
fn report(plan: &Plan, finished: &[Done], stopped: bool) -> String {
    let total = plan.items.len();
    let worked: Vec<&Done> = finished.iter().filter(|done| done.failed.is_none()).collect();
    let mut text = if stopped && finished.len() < total {
        format!("Stopped after {} of {total}, as asked. The rest are as they were.", finished.len())
    } else {
        format!("Done: {} of {} in {}.", worked.len(), photos(total), plan.folder.display())
    };
    let new = finished.iter().filter(|done| done.kept.as_ref().is_some_and(|(_, new)| *new)).count();
    let saved = finished.iter().filter(|done| done.kept.as_ref().is_some_and(|(_, new)| !*new)).count();
    if new + saved > 0 {
        text.push_str(&format!("\n• Edits kept: {new} new project{} beside {}, {saved} project{} saved with the change.", if new == 1 { "" } else { "s" }, if new == 1 { "its photo" } else { "their photos" }, if saved == 1 { "" } else { "s" }));
    }
    let exported: usize = finished.iter().map(|done| done.exported.len()).sum();
    if exported > 0 {
        text.push_str(&format!("\n• Exported {exported} file{} to {}.", if exported == 1 { "" } else { "s" }, plan.exports_to.display()));
    }
    let failed: Vec<String> = finished.iter().filter_map(|done| Some(format!("• {}: {}", done.name, done.failed.as_ref()?))).collect();
    if !failed.is_empty() {
        text.push_str(&format!("\nCouldn't do {}:\n{}", if failed.len() == 1 { "this one".to_owned() } else { format!("these {}", failed.len()) }, failed.join("\n")));
    }
    text
}

/// Whether a message asks a running folder batch to stop.
pub fn asks_to_stop(message: &str) -> bool {
    matches!(message.trim().trim_end_matches(['.', '!']).to_lowercase().as_str(), "stop" | "stop it" | "cancel" | "halt" | "stop that" | "stop now" | "cancel it" | "stop the batch")
}

/// Stops a running folder batch after the photo it is working on.
pub fn stop(workspace: &mut Workspace) -> bool {
    let Some(running) = &workspace.ui.iris.folder_run else { return false };
    running.stop.store(true, Ordering::Relaxed);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_is_read_as_its_photos_and_projects() {
        let folder = std::env::temp_dir().join(format!("develop-folder-contents-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(folder.join("Inside")).unwrap();
        for name in ["b.jpg", "a.png", "c.develop", "c.jpg", "notes.txt", ".hidden.jpg", "Inside/deep.jpg", "open.jpg"] {
            std::fs::write(folder.join(name), b"").unwrap();
        }
        let found = contents(&folder, &[folder.join("open.jpg")]).unwrap();
        assert_eq!(found.items, vec![Item::Picture(folder.join("a.png")), Item::Picture(folder.join("b.jpg")), Item::Project(folder.join("c.develop"))]);
        assert_eq!((found.not_photos, found.with_projects, found.open), (1, 1, vec!["open.jpg".to_owned()]));
        assert_eq!(expanded("~/Pictures"), PathBuf::from(std::env::var("HOME").unwrap()).join("Pictures"));
        assert!(asks_to_stop("Stop!") && asks_to_stop("cancel") && !asks_to_stop("stop the vignette"));
        std::fs::remove_dir_all(&folder).unwrap();
    }
}

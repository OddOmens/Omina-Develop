//! Files in and out: Open (photos, or .develop projects), Save, Save As, Close and Quit
//! (offering to save first), and what shows on launch. Every photo opened gets a tab of its
//! own beside those already open, which stay as they are. Photos and projects are read on a
//! background thread, so a big file never freezes the window. Saving a photo, or closing it
//! without saving, also takes it out of the recovery copy of unsaved work (see `recovery`).

use std::path::{Path, PathBuf};

use develop_core::file::EXTENSION;
use develop_core::{Photo, Source};
use gpui_kit::{AppContext as _, Context, PromptButton, PromptLevel, Window};

use crate::app_settings;
use crate::workspace::Workspace;

/// Something to do once a save (or the decision not to) is done.
pub type Then = Box<dyn FnOnce(&mut Workspace, &mut Window, &mut Context<Workspace>)>;

/// Where file dialogs start: the project's folder, the last folder used, or Pictures.
fn starting_folder(workspace: &Workspace) -> PathBuf {
    workspace
        .state
        .file()
        .and_then(|file| file.parent().map(Path::to_path_buf))
        .or_else(|| workspace.ui.settings.last_folder.clone())
        .unwrap_or_else(|| std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join("Pictures"))
}

/// What happens when Develop starts (see `Workspace::launch`), with the files named on the
/// command line (`develop a.jpg b.jpg`). (Headless tests never launch, so they don't call it.)
#[cfg_attr(test, allow(dead_code))]
pub fn on_launch(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let named: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).filter(|path| path.exists()).collect();
    workspace.launch(named, window, cx);
}

pub fn show_error(window: &mut Window, cx: &mut Context<Workspace>, title: &'static str, error: String) {
    crate::panels::chrome::open_alert_dialog(window, cx, move |dialog, _, _| dialog.title(title).description(error.clone()));
}

/// Whether a path names a Develop project (rather than a photo).
fn is_project(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case(EXTENSION))
}

/// Reads a photo or a project into a photo. Returns it and whether it was a project (which
/// then has a file to save back to).
pub fn read_any(path: &Path) -> Result<(Photo, bool), String> {
    if is_project(path) {
        return develop_core::file::load(path).map(|photo| (photo, true));
    }
    if !develop_core::image_io::is_picture(path) {
        return Err("Omina Develop opens photos (JPEG, PNG, TIFF, WebP, GIF and BMP) and Omina Develop projects.".to_owned());
    }
    let bytes = std::fs::read(path).map_err(|error| format!("{} can't be read: {error}", path.display()))?;
    let name = path.file_name().map_or_else(|| "Photo".to_owned(), |name| name.to_string_lossy().into_owned());
    Ok((Photo::new(Source::decode(name, bytes)?), false))
}

impl Workspace {
    /// What shows at start-up: unsaved photos offered back if Develop closed without saving
    /// them, otherwise the usual launch (`launch_as_usual`).
    pub fn launch(&mut self, named: Vec<PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        // Asking about unsaved work comes first; the rest waits for the answer.
        if self.offer_recovery(named.clone(), window, cx) {
            return;
        }
        self.launch_as_usual(named, window, cx);
    }

    /// The start screen, with `named` files opening straight away, each in a tab, or else the
    /// last project if Settings asks for it. The very first time, the welcome setup shows over
    /// it all.
    pub fn launch_as_usual(&mut self, named: Vec<PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        crate::panels::start_screen::show(self, window, cx);
        if !self.ui.settings.onboarded {
            crate::panels::onboarding::show(self, cx);
        }
        if !named.is_empty() {
            self.open_paths(named, window, cx);
            return;
        }
        let settings = app_settings::load();
        if settings.start_with == app_settings::StartWith::LastProject
            && let Some(recent) = settings.recent_projects.first().filter(|recent| recent.path.exists())
        {
            self.open_path(recent.path.clone(), window, cx);
        }
    }

    pub fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.state.session().map(|session| session.id) {
            self.save_then(id, window, cx, None);
        }
    }

    pub fn save_as(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.state.session().map(|session| session.id) {
            self.save_as_then(id, window, cx, None);
        }
    }

    /// Saves the open photo with session `id` to its file (or asks where, the first time), then
    /// runs `then`. The photo is named by its session, not by which one is showing, since another
    /// can come to the front while a dialog is up (one finishing opening in the background).
    fn save_then(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>, then: Option<Then>) {
        let Some(index) = self.state.index_of(id) else { return };
        match self.state.sessions[index].file.clone() {
            Some(path) => {
                if self.write_project(id, &path, window, cx)
                    && let Some(then) = then
                {
                    then(self, window, cx);
                }
            }
            None => self.save_as_then(id, window, cx, then),
        }
    }

    fn save_as_then(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>, then: Option<Then>) {
        let Some(index) = self.state.index_of(id) else { return };
        let folder = starting_folder(self);
        let suggested = format!("{}.{EXTENSION}", self.state.sessions[index].photo.name);
        let chosen = cx.prompt_for_new_path(&folder, Some(&suggested));
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(mut path))) = chosen.await else { return };
            if !is_project(&path) {
                path.set_extension(EXTENSION);
            }
            this.update_in(cx, |this, window, cx| {
                if this.write_project(id, &path, window, cx)
                    && let Some(then) = then
                {
                    then(this, window, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Writes the open photo with session `id` to `path`, showing what went wrong if it couldn't.
    fn write_project(&mut self, id: u64, path: &Path, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.commit_edit(window, cx);
        // Recipes set to run before a save run on the photo being saved, when it is the one
        // showing (they work on the photo showing).
        if self.state.session().is_some_and(|session| session.id == id) {
            crate::panels::iris::run_trigger(self, ::iris::recipes::Trigger::BeforeSave, window, cx);
        }
        let Some(index) = self.state.index_of(id) else { return false };
        let photo = &self.state.sessions[index].photo;
        // The preview inside the file, for the start screen.
        let preview = match self.ui.preview.pyramid_of(photo) {
            Some(pyramid) => crate::preview::render_small(&pyramid, &photo.edit, develop_core::file::PREVIEW_SIDE),
            None => develop_core::file::preview_of(&photo.edit.render_full(&photo.source.bitmap)),
        };
        match develop_core::file::save(photo, &preview, path) {
            Ok(()) => {
                self.state.mark_saved(index, path.to_path_buf());
                // Saved: it has nothing left to recover.
                self.refresh_recovery_copy(window, cx);
                app_settings::note_project_used(path);
                if let Some(folder) = path.parent() {
                    let folder = folder.to_path_buf();
                    app_settings::update(|settings| settings.last_folder = Some(folder));
                }
                crate::workspace::refresh_menus(cx);
                cx.notify();
                true
            }
            Err(error) => {
                show_error(window, cx, "The project couldn't be saved.", error);
                false
            }
        }
    }

    /// File ▸ Open: photos or Develop projects, each in a tab of its own.
    pub fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let chosen = cx.prompt_for_paths(gpui_kit::PathPromptOptions { files: true, directories: false, multiple: true, prompt: Some("Open".into()) });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = chosen.await else { return };
            this.update_in(cx, |this, window, cx| this.open_paths(paths, window, cx)).ok();
        })
        .detach();
    }

    /// Opens several files, each in a tab of its own.
    pub fn open_paths(&mut self, paths: Vec<PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        for path in paths {
            self.open_path(path, window, cx);
        }
    }

    /// Opens a file in a new tab, leaving the photos already open as they are. A project that
    /// is already open is shown instead of being opened twice. The file is read and decoded off
    /// the UI thread.
    pub fn open_path(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(index) = self.state.find_file(&path) {
            self.commit_edit(window, cx);
            self.state.activate(index);
            self.ui.start_screen = None;
            crate::panels::viewer::set_zoom(self, None);
            cx.notify();
            return;
        }
        cx.spawn_in(window, async move |this, cx| {
            let reading = path.clone();
            let read = cx.background_spawn(async move { read_any(&reading) }).await;
            this.update_in(cx, |this, window, cx| this.finish_opening(path, read, window, cx)).ok();
        })
        .detach();
    }

    /// Shows a photo that finished reading in a new tab, or what went wrong.
    fn finish_opening(&mut self, path: PathBuf, read: Result<(Photo, bool), String>, window: &mut Window, cx: &mut Context<Self>) {
        match read {
            Ok((photo, project)) => {
                // Opened twice while the first was still reading: show the one already open.
                if project && let Some(index) = self.state.find_file(&path) {
                    self.state.activate(index);
                } else {
                    self.commit_edit(window, cx);
                    let file = project.then(|| path.clone());
                    self.state.open(photo, file, if project { "Open" } else { "Import" });
                    if let Some(session) = self.state.sessions.last_mut() {
                        session.origin = Some(path.clone());
                    }
                }
                self.ui.start_screen = None;
                crate::panels::viewer::set_zoom(self, None);
                if project {
                    app_settings::note_project_used(&path);
                    crate::workspace::refresh_menus(cx);
                }
                // Recipes set to run when a photo opens, and (for a photo file brought in, rather
                // than a project) after an import.
                crate::panels::iris::run_trigger(self, ::iris::recipes::Trigger::OnOpen, window, cx);
                if !project {
                    crate::panels::iris::run_trigger(self, ::iris::recipes::Trigger::AfterImport, window, cx);
                }
                if let Some(folder) = path.parent() {
                    let folder = folder.to_path_buf();
                    self.ui.settings.last_folder = Some(folder.clone());
                    app_settings::update(|settings| settings.last_folder = Some(folder));
                }
            }
            Err(error) => {
                if !path.exists() {
                    app_settings::forget_recent(&path);
                    crate::workspace::refresh_menus(cx);
                }
                show_error(window, cx, "This file couldn't be opened.", error);
            }
        }
        cx.notify();
    }

    /// File ▸ Close: closes the photo showing, offering to save it first.
    pub fn close_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let index = self.state.active;
        self.close_photo(index, window, cx);
    }

    /// Closes open photo `index`, offering to save it first. The other photos stay open; with
    /// none left, the start screen shows.
    pub fn close_photo(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.state.sessions.get(index).map(|session| session.id) else { return };
        self.commit_edit(window, cx);
        if !self.state.is_unsaved(index) {
            self.discard_photo(id, window, cx);
            return;
        }
        // The question is about the photo on screen, so it is shown first.
        self.state.activate(index);
        cx.notify();
        let close: Then = Box::new(move |this, window, cx| this.discard_photo(id, window, cx));
        let close_too: Then = Box::new(move |this, window, cx| this.discard_photo(id, window, cx));
        self.ask_to_save(window, cx, close, close_too);
    }

    /// Closes the open photo with session `id` without asking, freeing its previews.
    fn discard_photo(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.state.sessions.iter().position(|session| session.id == id) else { return };
        let unsaved = self.state.is_unsaved(index);
        if let Some(closed) = self.state.close(index) {
            for image in self.ui.preview.release(crate::preview::source_id(&closed.photo)) {
                cx.drop_image(image, Some(window));
            }
        }
        crate::panels::viewer::set_zoom(self, None);
        // Thrown away on purpose: it has nothing left to recover.
        if unsaved {
            self.refresh_recovery_copy(window, cx);
        }
        if self.state.sessions.is_empty() {
            crate::panels::start_screen::show(self, window, cx);
        }
        cx.notify();
    }

    /// Quits, offering to save each photo with unsaved changes first, one at a time.
    /// Cancelling any of the questions stops the quit.
    pub fn quit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_edit(window, cx);
        let Some(index) = self.state.first_unsaved() else {
            self.remember_window(window);
            cx.quit();
            return;
        };
        let id = self.state.sessions[index].id;
        self.state.activate(index);
        cx.notify();
        // Saved, the photo no longer counts as unsaved; let go, it is closed so it isn't asked
        // about again. Either way the next unsaved one is asked about.
        let after_save: Then = Box::new(|this, window, cx| this.quit(window, cx));
        let after_discard: Then = Box::new(move |this, window, cx| {
            this.discard_photo(id, window, cx);
            this.quit(window, cx);
        });
        self.ask_to_save(window, cx, after_save, after_discard);
    }

    /// Remembers where the window is for next time.
    pub fn remember_window(&self, window: &Window) {
        let bounds = window.bounds();
        let placement = app_settings::WindowPlacement {
            x: f32::from(bounds.origin.x),
            y: f32::from(bounds.origin.y),
            width: f32::from(bounds.size.width),
            height: f32::from(bounds.size.height),
            maximized: window.is_maximized(),
        };
        app_settings::update(|settings| settings.window = Some(placement));
    }

    /// Asks whether to save the photo showing (Save, Don't Save, Cancel). Save saves it and
    /// then runs `after_save`; Don't Save runs `discard`; Cancel does nothing.
    fn ask_to_save(&mut self, window: &mut Window, cx: &mut Context<Self>, after_save: Then, discard: Then) {
        let Some(id) = self.state.session().map(|session| session.id) else { return };
        let message = format!("Save changes to “{}”?", self.state.project_name());
        let buttons = [PromptButton::ok("Save"), PromptButton::new("Don’t Save"), PromptButton::cancel("Cancel")];
        let answer = window.prompt(PromptLevel::Warning, &message, Some("Your changes will be lost if you don’t save them."), &buttons, cx);
        cx.spawn_in(window, async move |this, cx| {
            let Ok(choice) = answer.await else { return };
            this.update_in(cx, |this, window, cx| match choice {
                0 => this.save_then(id, window, cx, Some(after_save)),
                1 => discard(this, window, cx),
                _ => {}
            })
            .ok();
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn photos_and_projects_open_and_other_files_are_refused() {
        let folder = std::env::temp_dir().join(format!("develop-open-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let photo_path = folder.join("Dunes.png");
        let bitmap = develop_core::Bitmap::filled(12, 8, develop_core::Rgba::rgb(0.8, 0.6, 0.4));
        std::fs::write(&photo_path, develop_core::image_io::encode(&bitmap, develop_core::image_io::ExportFormat::Png, 100, [255; 3]).unwrap()).unwrap();
        let (photo, project) = read_any(&photo_path).unwrap();
        assert!(!project);
        assert_eq!(photo.name, "Dunes");
        let project_path = folder.join("Dunes.develop");
        develop_core::file::save(&photo, &bitmap, &project_path).unwrap();
        let (again, project) = read_any(&project_path).unwrap();
        assert!(project && again.name == "Dunes");
        let text = folder.join("notes.txt");
        std::fs::write(&text, "hello").unwrap();
        assert!(read_any(&text).unwrap_err().contains("opens photos"));
        std::fs::remove_dir_all(&folder).unwrap();
    }
}

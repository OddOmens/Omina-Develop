//! Export (⇧⌘E): the finished photo, developed from the original at full resolution, as JPEG,
//! PNG, TIFF, WebP or BMP, at full size or shrunk to a long edge, and (for JPEG) at a chosen
//! quality. The rendering runs off the UI thread; the top bar says so while it does.
//!
//! The sheet asks where each file goes. Exports can also be written without it (`write_all`,
//! which Iris uses for "export as JPEG" and "export all"): several photos at once, into the
//! folder the last export went to, never over a file that is already there.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use develop_core::image_io::{ExportFormat, encode, write_atomically};
use develop_core::{Bitmap, Edit, Source};
use gpui_kit::component::menu::DropdownMenu as _;
use gpui_kit::component::{Sizable as _, WindowExt as _};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dialog::{DialogAction, DialogClose, DialogFooter};
use gpui_kit::{AnyElement, AppContext as _, Context, InteractiveElement, IntoElement, ParentElement, StatefulInteractiveElement, Styled, TestSupportExt as _, Window, div, prelude::FluentBuilder as _, px};

use crate::app_settings::{ExportChoices, ExportSize};
use crate::theme::{color, hsla, size};
use crate::workspace::Workspace;

/// The long edges offered besides the full size.
const LONG_EDGES: [u32; 4] = [1080, 2048, 3840, 6000];
/// The JPEG qualities offered.
const QUALITIES: [u8; 5] = [60, 75, 85, 90, 100];

/// The size an export comes out at, for a finished photo of `width` × `height`. A long edge
/// never enlarges the photo.
pub fn exported_size(width: u32, height: u32, size: ExportSize) -> (u32, u32) {
    match size {
        ExportSize::Full => (width, height),
        ExportSize::LongEdge(edge) => {
            let longest = width.max(height).max(1);
            if edge >= longest {
                return (width, height);
            }
            let scale = edge as f32 / longest as f32;
            (((width as f32 * scale).round() as u32).max(1), ((height as f32 * scale).round() as u32).max(1))
        }
    }
}

/// Develops the photo at full resolution and encodes it as `choices` say.
fn render_file(source: &Source, edit: &Edit, choices: ExportChoices) -> Result<Vec<u8>, String> {
    let finished: Bitmap = edit.render_full(&source.bitmap);
    let (width, height) = exported_size(finished.width, finished.height, choices.size);
    let sized = if (width, height) == (finished.width, finished.height) { finished } else { finished.resized(width, height) };
    encode(&sized, choices.format, choices.quality, [255, 255, 255])
}

/// Asks where to save, then renders and writes the file in the background.
fn save_file(workspace: &mut Workspace, choices: ExportChoices, window: &mut Window, cx: &mut Context<Workspace>) {
    // Recipes set to run before an export run first; what they change is in what is written.
    crate::panels::iris::run_trigger(workspace, ::iris::recipes::Trigger::BeforeExport, window, cx);
    let Some(photo) = workspace.state.photo() else { return };
    let folder = workspace
        .ui
        .settings
        .last_folder
        .clone()
        .or_else(|| workspace.state.file().and_then(|file| file.parent().map(PathBuf::from)))
        .unwrap_or_else(|| std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join("Pictures"));
    let suggested = format!("{}.{}", photo.name, choices.format.extension());
    let (source, edit) = (photo.source.clone(), photo.edit.clone());
    let chosen = cx.prompt_for_new_path(&folder, Some(&suggested));
    cx.spawn_in(window, async move |this, cx| {
        let Ok(Ok(Some(mut path))) = chosen.await else { return };
        if path.extension().is_none() {
            path.set_extension(choices.format.extension());
        }
        let name = path.file_name().map_or_else(String::new, |name| name.to_string_lossy().into_owned());
        this.update(cx, |this, cx| {
            this.ui.exporting = Some(format!("Exporting {name}…"));
            cx.notify();
        })
        .ok();
        let writing = path.clone();
        let written = cx.background_spawn(async move { render_file(&source, &edit, choices).and_then(|bytes| write_atomically(&writing, &bytes)) }).await;
        this.update_in(cx, |this, window, cx| {
            this.ui.exporting = None;
            match written {
                Ok(()) => {
                    window.push_notification(format!("Exported {name}.").as_str(), cx);
                    if let Some(parent) = path.parent() {
                        let parent = parent.to_path_buf();
                        this.ui.settings.last_folder = Some(parent.clone());
                        this.ui.settings.export_folder = Some(parent.clone());
                        crate::app_settings::update(|settings| {
                            settings.last_folder = Some(parent.clone());
                            settings.export_folder = Some(parent);
                        });
                    }
                }
                Err(error) => super::chrome::open_alert_dialog(window, cx, move |dialog, _, _| dialog.title("The photo couldn't be exported.").description(error.clone())),
            }
            cx.notify();
        })
        .ok();
    })
    .detach();
}

/// A size an export preset asks for. Unlike Export's own choices, a preset may ask for a
/// width ("Instagram": 1080 wide), which is turned into the long edge for each photo.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PresetSize {
    Full,
    LongEdge(u32),
    Wide(u32),
}

/// The export choices an export preset keeps (see `iris::export_presets`, which keeps them as
/// this app writes them).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PresetChoices {
    pub format: ExportFormat,
    pub quality: u8,
    pub size: PresetSize,
}

impl PresetChoices {
    /// The choices Export is set to now, kept as a preset.
    pub fn from_choices(choices: ExportChoices) -> Self {
        let size = match choices.size {
            ExportSize::Full => PresetSize::Full,
            ExportSize::LongEdge(edge) => PresetSize::LongEdge(edge),
        };
        Self { format: choices.format, quality: choices.quality, size }
    }

    /// The preset as a request to export straight away, which Iris's export makes for each photo.
    pub fn ask(&self) -> develop_core::chat::ExportAsk {
        use develop_core::chat::SizeAsk;
        let size = match self.size {
            PresetSize::Full => SizeAsk::Full,
            PresetSize::LongEdge(edge) => SizeAsk::Long(edge),
            PresetSize::Wide(wide) => SizeAsk::Wide(wide),
        };
        develop_core::chat::ExportAsk { formats: vec![self.format], quality: Some(self.quality), size: Some(size), all: false, now: true }
    }

    /// The choices for a photo whose finished size is `width` × `height`, as the sheet takes them.
    pub fn for_photo(&self, (width, height): (u32, u32)) -> ExportChoices {
        let size = match self.size {
            PresetSize::Full => ExportSize::Full,
            PresetSize::LongEdge(edge) => ExportSize::LongEdge(edge),
            PresetSize::Wide(wide) if width >= height => ExportSize::LongEdge(wide),
            PresetSize::Wide(wide) => ExportSize::LongEdge((wide as f32 * height as f32 / width.max(1) as f32).round() as u32),
        };
        ExportChoices { format: self.format, quality: self.quality, size }
    }

    /// The choices in words: "JPEG at quality 80, 2048 pixels on the long edge".
    pub fn described(&self) -> String {
        let size = match self.size {
            PresetSize::Full => "full size".to_owned(),
            PresetSize::LongEdge(edge) => format!("{edge} pixels on the long edge"),
            PresetSize::Wide(wide) => format!("{wide} pixels wide"),
        };
        if self.format.has_quality() { format!("{} at quality {}, {size}", self.format.name(), self.quality) } else { format!("{}, {size}", self.format.name()) }
    }

    /// Anything to know about what the preset doesn't do.
    pub fn note(&self) -> Option<&'static str> {
        matches!(self.size, PresetSize::Wide(1080)).then_some("this is Instagram's width only: crop to 4:5 or 1:1 first if you want its shape")
    }
}

/// The export presets Develop comes with, each a real combination of Export's choices.
pub fn built_in_presets() -> Vec<::iris::export_presets::Preset> {
    let preset = |name: &str, format: ExportFormat, quality: u8, size: PresetSize| ::iris::export_presets::Preset {
        name: name.to_owned(),
        settings: serde_json::to_value(PresetChoices { format, quality, size }).unwrap_or_default(),
    };
    vec![
        preset("Web", ExportFormat::Jpeg, 80, PresetSize::LongEdge(2048)),
        preset("Instagram", ExportFormat::Jpeg, 90, PresetSize::Wide(1080)),
        preset("Print", ExportFormat::Tiff, 100, PresetSize::Full),
        preset("Email", ExportFormat::Jpeg, 70, PresetSize::LongEdge(1200)),
    ]
}

/// The export presets saved in Develop, from Iris's shared folder.
fn saved_presets() -> Vec<::iris::export_presets::Preset> {
    crate::panels::iris::shared_folder().map(|folder| ::iris::export_presets::load(&folder, ::iris::App::Develop)).unwrap_or_default()
}

/// Every export preset: the saved ones, then the ones Develop comes with that no saved one
/// replaces.
pub fn all_presets() -> Vec<::iris::export_presets::Preset> {
    let saved = saved_presets();
    let own: Vec<_> = built_in_presets().into_iter().filter(|preset| !saved.iter().any(|kept| kept.name.eq_ignore_ascii_case(&preset.name))).collect();
    saved.into_iter().chain(own).collect()
}

/// A preset's choices, if they can be read.
pub fn preset_choices(preset: &::iris::export_presets::Preset) -> Option<PresetChoices> {
    serde_json::from_value(preset.settings.clone()).ok()
}

/// Keeps `choices` as the export preset `name`, replacing one of that name. Returns the
/// choices kept, or why they couldn't be.
pub fn save_preset(name: &str, choices: PresetChoices) -> Result<PresetChoices, String> {
    let folder = crate::panels::iris::shared_folder().ok_or_else(|| "There's no folder to keep export presets in.".to_owned())?;
    let mut saved = saved_presets();
    ::iris::export_presets::keep(&mut saved, ::iris::export_presets::Preset { name: name.to_owned(), settings: serde_json::to_value(choices).map_err(|error| error.to_string())? })?;
    ::iris::export_presets::save(&folder, ::iris::App::Develop, &saved).map_err(|error| format!("The export preset couldn't be saved: {error}"))?;
    Ok(choices)
}

/// Forgets the saved export preset `name`, and says what came of it.
pub fn forget_preset(name: &str) -> String {
    let Some(folder) = crate::panels::iris::shared_folder() else { return "There are no saved export presets.".to_owned() };
    let mut saved = saved_presets();
    let before = saved.len();
    saved.retain(|preset| !preset.name.eq_ignore_ascii_case(name));
    if saved.len() == before {
        return if built_in_presets().iter().any(|preset| preset.name.eq_ignore_ascii_case(name)) { format!("“{name}” is a preset Omina Develop comes with, so it can't be forgotten.") } else { format!("There's no saved export preset called “{name}”.") };
    }
    match ::iris::export_presets::save(&folder, ::iris::App::Develop, &saved) {
        Ok(()) => format!("Forgotten: the export preset “{name}”."),
        Err(error) => format!("The export presets couldn't be saved: {error}"),
    }
}

/// One file to export: a photo as it is edited now, and how to write it.
pub struct Job {
    /// The photo's name, which the file is named after.
    pub name: String,
    pub source: Arc<Source>,
    pub edit: Edit,
    pub choices: ExportChoices,
}

/// What came of exporting several files.
pub struct Written {
    /// The folder they went to.
    pub folder: PathBuf,
    /// The files written, in the order asked for.
    pub files: Vec<PathBuf>,
    /// The photos that couldn't be exported, by name, with what went wrong.
    pub failed: Vec<(String, String)>,
}

/// A name fit to be a file's: a photo's name without the characters a path gives meaning to.
fn file_stem(name: &str) -> String {
    let stem: String = name.trim().chars().map(|character| if matches!(character, '/' | '\\' | ':') { '-' } else { character }).collect();
    if stem.is_empty() { "Photo".to_owned() } else { stem }
}

/// A path in `folder` for a file named after `name` that nothing has yet: the name itself, or
/// with a number added ("Beach 2.jpg"). `promised` are paths already given to earlier files of
/// the same export, which aren't on disk yet.
pub fn unused_path(folder: &Path, name: &str, extension: &str, promised: &[PathBuf]) -> PathBuf {
    let stem = file_stem(name);
    let free = |path: &PathBuf| !path.exists() && !promised.contains(path);
    std::iter::once(folder.join(format!("{stem}.{extension}"))).chain((2..).map(|number| folder.join(format!("{stem} {number}.{extension}")))).find(free).unwrap_or_else(|| folder.join(format!("{stem}.{extension}")))
}

/// Where exports go when nobody is asked: the folder the last export was written to, else an
/// "Exports" folder beside the showing photo's project file. `None` when there is neither (a
/// photo never saved, and nothing exported before), which is when the sheet must ask.
pub fn folder_without_asking(workspace: &Workspace) -> Option<PathBuf> {
    workspace.ui.settings.export_folder.clone().or_else(|| workspace.state.file().and_then(|file| file.parent()).map(|folder| folder.join("Exports")))
}

/// Develops and writes every job into `folder`, one after another.
fn write_jobs(jobs: Vec<Job>, folder: PathBuf) -> Written {
    let mut written = Written { folder, files: Vec::new(), failed: Vec::new() };
    if let Err(error) = std::fs::create_dir_all(&written.folder) {
        let why = format!("{} can't be made: {error}", written.folder.display());
        written.failed = jobs.into_iter().map(|job| (job.name, why.clone())).collect();
        return written;
    }
    for job in jobs {
        match write_one(&job, &written.folder, &written.files) {
            Ok(path) => written.files.push(path),
            Err(error) => written.failed.push((job.name, error)),
        }
    }
    written
}

/// Develops and writes one job into `folder` (which must exist), under a name nothing has yet
/// and none of `promised` either. Returns where it went.
pub fn write_one(job: &Job, folder: &Path, promised: &[PathBuf]) -> Result<PathBuf, String> {
    let path = unused_path(folder, &job.name, job.choices.format.extension(), promised);
    render_file(&job.source, &job.edit, job.choices).and_then(|bytes| write_atomically(&path, &bytes))?;
    Ok(path)
}

/// Exports `jobs` into `folder` off the UI thread, without asking where: each under its
/// photo's name, never over a file already there. The top bar says so while it runs; when it
/// is done the folder is remembered for next time and `done` is told what was written.
pub fn write_all(workspace: &mut Workspace, jobs: Vec<Job>, folder: PathBuf, window: &mut Window, cx: &mut Context<Workspace>, done: impl FnOnce(&mut Workspace, &Written, &mut Window, &mut Context<Workspace>) + 'static) {
    workspace.ui.exporting = Some(if jobs.len() == 1 { format!("Exporting {}…", jobs[0].name) } else { format!("Exporting {} files…", jobs.len()) });
    cx.spawn_in(window, async move |this, cx| {
        let written = cx.background_spawn(async move { write_jobs(jobs, folder) }).await;
        this.update_in(cx, |this, window, cx| {
            this.ui.exporting = None;
            if !written.files.is_empty() {
                let folder = written.folder.clone();
                this.ui.settings.export_folder = Some(folder.clone());
                crate::app_settings::update(|settings| settings.export_folder = Some(folder));
            }
            done(this, &written, window, cx);
            cx.notify();
        })
        .ok();
    })
    .detach();
}

/// A note floating over the bottom of the window while an export is written.
pub fn progress(what: String) -> AnyElement {
    div()
        .absolute()
        .bottom(px(18.0))
        .left_0()
        .right_0()
        .flex()
        .justify_center()
        .child(super::chrome::popup_surface(div().px_4().py_2().flex().items_center().gap_2().text_size(px(size::TEXT))).child(gpui_kit::component::Icon::new(gpui_kit::assets::IconName::Download).size(px(14.0))).child(what))
        .into_any_element()
}

/// File ▸ Export…, starting with the choices used last time.
pub fn open(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let choices = workspace.ui.settings.export;
    open_with(workspace, choices, window, cx);
}

/// The Export sheet starting with `choices` made (Iris opens it this way for "export as JPEG
/// at 80% quality"). Nothing is remembered or written until Export… is pressed.
pub fn open_with(workspace: &mut Workspace, choices: ExportChoices, window: &mut Window, cx: &mut Context<Workspace>) {
    let Some(photo) = workspace.state.photo() else { return };
    let (width, height) = photo.edit.output_size(photo.source.size());
    let choices = Rc::new(Cell::new(choices));
    let target = cx.weak_entity();
    // The name a new export preset is saved under.
    let preset_name = cx.new(|cx| gpui_kit::component::input::InputState::new(window, cx).placeholder("Preset name"));
    super::chrome::open_dialog(window, cx, move |dialog, _, _| {
        let current = choices.get();
        // A size or quality that isn't one of the usual ones (from a preset, say) gets a chip
        // of its own, so the sheet shows what it will use.
        let mut edges = LONG_EDGES.to_vec();
        if let ExportSize::LongEdge(edge) = current.size
            && !edges.contains(&edge)
        {
            edges.push(edge);
            edges.sort_unstable();
        }
        let mut quality_steps = QUALITIES.to_vec();
        if !quality_steps.contains(&current.quality) {
            quality_steps.push(current.quality);
            quality_steps.sort_unstable();
        }
        // The presets: one picked sets the choices below for this photo.
        let presets = {
            let choices = choices.clone();
            Button::new("export-preset").outline().small().label("Choose…").dropdown_caret(true).dropdown_menu(move |menu, _, _| {
                all_presets().into_iter().filter_map(|preset| Some((preset.name.clone(), preset_choices(&preset)?))).fold(menu, |menu, (name, picked)| {
                    let choices = choices.clone();
                    menu.item(gpui_kit::component::menu::PopupMenuItem::new(format!("{name}: {}", picked.described())).on_click(move |_, window, _| {
                        choices.set(picked.for_photo((width, height)));
                        window.refresh();
                    }))
                })
            })
        };
        let save_preset_button = {
            let (choices, preset_name) = (choices.clone(), preset_name.clone());
            Button::new("export-save-preset").ghost().small().label("Save as Preset…").on_click(move |_, window, cx| {
                let name = preset_name.read(cx).value().trim().to_owned();
                if name.is_empty() {
                    window.push_notification("Type a name for the preset first.", cx);
                    return;
                }
                let note = match save_preset(&name, PresetChoices::from_choices(choices.get())) {
                    Ok(kept) => format!("Saved the export preset “{name}”: {}.", kept.described()),
                    Err(why) => why,
                };
                window.push_notification(note.as_str(), cx);
                window.refresh();
            })
        };
        let chip = |id: String, label: String, picked: bool, set: Rc<dyn Fn()>| {
            div()
                .id(gpui_kit::ElementId::Name(id.into()))
                .px_2()
                .py_1()
                .rounded(px(4.0))
                .cursor_pointer()
                .when(picked, |chip| chip.bg(hsla(color::ROW_SELECTED)).text_color(hsla(color::TEXT)))
                .when(!picked, |chip| chip.bg(hsla(color::RAISED)).text_color(hsla(color::MUTED)))
                .on_click(move |_, window, _| {
                    set();
                    window.refresh();
                })
                .child(label)
                .test_support()
        };
        let formats = ExportFormat::ALL.into_iter().map(|format| {
            let choices = choices.clone();
            chip(format!("export-format-{}", format.extension()), format.name().to_owned(), current.format == format, Rc::new(move || choices.set(ExportChoices { format, ..choices.get() })))
        });
        let sizes = std::iter::once(ExportSize::Full).chain(edges.into_iter().map(ExportSize::LongEdge)).map(|size| {
            let choices = choices.clone();
            let label = match size {
                ExportSize::Full => "Full size".to_owned(),
                ExportSize::LongEdge(edge) => format!("{edge} px"),
            };
            chip(format!("export-size-{label}"), label, current.size == size, Rc::new(move || choices.set(ExportChoices { size, ..choices.get() })))
        });
        let qualities = quality_steps.into_iter().map(|quality| {
            let choices = choices.clone();
            chip(format!("export-quality-{quality}"), quality.to_string(), current.quality == quality, Rc::new(move || choices.set(ExportChoices { quality, ..choices.get() })))
        });
        let (out_width, out_height) = exported_size(width, height, current.size);
        let size_line = format!("{out_width} × {out_height} pixels, developed from the original");
        let (choices, target) = (choices.clone(), target.clone());
        let row = |label: &str, body: gpui_kit::Div| div().flex().items_center().gap_3().child(div().w(px(90.0)).flex_none().text_color(hsla(color::MUTED)).child(label.to_owned())).child(body);
        dialog
            .title("Export")
            .w(px(560.0))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .text_size(px(size::TEXT))
                    .child(row("Preset", div().flex().items_center().gap_1p5().child(presets).child(div().w(px(150.0)).child(gpui_kit::component::input::Input::new(&preset_name).small())).child(save_preset_button)))
                    .child(row("Format", div().flex().flex_wrap().gap_1().children(formats)))
                    .child(row("Size", div().flex().flex_wrap().gap_1().children(sizes)))
                    .when(current.format.has_quality(), |body| body.child(row("Quality", div().flex().gap_1().children(qualities))))
                    .child(div().text_color(hsla(color::MUTED)).child(size_line)),
            )
            .footer(
                DialogFooter::new()
                    .child(DialogClose::new().trigger(|button| button.outline().label("Cancel")))
                    .child(DialogAction::new().child(Button::new("export-ok").primary().label("Export…"))),
            )
            .on_ok(move |_, window, cx| {
                let choices = choices.get();
                crate::app_settings::update(|settings| settings.export = choices);
                target
                    .update(cx, |this, cx| {
                        this.ui.settings.export = choices;
                        save_file(this, choices, window, cx);
                    })
                    .ok();
                true
            })
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_edges_shrink_but_never_enlarge() {
        assert_eq!(exported_size(6000, 4000, ExportSize::Full), (6000, 4000));
        assert_eq!(exported_size(6000, 4000, ExportSize::LongEdge(2048)), (2048, 1365));
        assert_eq!(exported_size(4000, 6000, ExportSize::LongEdge(1080)), (720, 1080));
        assert_eq!(exported_size(800, 600, ExportSize::LongEdge(2048)), (800, 600));
    }

    #[test]
    fn files_are_named_after_their_photos_and_never_over_one_that_is_there() {
        let folder = std::env::temp_dir().join(format!("develop-export-names-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        let photo = crate::state::test_support::photo();
        let job = |name: &str, format: ExportFormat| Job { name: name.to_owned(), source: photo.source.clone(), edit: Edit::default(), choices: ExportChoices { format, quality: 80, size: ExportSize::LongEdge(50) } };
        // Two photos of one name, and one whose name has a slash in it.
        let written = write_jobs(vec![job("Beach", ExportFormat::Jpeg), job("Beach", ExportFormat::Jpeg), job("Beach", ExportFormat::Png), job("Day 1/2", ExportFormat::Jpeg)], folder.clone());
        let names: Vec<String> = written.files.iter().map(|path| path.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, vec!["Beach.jpg", "Beach 2.jpg", "Beach.png", "Day 1-2.jpg"]);
        assert!(written.failed.is_empty() && written.files.iter().all(|path| path.exists()));
        let back = develop_core::image_io::decode(&std::fs::read(&written.files[0]).unwrap()).unwrap();
        assert_eq!((back.width, back.height), (50, 25), "sized as asked");
        // Exported again, the first files are left exactly as they were.
        let first = std::fs::read(&written.files[0]).unwrap();
        let again = write_jobs(vec![job("Beach", ExportFormat::Jpeg)], folder.clone());
        assert_eq!(again.files, vec![folder.join("Beach 3.jpg")]);
        assert_eq!(std::fs::read(&written.files[0]).unwrap(), first);
        // A folder that can't be made fails every photo by name instead of losing them.
        let blocked = write_jobs(vec![job("Beach", ExportFormat::Jpeg)], written.files[0].join("inside-a-file"));
        assert!(blocked.files.is_empty() && blocked.failed.len() == 1 && blocked.failed[0].0 == "Beach");
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn exports_are_developed_from_the_original() {
        let photo = crate::state::test_support::photo();
        let mut edit = Edit::default();
        edit.crop.rect = Some(develop_core::Rect::new(0.0, 0.0, 100.0, 50.0));
        edit.develop.exposure = 1.0;
        let bytes = render_file(&photo.source, &edit, ExportChoices { format: ExportFormat::Png, quality: 90, size: ExportSize::Full }).unwrap();
        let back = develop_core::image_io::decode(&bytes).unwrap();
        assert_eq!((back.width, back.height), (100, 50));
        assert!(back.pixel(50, 25)[2] > photo.source.bitmap.pixel(50, 25)[2], "brighter than the original");
    }
}

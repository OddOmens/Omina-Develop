//! What each menu command and shortcut does: the window's action handlers, kept apart from
//! `workspace` so the layout code stays readable.

use gpui_kit::{Context, Div, InteractiveElement as _, Window};

use crate::panels;
use crate::state::{Compare, EditorState, MaskKind, Tool};
use crate::workspace::*;

/// Runs a state change, shows any notices it left, and redraws.
fn act(this: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>, step: impl FnOnce(&mut EditorState)) {
    use gpui_kit::component::WindowExt as _;
    this.commit_edit(window, cx);
    step(&mut this.state);
    for notice in std::mem::take(&mut this.state.notices) {
        window.push_notification(notice.as_str(), cx);
    }
    cx.notify();
}

/// Whether command `id` can be used now: its feature is on (Settings ▸ Tools & Features). A
/// turned-off feature's menu items and keys are gone already; this also stops it from Iris's
/// "Do It" and any other way the command could still be sent.
fn allowed(this: &Workspace, id: &str) -> bool {
    this.ui.settings.command_on(id)
}

/// The action a command id (as `keymap` and Iris's shared help name them) stands for, so a
/// help answer can offer to do what it describes. `None` for ids Develop doesn't have.
pub fn action_for(id: &str) -> Option<Box<dyn gpui_kit::Action>> {
    Some(match id {
        "view-tool" => Box::new(ViewTool),
        "crop-tool" => Box::new(CropTool),
        "white-balance-tool" => Box::new(WhiteBalanceTool),
        "linear-mask-tool" => Box::new(LinearMaskTool),
        "radial-mask-tool" => Box::new(RadialMaskTool),
        "before" => Box::new(ToggleBefore),
        "split" => Box::new(ToggleSplit),
        "clipping" => Box::new(ToggleClipping),
        "auto" => Box::new(Auto),
        "reset-all" => Box::new(ResetAll),
        "undo" => Box::new(Undo),
        "redo" => Box::new(Redo),
        "open" | "import" => Box::new(Open),
        "save" => Box::new(Save),
        "save-as" => Box::new(SaveAs),
        "close" => Box::new(CloseProject),
        "export" => Box::new(Export),
        "copy-settings" => Box::new(CopySettings),
        "paste-settings" => Box::new(PasteSettings),
        "new-snapshot" => Box::new(NewSnapshot),
        "new-preset" => Box::new(NewPreset),
        "rotate-left" => Box::new(RotateLeft),
        "rotate-right" => Box::new(RotateRight),
        "delete-mask" => Box::new(DeleteMask),
        "zoom-in" => Box::new(ZoomIn),
        "zoom-out" => Box::new(ZoomOut),
        "fit" => Box::new(FitOnScreen),
        "actual-size" => Box::new(ActualSize),
        "toggle-zoom" => Box::new(ToggleZoom),
        "settings" => Box::new(OpenSettings),
        "next-photo" => Box::new(NextPhoto),
        "previous-photo" => Box::new(PreviousPhoto),
        // Commands with no key of their own, which only the menus and Iris reach.
        "start-screen" => Box::new(ShowStartScreen),
        "about" => Box::new(About),
        "full-screen" => Box::new(ToggleFullScreen),
        "reset-workspace" => Box::new(ResetWorkspace),
        "level" => Box::new(LevelPhoto),
        _ => return None,
    })
}

/// A command as it can be asked for by name: "zoom in", "open settings", "white balance
/// selector".
pub struct Named {
    /// Its id, as `action_for` and `keymap` know it.
    pub id: &'static str,
    /// The name its menu or button shows.
    pub name: &'static str,
    /// Other words people ask for it with.
    pub keywords: &'static str,
    /// Whether it does nothing until a photo is open.
    pub needs_photo: bool,
    /// Whether it opens a dialog that waits for an answer, so whatever was asked for after it
    /// can't simply carry on.
    pub opens: bool,
}

const fn named(id: &'static str, name: &'static str, keywords: &'static str, needs_photo: bool, opens: bool) -> Named {
    Named { id, name, keywords, needs_photo, opens }
}

/// The commands Iris runs when asked for by name. Edits, the crop, comparing, undo and the
/// other open photos are not here: she reads those herself, with what they should do. Left out
/// on purpose: quitting and hiding the app, which nobody should trigger by a slip of a word,
/// and Delete Mask, which a stray "mask" would otherwise find.
pub const NAMED: &[Named] = &[
    named("open", "Open…", "import load add photo photos file files picture", false, true),
    named("save", "Save", "project", true, true),
    named("save-as", "Save As…", "copy project", true, true),
    named("close", "Close Photo", "tab", true, true),
    named("start-screen", "Start Screen", "home welcome recent", false, false),
    named("view-tool", "Hand", "pan move drag", true, false),
    named("white-balance-tool", "White Balance Selector", "eyedropper dropper picker neutral wb", true, false),
    named("linear-mask-tool", "Linear Gradient Mask", "graduated filter add", true, false),
    named("radial-mask-tool", "Radial Gradient Mask", "ellipse circle spot add", true, false),
    named("clipping", "Show Clipping", "warning clipped hide off on", true, false),
    named("zoom-in", "Zoom In", "closer magnify", true, false),
    named("zoom-out", "Zoom Out", "further", true, false),
    named("fit", "Fit on Screen", "zoom whole window", true, false),
    named("actual-size", "100%", "zoom actual size percent pixels full", true, false),
    named("show-presets", "Presets", "looks list", false, false),
    named("show-snapshots", "Snapshots", "versions list", false, false),
    named("show-history", "History", "steps list", false, false),
    named("settings", "Settings…", "preferences options", false, true),
    named("about", "About Omina Develop", "version", false, true),
    named("full-screen", "Enter Full Screen", "fullscreen leave exit", false, false),
    named("reset-workspace", "Reset Workspace", "layout panels", false, false),
];

/// The longest side of the copy Auto and Level measure, in pixels.
const AUTO_SIDE: u32 = 512;

/// A small copy of `photo`'s original, turned and flipped as it is shown, and how much smaller
/// it is. Made from the original, so it works for a photo that has never been shown (and off
/// the UI thread).
fn small_oriented(photo: &develop_core::Photo) -> (develop_core::Bitmap, f32) {
    let (width, height) = photo.source.size();
    let scale = (AUTO_SIDE as f32 / width.max(height).max(1) as f32).min(1.0);
    let small = photo.source.bitmap.resized(((width as f32 * scale).round() as u32).max(1), ((height as f32 * scale).round() as u32).max(1));
    (photo.edit.crop.orient(&small), scale)
}

/// What Auto measures for `photo`: a small copy of it as it came, cut to its crop, made from
/// the original.
pub fn auto_image_of(photo: &develop_core::Photo) -> develop_core::Bitmap {
    let edit = develop_core::Edit { crop: photo.edit.crop.clone(), ..Default::default() };
    let (small, scale) = small_oriented(photo);
    edit.render(&small, photo.image_size(), scale, develop_core::View::Before)
}

/// The straighten angle that would level `photo`, measured from its original (see
/// `Workspace::level_angle_of`).
pub fn level_angle_from_original(photo: &develop_core::Photo) -> Option<f32> {
    develop_core::straighten::level_angle(&small_oriented(photo).0)
}

/// What Level did.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Leveled {
    /// Straightened to this angle, in degrees.
    To(f32),
    /// The photo was level already (or straightened to the angle found), so nothing changed.
    Already,
    /// No line in the photo was clear enough to level by.
    Unclear,
}

impl Workspace {
    /// A small copy of `photo` as it came, cut to its crop, for Auto to measure: from the
    /// preview's ready-made copies when it has them, else shrunk from the original (which a
    /// photo not yet shown has no preview for).
    pub fn auto_image(&self, photo: &develop_core::Photo) -> develop_core::Bitmap {
        let edit = develop_core::Edit { crop: photo.edit.crop.clone(), ..Default::default() };
        if let Some(pyramid) = self.ui.preview.pyramid_of(photo) {
            let level = pyramid.level_at_least(AUTO_SIDE);
            return edit.render(&pyramid.levels[level], pyramid.size, pyramid.scale(level), develop_core::View::Before);
        }
        auto_image_of(photo)
    }

    /// The straighten angle that would level `photo`, measured from the lines in a small copy
    /// of it as it came (turned and flipped as shown, but not straightened). `None` when no
    /// line close to level or plumb is clear enough to go by.
    pub fn level_angle_of(&self, photo: &develop_core::Photo) -> Option<f32> {
        if let Some(pyramid) = self.ui.preview.pyramid_of(photo) {
            return develop_core::straighten::level_angle(&pyramid.levels[pyramid.level_at_least(AUTO_SIDE)]);
        }
        level_angle_from_original(photo)
    }

    /// Crop & Straighten's Level: straightens the photo showing by its own horizon or
    /// uprights, when they are clear. `None` with no photo open.
    pub fn level(&mut self) -> Option<Leveled> {
        let photo = self.state.photo()?;
        let Some(angle) = self.level_angle_of(photo) else { return Some(Leveled::Unclear) };
        if (angle - photo.edit.crop.angle).abs() < 0.05 {
            return Some(Leveled::Already);
        }
        self.state.level(angle);
        Some(Leveled::To(angle))
    }

    /// Lightroom's Auto: the light set from the photo as it came, measured on a small copy of
    /// its crop.
    pub fn auto(&mut self) {
        let Some(photo) = self.state.photo() else { return };
        let small = self.auto_image(photo);
        self.state.auto_tone(&small);
    }
}

/// While the welcome setup shows: only quitting (which still asks about unsaved photos) and the
/// window's own commands. Escape does nothing, so the setup can't be skipped by accident.
pub fn bind_during_setup(root: Div, cx: &mut Context<Workspace>) -> Div {
    root.on_action(cx.listener(|this, _: &Quit, window, cx| this.quit(window, cx)))
        .on_action(cx.listener(|_, _: &MinimizeWindow, window, _| window.minimize_window()))
        .on_action(cx.listener(|_, _: &ZoomWindow, window, _| window.zoom_window()))
        .on_action(cx.listener(|_, _: &ToggleFullScreen, window, _| window.toggle_fullscreen()))
}

/// Attaches every command's handler to the window's root element.
pub fn bind(root: Div, cx: &mut Context<Workspace>) -> Div {
    root
        // Tools.
        .on_action(cx.listener(|this, _: &ViewTool, window, cx| act(this, window, cx, |state| state.set_tool(Tool::View))))
        .on_action(cx.listener(|this, _: &CropTool, window, cx| {
            if !allowed(this, "crop-tool") {
                return;
            }
            // Pressing R again finishes cropping, as in Lightroom.
            let next = if this.state.tool == Tool::Crop { Tool::View } else { Tool::Crop };
            act(this, window, cx, |state| state.set_tool(next));
        }))
        .on_action(cx.listener(|this, _: &WhiteBalanceTool, window, cx| {
            if allowed(this, "white-balance-tool") {
                act(this, window, cx, |state| state.set_tool(Tool::WhiteBalance));
            }
        }))
        .on_action(cx.listener(|this, _: &LinearMaskTool, window, cx| {
            if !allowed(this, "linear-mask-tool") {
                return;
            }
            act(this, window, cx, |state| state.use_mask_tool(MaskKind::Linear));
            this.reveal(PanelId::Develop);
        }))
        .on_action(cx.listener(|this, _: &RadialMaskTool, window, cx| {
            if !allowed(this, "radial-mask-tool") {
                return;
            }
            act(this, window, cx, |state| state.use_mask_tool(MaskKind::Radial));
            this.reveal(PanelId::Develop);
        }))
        .on_action(cx.listener(|this, _: &ToggleBefore, window, cx| {
            if allowed(this, "before") {
                act(this, window, cx, EditorState::toggle_before);
            }
        }))
        .on_action(cx.listener(|this, _: &ToggleSplit, window, cx| {
            if allowed(this, "split") {
                act(this, window, cx, EditorState::toggle_split);
            }
        }))
        .on_action(cx.listener(|this, _: &AskIris, _, cx| {
            if panels::iris::is_on(this) {
                this.show_side(LeftPanel::Iris);
                panels::iris::choose_tab(this, true);
                cx.notify();
            }
        }))
        .on_action(cx.listener(|this, _: &ShowIrisSuggestions, _, cx| {
            if panels::iris::is_on(this) {
                this.show_side(LeftPanel::Iris);
                panels::iris::choose_tab(this, false);
                cx.notify();
            }
        }))
        .on_action(cx.listener(|this, _: &ToggleClipping, _, cx| {
            if !allowed(this, "clipping") {
                return;
            }
            this.ui.show_clipping = !this.ui.show_clipping;
            cx.notify();
        }))
        .on_action(cx.listener(|this, _: &DeleteMask, window, cx| {
            if this.state.tool == Tool::Masks
                && allowed(this, "delete-mask")
                && let Some(index) = this.state.selected_mask
            {
                act(this, window, cx, |state| state.delete_mask(index));
            }
        }))
        // Enter finishes cropping; Esc leaves the tool, or the start screen.
        .on_action(cx.listener(|this, _: &Commit, window, cx| {
            if matches!(this.state.tool, Tool::Crop | Tool::Masks | Tool::WhiteBalance) {
                act(this, window, cx, |state| state.set_tool(Tool::View));
            }
        }))
        .on_action(cx.listener(|this, _: &Dismiss, window, cx| {
            if this.ui.start_screen.is_some() {
                panels::start_screen::hide(this);
                cx.notify();
            } else if this.state.compare != Compare::Off {
                act(this, window, cx, |state| state.compare = Compare::Off);
            } else {
                act(this, window, cx, |state| state.set_tool(Tool::View));
            }
        }))
        // Settings.
        .on_action(cx.listener(|this, _: &Auto, window, cx| {
            this.commit_edit(window, cx);
            this.auto();
            cx.notify();
        }))
        .on_action(cx.listener(|this, _: &ResetAll, window, cx| act(this, window, cx, EditorState::reset_all)))
        .on_action(cx.listener(|this, _: &ResetDevelop, window, cx| act(this, window, cx, EditorState::reset_develop)))
        .on_action(cx.listener(|this, _: &ResetCrop, window, cx| {
            if allowed(this, "reset-crop") {
                act(this, window, cx, EditorState::reset_crop);
            }
        }))
        // A recipe's own key runs it, as its name would in Iris's chat.
        .on_action(cx.listener(|this, action: &RunRecipe, window, cx| {
            this.commit_edit(window, cx);
            panels::iris::run_recipe(this, &action.name, window, cx);
            cx.notify();
        }))
        .on_action(cx.listener(|this, _: &LevelPhoto, window, cx| {
            if !allowed(this, "level") {
                return;
            }
            this.commit_edit(window, cx);
            // The menu and the crop bar have no reply to give, so what happened is a notice.
            match this.level() {
                Some(Leveled::Unclear) => this.state.notice("No clear horizon or upright edge to level by. Drag outside the crop box to straighten it by eye."),
                Some(Leveled::Already) => this.state.notice("The photo already looks level."),
                Some(Leveled::To(_)) | None => {}
            }
            cx.notify();
        }))
        .on_action(cx.listener(|this, _: &CopySettings, window, cx| {
            if !allowed(this, "copy-settings") {
                return;
            }
            act(this, window, cx, EditorState::copy_settings);
            if this.state.copied.is_some() {
                use gpui_kit::component::WindowExt as _;
                window.push_notification("Settings copied. Open another photo and Paste Settings (⇧⌘V) to give it the same look.", cx);
            }
        }))
        .on_action(cx.listener(|this, _: &PasteSettings, window, cx| {
            if !allowed(this, "paste-settings") {
                return;
            }
            act(this, window, cx, |state| {
                if !state.paste_settings() && state.photo().is_some() {
                    state.notice("There are no copied settings yet. Use Copy Settings (⇧⌘C) on a photo first.");
                }
            });
        }))
        .on_action(cx.listener(|this, _: &NewSnapshot, window, cx| {
            if !allowed(this, "new-snapshot") {
                return;
            }
            act(this, window, cx, |state| state.add_snapshot(None));
            this.show_side(LeftPanel::Snapshots);
        }))
        .on_action(cx.listener(|this, _: &NewPreset, window, cx| {
            if allowed(this, "new-preset") {
                panels::dialogs::new_preset(this, window, cx);
            }
        }))
        // Photo.
        .on_action(cx.listener(|this, _: &RotateLeft, window, cx| act(this, window, cx, |state| state.rotate(false))))
        .on_action(cx.listener(|this, _: &RotateRight, window, cx| act(this, window, cx, |state| state.rotate(true))))
        .on_action(cx.listener(|this, _: &FlipHorizontal, window, cx| act(this, window, cx, |state| state.flip(true))))
        .on_action(cx.listener(|this, _: &FlipVertical, window, cx| act(this, window, cx, |state| state.flip(false))))
        // Edit.
        .on_action(cx.listener(|this, _: &Undo, window, cx| act(this, window, cx, |state| {
            state.undo();
        })))
        .on_action(cx.listener(|this, _: &Redo, window, cx| act(this, window, cx, |state| {
            state.redo();
        })))
        // Files.
        .on_action(cx.listener(|this, _: &Open, window, cx| this.open(window, cx)))
        .on_action(cx.listener(|this, action: &OpenRecent, window, cx| this.open_path(action.path.clone(), window, cx)))
        .on_action(cx.listener(|this, _: &Save, window, cx| this.save(window, cx)))
        .on_action(cx.listener(|this, _: &SaveAs, window, cx| this.save_as(window, cx)))
        .on_action(cx.listener(|this, _: &CloseProject, window, cx| this.close_project(window, cx)))
        .on_action(cx.listener(|this, _: &Export, window, cx| panels::export::open(this, window, cx)))
        .on_action(cx.listener(|this, _: &ShowStartScreen, window, cx| panels::start_screen::go_to(this, window, cx)))
        .on_action(cx.listener(|_, _: &ClearRecentProjects, _, cx| {
            crate::app_settings::update(|settings| settings.recent_projects.clear());
            refresh_menus(cx);
            cx.notify();
        }))
        .on_action(cx.listener(|this, _: &Quit, window, cx| this.quit(window, cx)))
        .on_action(cx.listener(|this, _: &NextPhoto, window, cx| {
            act(this, window, cx, |state| state.activate_next(true));
            panels::viewer::set_zoom(this, None);
        }))
        .on_action(cx.listener(|this, _: &PreviousPhoto, window, cx| {
            act(this, window, cx, |state| state.activate_next(false));
            panels::viewer::set_zoom(this, None);
        }))
        // View.
        .on_action(cx.listener(|this, _: &ZoomIn, _, cx| {
            this.zoom_step(true);
            this.notify_canvas(cx);
        }))
        .on_action(cx.listener(|this, _: &ZoomOut, _, cx| {
            this.zoom_step(false);
            this.notify_canvas(cx);
        }))
        .on_action(cx.listener(|this, _: &FitOnScreen, _, cx| {
            panels::viewer::set_zoom(this, None);
            this.notify_canvas(cx);
        }))
        .on_action(cx.listener(|this, _: &ActualSize, _, cx| {
            panels::viewer::set_zoom(this, Some(1.0));
            this.notify_canvas(cx);
        }))
        .on_action(cx.listener(|this, _: &ToggleZoom, _, cx| {
            panels::viewer::toggle_zoom(this);
            this.notify_canvas(cx);
        }))
        // The app and its window.
        .on_action(cx.listener(|this, _: &OpenSettings, window, cx| panels::settings::open(this, window, cx)))
        .on_action(cx.listener(|this, _: &About, window, cx| panels::dialogs::about(this, window, cx)))
        .on_action(cx.listener(|_, _: &MinimizeWindow, window, _| window.minimize_window()))
        .on_action(cx.listener(|_, _: &ZoomWindow, window, _| window.zoom_window()))
        .on_action(cx.listener(|_, _: &ToggleFullScreen, window, _| window.toggle_fullscreen()))
        .on_action(cx.listener(|this, _: &ResetWorkspace, window, cx| this.reset_workspace(window, cx)))
        .on_action(cx.listener(|this, action: &TogglePanel, window, cx| {
            if let Some(id) = panels::dock::HIDEABLE.get(action.panel) {
                this.toggle_panel(*id, window, cx);
            }
        }))
}

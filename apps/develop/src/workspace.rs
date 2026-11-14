//! The editor window: owns the editor state, wires the menu bar and keyboard shortcuts, and
//! lays out the panels as Studio and Canvas do: a slim top bar of tools, a rail down the left
//! that picks the side panel (Presets, Snapshots, History) and gets photos in and out, and
//! movable panels in a dock (the side panel, the photo, and the Develop panel on the right).

use std::collections::HashSet;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::dock::{DockArea, DockAreaState, DockEvent};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, DragMoveEvent, Entity, FocusHandle, Global, InteractiveElement, IntoElement, KeyBinding, ParentElement, Render, StyleRefinement,
    Styled, Subscription, Window, actions, div, prelude::FluentBuilder as _, px,
};

use crate::keymap::binding;
use crate::panels;
use crate::panels::fields::{FieldDrag, FieldTarget};
use crate::panels::views::{PanelKind, PanelView};
use crate::state::EditorState;
use crate::theme::{color, hsla};

actions!(develop, [
    ViewTool,
    CropTool,
    WhiteBalanceTool,
    LinearMaskTool,
    RadialMaskTool,
    ToggleBefore,
    ToggleSplit,
    Auto,
    ResetAll,
    ResetDevelop,
    ResetCrop,
    LevelPhoto,
    Open,
    Save,
    SaveAs,
    CloseProject,
    Export,
    ShowStartScreen,
    ClearRecentProjects,
    Quit,
    Undo,
    Redo,
    CopySettings,
    PasteSettings,
    NewSnapshot,
    NewPreset,
    RotateLeft,
    RotateRight,
    FlipHorizontal,
    FlipVertical,
    DeleteMask,
    Commit,
    Dismiss,
    ZoomIn,
    ZoomOut,
    FitOnScreen,
    ActualSize,
    ToggleZoom,
    OpenSettings,
    About,
    HideDevelop,
    HideOtherApps,
    ShowAllApps,
    MinimizeWindow,
    ZoomWindow,
    ToggleFullScreen,
    ResetWorkspace,
    NextPhoto,
    PreviousPhoto,
    ToggleClipping,
    AskIris,
    ShowIrisSuggestions
]);

/// Window ▸ (a panel's name): shows or hides that panel (by its place in `dock::HIDEABLE`).
#[derive(Clone, PartialEq, gpui_kit::Action)]
#[action(namespace = develop, no_json)]
pub struct TogglePanel {
    pub panel: usize,
}

/// Runs one of Iris's recipes by name: what a recipe's own keyboard shortcut does.
#[derive(Clone, PartialEq, gpui_kit::Action)]
#[action(namespace = develop, no_json)]
pub struct RunRecipe {
    pub name: String,
}

/// File ▸ Open Recent.
#[derive(Clone, PartialEq, gpui_kit::Action)]
#[action(namespace = develop, no_json)]
pub struct OpenRecent {
    pub path: std::path::PathBuf,
}

/// How often Develop looks for photos Omina Edit or Omina Motion have sent.
#[cfg(not(test))]
const HANDOFF_EVERY: std::time::Duration = std::time::Duration::from_secs(3);

/// Key context the editor's shortcuts are bound in.
pub(crate) const KEY_CONTEXT: &str = "Develop";
/// Where plain-key shortcuts (letters, arrows) apply: anywhere except while a text field has
/// focus, so typing "r" into a field doesn't pick Crop.
pub(crate) const NOT_TYPING: &str = "Develop && !Input";

/// Every editor shortcut with the id of its command (`keymap::SHORTCUTS`; "" for keys with no
/// command id, which are always on), so the ones whose feature is turned off can be left out.
/// Each command is bound to its key now (the person's own, if changed); one left without a key
/// isn't bound.
fn editor_bindings(cx: &App) -> Vec<(&'static str, KeyBinding)> {
    let plain = Some(NOT_TYPING);
    let editor = Some(KEY_CONTEXT);
    let predicate = |context: Option<&str>| context.and_then(|context| gpui_kit::KeyBindingContextPredicate::parse(context).ok()).map(std::rc::Rc::new);
    let key = |id: &'static str, action: Box<dyn gpui_kit::Action>, context: Option<&str>| binding(id).and_then(|key| KeyBinding::load(&key, action, predicate(context), false, None, cx).ok()).map(|binding| (id, binding));
    // Letters and other plain keys only apply while no text field has focus.
    let mut bindings: Vec<(&'static str, KeyBinding)> = [
        key("view-tool", Box::new(ViewTool), plain),
        key("crop-tool", Box::new(CropTool), plain),
        key("white-balance-tool", Box::new(WhiteBalanceTool), plain),
        key("linear-mask-tool", Box::new(LinearMaskTool), plain),
        key("radial-mask-tool", Box::new(RadialMaskTool), plain),
        key("before", Box::new(ToggleBefore), plain),
        key("split", Box::new(ToggleSplit), plain),
        key("toggle-zoom", Box::new(ToggleZoom), plain),
        key("clipping", Box::new(ToggleClipping), plain),
        key("delete-mask", Box::new(DeleteMask), plain),
        // Commands with ⌘ work everywhere in the editor; undo is left to a text field while one has focus.
        key("undo", Box::new(Undo), plain),
        key("redo", Box::new(Redo), plain),
        key("auto", Box::new(Auto), editor),
        key("reset-all", Box::new(ResetAll), editor),
        key("open", Box::new(Open), editor),
        key("import", Box::new(Open), editor),
        key("save", Box::new(Save), editor),
        key("save-as", Box::new(SaveAs), editor),
        key("close", Box::new(CloseProject), editor),
        key("export", Box::new(Export), editor),
        key("copy-settings", Box::new(CopySettings), editor),
        key("paste-settings", Box::new(PasteSettings), editor),
        key("new-snapshot", Box::new(NewSnapshot), editor),
        key("new-preset", Box::new(NewPreset), editor),
        key("rotate-left", Box::new(RotateLeft), editor),
        key("rotate-right", Box::new(RotateRight), editor),
        key("zoom-in", Box::new(ZoomIn), editor),
        key("zoom-out", Box::new(ZoomOut), editor),
        key("fit", Box::new(FitOnScreen), editor),
        key("actual-size", Box::new(ActualSize), editor),
        key("settings", Box::new(OpenSettings), editor),
        key("next-photo", Box::new(NextPhoto), editor),
        key("previous-photo", Box::new(PreviousPhoto), editor),
    ]
    .into_iter()
    .flatten()
    .collect();
    // Delete Mask's second key, and the keys that are always the same.
    if binding("delete-mask").as_deref() == Some("backspace") {
        bindings.push(("delete-mask", KeyBinding::new("delete", DeleteMask, plain)));
    }
    bindings.extend([
        ("", KeyBinding::new("enter", Commit, plain)),
        ("", KeyBinding::new("escape", Dismiss, editor)),
        ("", KeyBinding::new("cmd-q", Quit, None)),
        ("", KeyBinding::new("cmd-h", HideDevelop, None)),
        ("", KeyBinding::new("cmd-alt-h", HideOtherApps, None)),
        ("", KeyBinding::new("cmd-m", MinimizeWindow, None)),
        ("", KeyBinding::new("ctrl-cmd-f", ToggleFullScreen, None)),
    ]);
    // The recipes Iris has been given keys for: each runs its recipe, as its name would.
    for (key, name) in crate::keymap::recipe_keys() {
        if let Ok(binding) = KeyBinding::load(&key, Box::new(RunRecipe { name }), predicate(editor), false, None, cx) {
            bindings.push(("", binding));
        }
    }
    bindings
}

/// The bindings GPUI's own components made at start-up (text fields, menus, dialogs), kept so
/// the keymap can be rebuilt around them when features are turned on or off.
struct BaseBindings(Vec<KeyBinding>);

impl Global for BaseBindings {}

/// Binds the editor's shortcuts and the app-wide commands. Call once, after `gpui_kit::init`.
pub fn bind_keys(cx: &mut App) {
    let base = cx.key_bindings().borrow().bindings().cloned().collect();
    cx.set_global(BaseBindings(base));
    apply_keys(&crate::app_settings::load(), cx);
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.on_action(|_: &HideDevelop, cx| cx.hide());
    cx.on_action(|_: &HideOtherApps, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAllApps, cx| cx.unhide_other_apps());
}

/// Rebuilds the keymap: the components' bindings, then every editor shortcut whose feature is
/// on (Settings ▸ Tools & Features), so a turned-off feature's key does nothing.
pub fn apply_keys(settings: &crate::app_settings::AppSettings, cx: &mut App) {
    crate::keymap::use_shortcuts(&settings.shortcuts);
    let base = cx.try_global::<BaseBindings>().map(|base| base.0.clone()).unwrap_or_default();
    cx.clear_key_bindings();
    cx.bind_keys(base);
    let bindings = editor_bindings(cx);
    cx.bind_keys(bindings.into_iter().filter(|(id, _)| settings.command_on(id)).map(|(_, binding)| binding));
}

/// A menu's items without those whose command is turned off (each paired with its command id;
/// "" is always on), and without the separators that would leave doubled or at either end.
fn menu_items(settings: &crate::app_settings::AppSettings, items: Vec<(&str, gpui_kit::MenuItem)>) -> Vec<gpui_kit::MenuItem> {
    use gpui_kit::MenuItem;
    let mut kept: Vec<MenuItem> = Vec::new();
    for (id, item) in items {
        if !settings.command_on(id) {
            continue;
        }
        let separator = matches!(item, MenuItem::Separator);
        if separator && kept.last().is_none_or(|last| matches!(last, MenuItem::Separator)) {
            continue;
        }
        kept.push(item);
    }
    if kept.last().is_some_and(|last| matches!(last, MenuItem::Separator)) {
        kept.pop();
    }
    kept
}

/// Rebuilds the menu bar (File ▸ Open Recent changes).
pub fn refresh_menus(cx: &mut App) {
    cx.set_menus(menus());
}

/// The panels the settings say are hidden.
fn hidden_panels_in(settings: &crate::app_settings::AppSettings) -> HashSet<PanelId> {
    settings.hidden_panels.iter().filter_map(|panel| PanelId::from_name(&panel.name)).filter(|id| *id != PanelId::Canvas).collect()
}

/// The menu bar, laid out as Lightroom's Develop module's.
pub fn menus() -> Vec<gpui_kit::Menu> {
    use gpui_kit::{Menu, MenuItem, OsAction, SystemMenuType};
    let settings = crate::app_settings::load();
    let recent: Vec<MenuItem> = settings
        .recent_projects
        .iter()
        .filter(|project| project.path.exists())
        .take(10)
        .map(|project| {
            let name = project.path.file_stem().map_or_else(|| project.path.display().to_string(), |name| name.to_string_lossy().into_owned());
            MenuItem::action(name, OpenRecent { path: project.path.clone() })
        })
        .collect();
    let mut open_recent = Menu::new("Open Recent").items(recent);
    open_recent.items.extend([MenuItem::separator(), MenuItem::action("Clear Menu", ClearRecentProjects)]);
    vec![
        Menu::new("Omina Develop").items([
            MenuItem::action("About Omina Develop", About),
            MenuItem::separator(),
            MenuItem::action("Settings…", OpenSettings),
            MenuItem::separator(),
            MenuItem::os_submenu("Services", SystemMenuType::Services),
            MenuItem::separator(),
            MenuItem::action("Hide Omina Develop", HideDevelop),
            MenuItem::action("Hide Others", HideOtherApps),
            MenuItem::action("Show All", ShowAllApps),
            MenuItem::separator(),
            MenuItem::action("Quit Omina Develop", Quit),
        ]),
        Menu::new("File").items([
            MenuItem::action("Open…", Open),
            MenuItem::submenu(open_recent),
            MenuItem::action("Close Photo", CloseProject),
            MenuItem::action("Start Screen", ShowStartScreen),
            MenuItem::separator(),
            MenuItem::action("Save", Save),
            MenuItem::action("Save As…", SaveAs),
            MenuItem::separator(),
            MenuItem::action("Export…", Export),
        ]),
        Menu::new("Edit").items([
            MenuItem::os_action("Undo", Undo, OsAction::Undo),
            MenuItem::os_action("Redo", Redo, OsAction::Redo),
        ]),
        Menu::new("Settings").items(menu_items(&settings, vec![
            ("copy-settings", MenuItem::action("Copy Settings", CopySettings)),
            ("paste-settings", MenuItem::action("Paste Settings", PasteSettings)),
            ("", MenuItem::separator()),
            ("", MenuItem::action("Auto", Auto)),
            ("", MenuItem::separator()),
            ("new-snapshot", MenuItem::action("New Snapshot", NewSnapshot)),
            ("new-preset", MenuItem::action("New Preset…", NewPreset)),
            ("", MenuItem::separator()),
            ("", MenuItem::action("Reset Develop Settings", ResetDevelop)),
            ("reset-crop", MenuItem::action("Reset Crop", ResetCrop)),
            ("", MenuItem::action("Reset All", ResetAll)),
        ])),
        Menu::new("Photo").items(menu_items(&settings, vec![
            ("", MenuItem::action("Rotate Left", RotateLeft)),
            ("", MenuItem::action("Rotate Right", RotateRight)),
            ("", MenuItem::separator()),
            ("", MenuItem::action("Flip Horizontal", FlipHorizontal)),
            ("", MenuItem::action("Flip Vertical", FlipVertical)),
            // Level belongs to Crop & Straighten, and goes with it when that is turned off.
            ("level", MenuItem::separator()),
            ("level", MenuItem::action("Level", LevelPhoto)),
        ])),
        Menu::new("Tools").items(menu_items(&settings, vec![
            ("", MenuItem::action("Hand", ViewTool)),
            ("crop-tool", MenuItem::action("Crop & Straighten", CropTool)),
            ("white-balance-tool", MenuItem::action("White Balance Selector", WhiteBalanceTool)),
            ("", MenuItem::separator()),
            ("linear-mask-tool", MenuItem::action("Linear Gradient Mask", LinearMaskTool)),
            ("radial-mask-tool", MenuItem::action("Radial Gradient Mask", RadialMaskTool)),
            ("delete-mask", MenuItem::action("Delete Mask", DeleteMask)),
        ])),
        Menu::new("View").items(menu_items(&settings, vec![
            ("before", MenuItem::action("Before", ToggleBefore)),
            ("split", MenuItem::action("Before / After Split", ToggleSplit)),
            ("clipping", MenuItem::action("Show Clipping", ToggleClipping)),
            ("", MenuItem::separator()),
            ("", MenuItem::action("Zoom In", ZoomIn)),
            ("", MenuItem::action("Zoom Out", ZoomOut)),
            ("", MenuItem::action("Fit on Screen", FitOnScreen)),
            ("", MenuItem::action("100%", ActualSize)),
        ])),
        Menu::new("Window").items(
            [
                MenuItem::action("Minimize", MinimizeWindow),
                MenuItem::action("Zoom", ZoomWindow),
                MenuItem::separator(),
                MenuItem::action("Enter Full Screen", ToggleFullScreen),
                MenuItem::separator(),
                MenuItem::action("Next Photo", NextPhoto),
                MenuItem::action("Previous Photo", PreviousPhoto),
                MenuItem::separator(),
            ]
            .into_iter()
            // A panel per line, checked while it is showing; picking one shows or hides it.
            .chain(panels::dock::HIDEABLE.into_iter().enumerate().map(|(index, id)| {
                let hidden = settings.hidden_panels.iter().any(|panel| panel.name == id.name());
                MenuItem::action(id.menu_name(), TogglePanel { panel: index }).checked(!hidden)
            }))
            .chain([MenuItem::separator(), MenuItem::action("Reset Workspace", ResetWorkspace)])
            .collect::<Vec<_>>(),
        ),
        Menu::new("Help").items(if settings.iris_on {
            vec![MenuItem::action("Ask Iris…", AskIris), MenuItem::action("Iris's Suggestions", ShowIrisSuggestions)]
        } else {
            vec![MenuItem::action("About Omina Develop", About)]
        }),
    ]
}

/// What the side panel beside the rail shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeftPanel {
    /// Built-in looks and your own.
    Presets,
    /// Saved copies of the photo's edit.
    Snapshots,
    /// Every change, to step back to.
    History,
    /// Iris, the assistant: what it notices in the photo, and a chat.
    Iris,
}

/// The movable panels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PanelId {
    /// The panel beside the rail: Presets, Snapshots or History, whichever the rail picked.
    Side,
    Canvas,
    Develop,
}

/// A text field open over something (a number, a name).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EditTarget {
    Number(FieldTarget),
    PhotoName,
    SnapshotName(usize),
    MaskName(usize),
}

pub struct EditingField {
    pub target: EditTarget,
    pub input: Entity<InputState>,
    pub initial: String,
    _events: Subscription,
    _changes: Subscription,
}

/// UI-only state: not part of the photo.
pub struct UiState {
    pub active_panel: PanelId,
    pub left_panel: LeftPanel,
    /// The rail button under the pointer and the number of that hover, the count of hovers so
    /// far, and the button whose name label is showing (after a short rest on it).
    pub rail_hover: Option<(&'static str, u64)>,
    pub rail_hovers: u64,
    pub rail_label: Option<&'static str>,
    /// The tab each panel with tabs is showing.
    pub tabs: std::collections::HashMap<PanelId, usize>,
    pub reveal_panels: Vec<PanelId>,
    pub showing_panels: HashSet<PanelId>,
    /// Panels taken off screen with Hide Panel or the Window menu. Where each was is kept in
    /// the app settings, so showing it puts it back there.
    pub hidden_panels: HashSet<PanelId>,
    pub layout_resetting: bool,
    pub layout_save_countdown: u8,
    pub panel_menu: Option<panels::dock::PanelMenu>,
    /// Cards folded or unfolded from how they start.
    pub toggled_sections: HashSet<String>,
    pub viewer: panels::viewer::ViewerUi,
    pub preview: crate::preview::Preview,
    pub editing: Option<EditingField>,
    /// Where a value field's scrub started: pointer x and the value then.
    pub field_anchor: Option<(f32, f32)>,
    pub settings: crate::app_settings::AppSettings,
    pub start_screen: Option<panels::start_screen::StartScreen>,
    pub recent_previews: panels::start_screen::RecentPreviews,
    /// Slider tracks' bounds as drawn last, for dragging along them.
    pub slider_bounds: std::cell::RefCell<std::collections::HashMap<String, panels::chrome::MeasuredBounds>>,
    pub slider_pressed: bool,
    pub window_title: String,
    pub curve_channel: usize,
    pub mixer_tab: usize,
    /// Small pictures of the photo with each preset, for the Presets panel.
    pub preset_pictures: panels::presets::PresetPictures,
    /// An export being written, shown in the top bar until it is done.
    pub exporting: Option<String>,
    /// Iris's findings, conversation and message field.
    pub iris: panels::iris::IrisUi,
    /// An edit to show in place of the photo's own while a preset or a fix is hovered, with the
    /// photo (its original, see `preview::source_id`) it is for.
    pub hover_edit: Option<(usize, develop_core::Edit)>,
    /// Whether clipped highlights and shadows are painted red and blue on the photo.
    pub show_clipping: bool,
    /// The welcome setup's page, while it shows (see `panels::onboarding`).
    pub onboarding: Option<panels::onboarding::Step>,
    /// The recovery copy of unsaved photos: whether it is being offered back or written.
    pub recovery: crate::recovery::Recovery,
}

impl UiState {
    pub fn tab(&self, panel: PanelId) -> usize {
        self.tabs.get(&panel).copied().unwrap_or(0)
    }

    /// Whether a card is open: cards start open unless `closed_by_default`, and a click flips them.
    pub fn section_open(&self, key: &str, closed_by_default: bool) -> bool {
        self.toggled_sections.contains(key) == closed_by_default
    }

    pub fn toggle_section(&mut self, key: &str) {
        if !self.toggled_sections.remove(key) {
            self.toggled_sections.insert(key.to_owned());
        }
    }

    /// What the side panel shows: the panel the rail picked, or, when that one's feature is
    /// turned off, Presets, or History (which is always on).
    pub fn side_panel(&self) -> LeftPanel {
        [self.left_panel, LeftPanel::Presets].into_iter().find(|panel| self.settings.panel_on(*panel)).unwrap_or(LeftPanel::History)
    }
}

pub struct Workspace {
    pub state: EditorState,
    pub ui: UiState,
    pub(crate) focus: FocusHandle,
    panels: Panels,
    dock: Entity<DockArea>,
    _dock_events: Subscription,
}

/// The cached panel views.
struct Panels {
    toolbar: Entity<PanelView>,
    side: Entity<PanelView>,
    canvas: Entity<PanelView>,
    develop: Entity<PanelView>,
}

impl Panels {
    fn new(focus: &FocusHandle, cx: &mut Context<Workspace>) -> Self {
        let workspace = cx.entity();
        let mut view = |kind| cx.new(|cx| PanelView::new(&workspace, kind, focus.clone(), cx));
        Self {
            toolbar: view(PanelKind::Toolbar),
            side: view(PanelKind::Side),
            canvas: view(PanelKind::Canvas),
            develop: view(PanelKind::Develop),
        }
    }

    fn view(&self, id: PanelId) -> &Entity<PanelView> {
        match id {
            PanelId::Side => &self.side,
            PanelId::Canvas => &self.canvas,
            PanelId::Develop => &self.develop,
        }
    }
}

/// Embeds a panel view, reusing last frame's drawing unless the panel was notified.
fn cached(view: &Entity<PanelView>) -> impl IntoElement + use<> {
    view.clone().cached(StyleRefinement::default().size_full())
}

impl Workspace {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        // Closing the window with unsaved changes asks first.
        let workspace = cx.weak_entity();
        window.on_window_should_close(cx, move |window, cx| {
            workspace
                .update(cx, |this, cx| {
                    let unsaved = this.state.first_unsaved().is_some();
                    if unsaved {
                        this.quit(window, cx);
                    }
                    !unsaved
                })
                .unwrap_or(true)
        });
        let panels = Panels::new(&focus, cx);
        let (dock, dock_events) = Self::make_dock(&panels, window, cx);
        let settings = crate::app_settings::load();
        let iris = panels::iris::IrisUi::new(&settings);
        // The recipes given keys of their own are bound from the start.
        crate::keymap::use_recipe_keys(iris.recipe_keys());
        apply_keys(&settings, cx);
        let mut state = EditorState::new();
        state.history_limit = settings.history_states.clamp(5, 1000);
        // The start screen shows first, unless Settings asks to reopen the last project.
        #[cfg(not(test))]
        cx.defer_in(window, crate::project_files::on_launch);
        // Unsaved photos are copied aside as often as Settings says, and once more if the app
        // quits without asking about them (see `recovery`). (Not in headless tests, which call
        // these themselves.)
        #[cfg(not(test))]
        {
            cx.spawn_in(window, async move |this, cx| {
                loop {
                    // Read each time round, so a change in Settings applies.
                    let Ok(seconds) = this.read_with(cx, |this, _| this.ui.settings.recovery_seconds) else { break };
                    let wait = if seconds == 0 { crate::recovery::RECOVERY_INTERVAL } else { std::time::Duration::from_secs(u64::from(seconds)) };
                    cx.background_executor().timer(wait).await;
                    if this.update_in(cx, |this, window, cx| this.keep_work_safe(window, cx)).is_err() {
                        break;
                    }
                }
            })
            .detach();
            // What Omina Edit and Omina Motion send is brought in now and every few seconds.
            cx.spawn_in(window, async move |this, cx| {
                loop {
                    if this.update_in(cx, panels::iris::receive_handoffs).is_err() {
                        break;
                    }
                    cx.background_executor().timer(HANDOFF_EVERY).await;
                }
            })
            .detach();
            cx.on_app_quit(|this, _| {
                this.keep_recovery_copy_on_quit();
                async {}
            })
            .detach();
        }
        Self {
            state,
            ui: UiState {
                active_panel: PanelId::Canvas,
                left_panel: LeftPanel::Presets,
                rail_hover: None,
                rail_hovers: 0,
                rail_label: None,
                tabs: Default::default(),
                reveal_panels: Vec::new(),
                showing_panels: HashSet::new(),
                hidden_panels: hidden_panels_in(&settings),
                layout_resetting: false,
                layout_save_countdown: 0,
                panel_menu: None,
                toggled_sections: HashSet::new(),
                viewer: panels::viewer::ViewerUi::default(),
                preview: crate::preview::Preview::default(),
                editing: None,
                field_anchor: None,
                settings,
                start_screen: None,
                recent_previews: Default::default(),
                slider_bounds: Default::default(),
                slider_pressed: false,
                window_title: String::new(),
                curve_channel: 0,
                mixer_tab: 0,
                preset_pictures: Default::default(),
                exporting: None,
                iris,
                hover_edit: None,
                show_clipping: false,
                onboarding: None,
                recovery: Default::default(),
            },
            focus,
            panels,
            dock,
            _dock_events: dock_events,
        }
    }

    /// The photo's name in the title bar, with a dot while there are unsaved changes.
    fn update_window_title(&mut self, window: &mut Window) {
        let unsaved = self.state.has_unsaved_changes();
        let title = match self.state.photo() {
            Some(photo) => format!("{}{} — Omina Develop", photo.name, if unsaved { " •" } else { "" }),
            None => "Omina Develop".to_owned(),
        };
        if title != self.ui.window_title {
            window.set_window_title(&title);
            window.set_window_edited(unsaved);
            self.ui.window_title = title;
        }
    }

    /// Shows notices the state collected (things that couldn't be done, and why).
    fn show_notices(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for notice in std::mem::take(&mut self.state.notices) {
            window.push_notification(notice.as_str(), cx);
        }
    }

    /// Takes changed settings (from Settings or the welcome setup) and applies them: the theme,
    /// the shortcuts and menus (which follow the features that are on), how much undo keeps, and
    /// letting go of a tool, comparison or clipping view whose feature was just turned off.
    pub fn use_settings(&mut self, settings: crate::app_settings::AppSettings, cx: &mut Context<Self>) {
        let before = std::mem::replace(&mut self.ui.settings, settings);
        let now = &self.ui.settings;
        if (now.theme, now.accent) != (before.theme, before.accent) {
            crate::theme::apply(now.theme, now.accent, cx);
            cx.refresh_windows();
        }
        // The Help menu follows Iris's switch, so it changes with the features.
        if now.features_off != before.features_off || now.iris_on != before.iris_on || now.shortcuts != before.shortcuts {
            apply_keys(now, cx);
            refresh_menus(cx);
            // Iris never gives a recipe a key a command has now.
            self.ui.iris.use_app_keys();
        }
        let limit = now.history_states.clamp(5, 1000);
        self.state.history_limit = limit;
        for session in &mut self.state.sessions {
            session.history.limit = limit;
        }
        if !now.tool_on(self.state.tool) {
            self.state.set_tool(crate::state::Tool::View);
        }
        if !now.feature_on(crate::features::Feature::BeforeAfter) {
            self.state.compare = crate::state::Compare::Off;
        }
        if !now.feature_on(crate::features::Feature::Clipping) {
            self.ui.show_clipping = false;
        }
        cx.notify();
    }

    // ----- Text fields -----

    /// Opens a text field for `target`, starting with `text` selected.
    pub fn begin_edit(&mut self, target: EditTarget, text: String, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_edit(window, cx);
        let input = cx.new(|cx| InputState::new(window, cx).default_value(text.clone()));
        input.update(cx, |state, cx| {
            state.focus(window, cx);
            state.select_all(window, cx);
        });
        let events = cx.subscribe_in(&input, window, move |this, _, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::Blur | InputEvent::PressEnter { .. }) {
                this.commit_edit(window, cx);
            }
        });
        let changes = cx.observe(&input, |_, _, cx| cx.notify());
        self.ui.editing = Some(EditingField { target, input, initial: text, _events: events, _changes: changes });
        cx.notify();
    }

    /// Applies and closes the open text field, if any.
    pub fn commit_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editing) = self.ui.editing.take() else { return };
        let value = editing.input.read(cx).value().to_string();
        match editing.target {
            EditTarget::Number(target) => {
                if value != editing.initial
                    && let Some(number) = panels::fields::parse_value(&value)
                {
                    target.write(&mut self.state, number);
                }
            }
            EditTarget::PhotoName => self.state.rename(&value),
            EditTarget::SnapshotName(index) => self.state.rename_snapshot(index, &value),
            EditTarget::MaskName(index) => {
                let name = value.trim().to_owned();
                if !name.is_empty() {
                    self.state.edit_mask(index, "Rename Mask", |mask| mask.name = name);
                }
            }
        }
        // Focus goes back to the editor so shortcuts work again.
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// The text field open for `target`, if it is.
    pub fn editing_field(&self, target: EditTarget) -> Option<AnyElement> {
        use gpui_kit::component::Sizable as _;
        let editing = self.ui.editing.as_ref().filter(|editing| editing.target == target)?;
        Some(gpui_kit::component::input::Input::new(&editing.input).xsmall().into_any_element())
    }

    /// Scrubs the field being dragged to `pointer_x`.
    pub fn scrub_field(&mut self, target: FieldTarget, pointer_x: f32, coarse: bool, fine: bool) {
        let Some((origin_x, start)) = self.ui.field_anchor else { return };
        let Some((_, format)) = target.read(&self.state) else { return };
        let speed = if coarse { 10.0 } else if fine { 0.1 } else { 1.0 };
        target.write(&mut self.state, start + (pointer_x - origin_x) * format.step * speed);
    }

    // ----- Panels -----

    /// Shows `panel` in the side panel and brings the side panel to the front.
    pub fn show_side(&mut self, panel: LeftPanel) {
        self.ui.left_panel = panel;
        self.reveal(PanelId::Side);
    }

    pub fn reveal(&mut self, panel: PanelId) {
        if !self.ui.reveal_panels.contains(&panel) {
            self.ui.reveal_panels.push(panel);
        }
    }

    /// Redraws only the viewer (a zoom or pan), not every panel.
    pub fn notify_canvas(&self, cx: &mut App) {
        cx.notify(self.panels.canvas.entity_id());
    }

    fn make_dock(panels: &Panels, window: &mut Window, cx: &mut Context<Self>) -> (Entity<DockArea>, Subscription) {
        let skin = panels::dock::DevelopSkin::new(cx.weak_entity());
        let dock = cx.new(|cx| DockArea::new("develop-workspace", Some(panels::dock::LAYOUT_VERSION), window, cx).with_renderer(skin));
        let view = |id| panels.view(id).clone();
        let settings = crate::app_settings::load();
        let hidden = hidden_panels_in(&settings);
        let saved = settings.workspace_layout.and_then(|saved| serde_json::from_value::<DockAreaState>(saved).ok());
        let layout = match saved.and_then(|saved| panels::dock::restore_layout(&saved, &hidden, view)) {
            Some(layout) => layout,
            None => {
                crate::app_settings::update(|settings| settings.hidden_panels = panels::dock::standard_hidden());
                panels::dock::default_layout(view)
            }
        };
        dock.update(cx, |dock, cx| dock.set_center(layout, window, cx));
        let events = cx.subscribe(&dock, |this, dock, event: &DockEvent, cx| {
            if let DockEvent::LayoutChanged = event {
                if std::mem::take(&mut this.ui.layout_resetting) {
                    this.ui.layout_save_countdown = 0;
                    crate::app_settings::update(|settings| settings.workspace_layout = None);
                } else {
                    this.ui.layout_save_countdown = 2;
                    if let Ok(layout) = serde_json::to_value(dock.read(cx).dump(cx)) {
                        crate::app_settings::update(|settings| settings.workspace_layout = Some(layout));
                    }
                }
                this.refresh_showing(cx);
                cx.notify();
            }
        });
        (dock, events)
    }

    /// Window ▸ Reset Workspace: every panel back in its standard place.
    pub fn reset_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.ui.layout_resetting = true;
        let view = |id| self.panels.view(id).clone();
        let layout = panels::dock::default_layout(view);
        self.dock.update(cx, |dock, cx| dock.set_center(layout, window, cx));
        let standard = panels::dock::standard_hidden();
        self.ui.hidden_panels = standard.iter().filter_map(|panel| PanelId::from_name(&panel.name)).collect();
        crate::app_settings::update(|settings| {
            settings.workspace_layout = None;
            settings.hidden_panels = standard;
        });
        refresh_menus(cx);
        cx.notify();
    }

    /// Takes panel `id` off screen (any panel but the photo), remembering where it was.
    pub fn hide_panel(&mut self, id: PanelId, window: &mut Window, cx: &mut Context<Self>) {
        if id == PanelId::Canvas || self.ui.hidden_panels.contains(&id) {
            return;
        }
        let mut saved = self.dock.read(cx).dump(cx);
        let Some(hidden) = panels::dock::take_panel(&mut saved.center, id.name()) else { return };
        self.ui.hidden_panels.insert(id);
        crate::app_settings::update(|settings| {
            settings.hidden_panels.retain(|panel| panel.name != hidden.name);
            settings.hidden_panels.push(hidden);
        });
        self.rebuild_dock(&saved, window, cx);
    }

    /// Brings hidden panel `id` back where it was and to the front of its group.
    pub fn show_panel(&mut self, id: PanelId, window: &mut Window, cx: &mut Context<Self>) {
        if !self.ui.hidden_panels.remove(&id) {
            return;
        }
        let mut remembered = None;
        crate::app_settings::update(|settings| {
            if let Some(index) = settings.hidden_panels.iter().position(|panel| panel.name == id.name()) {
                remembered = Some(settings.hidden_panels.remove(index));
            }
        });
        let mut saved = self.dock.read(cx).dump(cx);
        if let Some(remembered) = remembered {
            panels::dock::put_back(&mut saved.center, &remembered);
        }
        self.rebuild_dock(&saved, window, cx);
        self.reveal(id);
    }

    /// Window ▸ (a panel's name): shows the panel if hidden, hides it otherwise.
    pub fn toggle_panel(&mut self, id: PanelId, window: &mut Window, cx: &mut Context<Self>) {
        if self.ui.hidden_panels.contains(&id) {
            self.show_panel(id, window, cx);
        } else {
            self.hide_panel(id, window, cx);
        }
    }

    /// Lays the dock out again from `saved`, without the hidden panels.
    fn rebuild_dock(&mut self, saved: &DockAreaState, window: &mut Window, cx: &mut Context<Self>) {
        let view = |id| self.panels.view(id).clone();
        if let Some(layout) = panels::dock::restore_layout(saved, &self.ui.hidden_panels, view) {
            self.dock.update(cx, |dock, cx| dock.set_center(layout, window, cx));
        }
        refresh_menus(cx);
        cx.notify();
    }

    /// Brings asked-for panels to the front of their groups and notes which are showing.
    fn sync_dock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for id in std::mem::take(&mut self.ui.reveal_panels) {
            // Asking for a hidden panel brings it back first.
            self.show_panel(id, window, cx);
            let panel = gpui_kit::component::dock::PanelId::from(self.panels.view(id).entity_id());
            self.dock.update(cx, |dock, cx| dock.select_panel(panel, window, cx));
        }
        self.refresh_showing(cx);
    }

    /// Notes which panels are on screen (the front tab of each group).
    fn refresh_showing(&mut self, cx: &App) {
        let dock = self.dock.read(cx);
        let front: HashSet<gpui_kit::component::dock::PanelId> = dock
            .layout(gpui_kit::component::dock::DockPlacement::Center)
            .map(|tree| {
                let mut front = HashSet::new();
                tree.root().walk(&mut |node| {
                    if let gpui_kit::component::dock::PaneRef::Tabs { panels, active_ix } = node.kind()
                        && let Some(panel) = panels.get(active_ix).or(panels.first())
                    {
                        front.insert(*panel);
                    }
                });
                front
            })
            .unwrap_or_default();
        self.ui.showing_panels = panels::dock::DOCKABLE.into_iter().filter(|id| front.contains(&gpui_kit::component::dock::PanelId::from(self.panels.view(*id).entity_id()))).collect();
    }

    /// Zooms one step in or out around the middle of the photo.
    pub fn zoom_step(&mut self, zoom_in: bool) {
        panels::viewer::zoom_step(self, zoom_in);
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // A changed panel arrangement is saved again once a draw has measured it.
        if self.ui.layout_save_countdown > 0 {
            self.ui.layout_save_countdown -= 1;
            if self.ui.layout_save_countdown > 0 {
                cx.notify();
            } else if let Ok(layout) = serde_json::to_value(self.dock.read(cx).dump(cx)) {
                crate::app_settings::update(|settings| settings.workspace_layout = Some(layout));
            }
        }
        self.sync_dock(window, cx);
        panels::iris::keep_up(self, window, cx);
        // Every edit redraws the workspace, so this is where edits become undo steps. Dragging a
        // slider, a curve point, a value or a handle on the photo counts as one change.
        let dragging = cx.has_active_drag() || self.ui.slider_pressed || self.state.interaction.is_some();
        self.state.record_history(dragging);
        self.show_notices(window, cx);
        // A dialog closed with OK leaves nothing focused; give focus back to the editor.
        if window.focused(cx).is_none() && !window.has_active_dialog(cx) {
            window.focus(&self.focus, cx);
        }
        self.update_window_title(window);
        let panels = &self.panels;
        let gutter = px(crate::theme::size::GUTTER);
        let dock = div().id("workspace-dock").size_full().child(self.dock.clone());

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(hsla(color::BACKGROUND))
            .text_color(hsla(color::TEXT))
            .text_size(px(crate::theme::size::TEXT))
            .track_focus(&self.focus)
            .key_context(KEY_CONTEXT)
            // While the welcome setup shows, the editor behind it takes no commands, so a key
            // can't change a photo nobody can see.
            .map(|root| if self.ui.onboarding.is_some() { crate::commands::bind_during_setup(root, cx) } else { crate::commands::bind(root, cx) })
            // Releasing the mouse anywhere ends whatever drag was in progress.
            // A rail button's name label goes as soon as the pointer is anywhere off the rail, even
            // if the move off it was too quick for the button to notice.
            .on_mouse_move(cx.listener(|this, event: &gpui_kit::MouseMoveEvent, _, cx| {
                if this.ui.rail_hover.is_some() && f32::from(event.position.x) > panels::toolbar::RAIL_WIDTH {
                    this.ui.rail_hover = None;
                    if this.ui.rail_label.take().is_some() {
                        cx.notify();
                    }
                }
            }))
            .capture_any_mouse_up(cx.listener(|this, _, _, cx| {
                this.ui.field_anchor = None;
                this.ui.slider_pressed = false;
                if this.state.interaction.is_some() {
                    this.state.release();
                }
                this.ui.viewer.hand = None;
                this.ui.viewer.dragging_split = false;
                cx.notify();
            }))
            // Scrubbing a value field: the drag can wander anywhere in the window.
            .on_drag_move(cx.listener(|this, event: &DragMoveEvent<FieldDrag>, _, cx| {
                let target = event.drag(cx).0;
                let modifiers = event.event.modifiers;
                this.scrub_field(target, f32::from(event.event.position.x), modifiers.shift, modifiers.alt);
                cx.notify();
            }))
            .child(div().h(px(crate::theme::size::TOOLBAR_HEIGHT)).flex_none().child(cached(&panels.toolbar)))
            .child(div().flex_1().min_h_0().flex().child(panels::toolbar::rail(self, cx)).child(div().flex_1().min_w_0().pr(gutter).pb(gutter).child(dock)))
            .children(panels::dock::panel_menu(self))
            .children(panels::start_screen::render(self, cx))
            .children(panels::onboarding::render(self, cx))
            .when_some(self.ui.exporting.clone(), |root, what| root.child(panels::export::progress(what)))
    }
}

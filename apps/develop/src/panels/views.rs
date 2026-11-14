//! Each panel is its own cached GPUI view, so a frame only redraws the panels whose content
//! changed. Edits notify the workspace, which every panel observes; zooming and panning the
//! photo notify only the viewer.

use gpui_kit::component::dock::{BasePanel, PanelEvent};
use gpui_kit::{AnyElement, App, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, Render, WeakEntity, Window, div};

use crate::workspace::{LeftPanel, PanelId, Workspace};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelKind {
    Toolbar,
    /// The panel beside the rail: Presets, Snapshots or History, whichever the rail picked.
    Side,
    Canvas,
    Develop,
}

impl PanelKind {
    /// The dockable panel this view draws; `None` for the toolbar, which stays at the top.
    pub fn dock_id(self) -> Option<PanelId> {
        Some(match self {
            PanelKind::Toolbar => return None,
            PanelKind::Side => PanelId::Side,
            PanelKind::Canvas => PanelId::Canvas,
            PanelKind::Develop => PanelId::Develop,
        })
    }
}

/// A panel view: renders one panel from the shared workspace state.
pub struct PanelView {
    workspace: WeakEntity<Workspace>,
    pub kind: PanelKind,
    /// The editor's own focus. A panel picked in the dock hands focus back to it, so keyboard
    /// shortcuts keep working wherever the panel has been moved.
    focus: FocusHandle,
}

impl PanelView {
    pub fn new(workspace: &Entity<Workspace>, kind: PanelKind, focus: FocusHandle, cx: &mut Context<Self>) -> Self {
        // Any edit notifies the workspace; redraw this panel when that happens.
        cx.observe(workspace, |_, _, cx| cx.notify()).detach();
        Self { workspace: workspace.downgrade(), kind, focus }
    }
}

impl Render for PanelView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let kind = self.kind;
        self.workspace
            .update(cx, |workspace, cx| -> AnyElement {
                use super::*;
                match kind {
                    PanelKind::Toolbar => toolbar::render(workspace, cx).into_any_element(),
                    // A panel whose feature is turned off (Iris, say) gives way to Presets or History.
                    PanelKind::Side => match workspace.ui.side_panel() {
                        LeftPanel::Presets => presets::render(workspace, window, cx).into_any_element(),
                        LeftPanel::Snapshots => snapshots::render(workspace, cx).into_any_element(),
                        LeftPanel::History => history::render(workspace, cx).into_any_element(),
                        LeftPanel::Iris => iris::render(workspace, cx).into_any_element(),
                    },
                    PanelKind::Canvas => viewer::render(workspace, window, cx).into_any_element(),
                    PanelKind::Develop => develop::render(workspace, window, cx).into_any_element(),
                }
            })
            .unwrap_or_else(|_| div().into_any_element())
    }
}

impl EventEmitter<PanelEvent> for PanelView {}

impl Focusable for PanelView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// How the dock sees a panel: never closed (each always has a place), and any can fill the
/// window for a while (Maximize Panel, in the menu on its tab's grip).
impl BasePanel for PanelView {
    fn panel_name(&self) -> &'static str {
        self.kind.dock_id().map_or("Toolbar", PanelId::name)
    }

    fn closable(&self, _: &App) -> bool {
        false
    }

    fn zoomable(&self, _: &App) -> bool {
        true
    }
}

//! The window's chrome around the panels, as in Studio and Canvas: a slim top bar and a rail
//! down the left side.
//!
//! The top bar has Develop's mark on the left (a click goes to the start screen), the tools,
//! one small icon each named in a tooltip with its key, then before / after and copy / paste
//! settings, and on the right the photo's name (click to rename it) and the one call to action,
//! Export. The rail is a quiet column of icons, each named in a label that pops out beside it
//! while the pointer rests on it; it picks the side panel (Presets, Snapshots, History, Iris) and, at
//! the bottom, has Import, Save and Export, then Iris and Settings.

use std::time::Duration;

use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::{
    Context, InteractiveElement, IntoElement, ParentElement, StatefulInteractiveElement, Styled, TestSupportExt as _, Window, deferred, div, prelude::FluentBuilder as _, px,
};

use crate::features::Feature;
use crate::state::{Compare, MaskKind, Tool};
use crate::theme::{color, hsla, size};
use crate::workspace::{EditTarget, LeftPanel, PanelId, Workspace};

/// Width of the left rail: room for one icon button with a margin either side.
pub const RAIL_WIDTH: f32 = 52.0;
/// The side of a rail button, a rounded square around its icon.
const RAIL_BUTTON: f32 = 36.0;
/// How long the pointer rests on a rail button before its name shows.
const RAIL_LABEL_WAIT: Duration = Duration::from_millis(500);

/// Width and height of a tool button in the top bar.
const TOOL_WIDTH: f32 = 36.0;
const TOOL_HEIGHT: f32 = 34.0;

/// The top bar.
pub fn render(workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let open = workspace.state.photo().is_some();
    let tool = workspace.state.tool;
    let masking = |kind: MaskKind| tool == Tool::Masks && workspace.state.mask_kind == kind;
    let settings = &workspace.ui.settings;
    let on = |feature: Feature| settings.feature_on(feature);
    // The tools in groups; a feature turned off in Settings ▸ Tools & Features takes its button
    // away, and a group left empty takes its divider with it.
    let mut picking = vec![tool_button("tool-view", IconName::Hand, crate::keymap::with_key("Hand: drag to move around, double-click to zoom", "view-tool"), tool == Tool::View, open, |this| this.state.set_tool(Tool::View), cx).into_any_element()];
    if on(Feature::Crop) {
        picking.push(tool_button("tool-crop", IconName::Crop, crate::keymap::with_key("Crop & Straighten", "crop-tool"), tool == Tool::Crop, open, |this| {
            let next = if this.state.tool == Tool::Crop { Tool::View } else { Tool::Crop };
            this.state.set_tool(next);
        }, cx).into_any_element());
    }
    if on(Feature::WhiteBalanceSelector) {
        picking.push(tool_button("tool-white-balance", IconName::Pipette, crate::keymap::with_key("White Balance Selector", "white-balance-tool"), tool == Tool::WhiteBalance, open, |this| this.state.set_tool(Tool::WhiteBalance), cx).into_any_element());
    }
    let mut masks = Vec::new();
    if on(Feature::Masks) {
        masks.push(tool_button("tool-linear", IconName::SquareSplitVertical, crate::keymap::with_key("Linear Gradient Mask", "linear-mask-tool"), masking(MaskKind::Linear), open, |this| this.state.use_mask_tool(MaskKind::Linear), cx).into_any_element());
        masks.push(tool_button("tool-radial", IconName::CircleDashed, crate::keymap::with_key("Radial Gradient Mask", "radial-mask-tool"), masking(MaskKind::Radial), open, |this| this.state.use_mask_tool(MaskKind::Radial), cx).into_any_element());
    }
    let mut comparing = Vec::new();
    if on(Feature::BeforeAfter) {
        comparing.push(tool_button("tool-before", IconName::Eye, crate::keymap::with_key("Show Before", "before"), workspace.state.compare == Compare::Before, open, |this| this.state.toggle_before(), cx).into_any_element());
        comparing.push(tool_button("tool-split", IconName::Columns2, crate::keymap::with_key("Before / After Split", "split"), workspace.state.compare == Compare::Split, open, |this| this.state.toggle_split(), cx).into_any_element());
    }
    let mut copying = Vec::new();
    if on(Feature::CopyPaste) {
        copying.push(tool_button("tool-copy", IconName::Copy, crate::keymap::with_key("Copy Settings (not the crop)", "copy-settings"), false, open, |this| this.state.copy_settings(), cx).into_any_element());
        copying.push(tool_button("tool-paste", IconName::ClipboardPaste, crate::keymap::with_key("Paste Settings", "paste-settings"), false, open && workspace.state.copied.is_some(), |this| {
            this.state.paste_settings();
        }, cx).into_any_element());
    }
    let mut tools = div().flex_none().flex().items_center().gap_0p5();
    for (index, group) in [picking, masks, comparing, copying].into_iter().filter(|group| !group.is_empty()).enumerate() {
        tools = tools.when(index > 0, |tools| tools.child(divider())).children(group);
    }
    // The photo's name; a click opens a field to rename it.
    let name = workspace.state.photo().map(|photo| {
        let unsaved = workspace.state.has_unsaved_changes();
        match workspace.editing_field(EditTarget::PhotoName) {
            Some(field) => div().w(px(220.0)).child(field).into_any_element(),
            None => {
                let current = photo.name.clone();
                div()
                    .id("photo-name")
                    .min_w_0()
                    .truncate()
                    .px_1p5()
                    .py_0p5()
                    .rounded(px(4.0))
                    .text_color(hsla(color::TEXT))
                    .hover(|style| style.bg(hsla(color::HOVER)))
                    .cursor_pointer()
                    .tooltip(|window, cx| gpui_kit::component::tooltip::Tooltip::new("Rename").build(window, cx))
                    .on_click(cx.listener(move |this, _, window, cx| this.begin_edit(EditTarget::PhotoName, current.clone(), window, cx)))
                    .child(if unsaved { format!("{} •", photo.name) } else { photo.name.clone() })
                    .test_support()
                    .into_any_element()
            }
        }
    });
    let exporting = workspace.ui.exporting.as_ref().map(|what| div().flex_none().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(what.clone()));
    div()
        .w_full()
        .h(px(size::TOOLBAR_HEIGHT))
        .flex_none()
        .flex()
        .items_center()
        .gap_3()
        .px_3()
        .bg(hsla(color::BACKGROUND))
        .child(home_button(cx))
        .child(tools)
        .child(div().flex_1().min_w_0().flex().items_center().justify_end().gap_2().children(exporting).children(name))
        .child(export_button(open, cx))
}

/// Develop's mark in the window's top left corner, which goes to the start screen when clicked.
fn home_button(cx: &mut Context<Workspace>) -> impl IntoElement {
    div()
        .id("develop-home")
        .flex_none()
        // As wide as the rail's buttons, so the mark sits centered over them.
        .w(px(RAIL_BUTTON))
        .flex()
        .justify_center()
        .rounded(px(8.0))
        .hover(|style| style.opacity(0.85))
        .cursor_pointer()
        .tooltip(|window, cx| gpui_kit::component::tooltip::Tooltip::new("Start Screen: open a photo or a recent project").build(window, cx))
        .on_click(cx.listener(|this, _, window, cx| super::start_screen::go_to(this, window, cx)))
        .child(super::start_screen::brand_mark(30.0))
        .test_support()
}

/// Export, in the accent color: the one call to action on screen.
fn export_button(enabled: bool, cx: &mut Context<Workspace>) -> impl IntoElement {
    div()
        .id("toolbar-export")
        .flex_none()
        .h(px(28.0))
        .px_3()
        .flex()
        .items_center()
        .gap_1p5()
        .rounded(px(6.0))
        .bg(hsla(color::HIGHLIGHT))
        .text_color(crate::theme::on_accent())
        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
        .when(!enabled, |button| button.opacity(0.4))
        .when(enabled, |button| {
            button.cursor_pointer().hover(|style| style.opacity(0.9)).on_click(cx.listener(|this, _, window, cx| super::export::open(this, window, cx)))
        })
        .tooltip(|window, cx| gpui_kit::component::tooltip::Tooltip::new(crate::keymap::with_key("Export the finished photo", "export")).build(window, cx))
        .child(Icon::new(IconName::Download).size(px(14.0)))
        .child("Export")
        .test_support()
}

/// A tool button: an icon, lit while its tool is in use, named in a tooltip with its key.
fn tool_button(
    id: &'static str,
    icon: IconName,
    tip: String,
    active: bool,
    enabled: bool,
    pick: impl Fn(&mut Workspace) + 'static,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
    div()
        .id(id)
        .relative()
        .w(px(TOOL_WIDTH))
        .h(px(TOOL_HEIGHT))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .text_color(hsla(if active { color::TEXT } else { color::MUTED }))
        .when(active, |button| button.bg(hsla(color::ROW_SELECTED)))
        .when(!enabled, |button| button.opacity(0.35))
        .when(enabled, |button| {
            button.hover(|style| style.bg(hsla(color::HOVER)).text_color(hsla(color::TEXT))).on_click(cx.listener(move |this, _, window, cx| {
                this.commit_edit(window, cx);
                pick(this);
                cx.notify();
            }))
        })
        .tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(tip.clone()).build(window, cx))
        .child(Icon::new(icon).size(px(19.0)))
        .test_support()
}

/// A short line between groups of tools.
fn divider() -> gpui_kit::Div {
    div().flex_none().w(px(1.0)).h(px(16.0)).mx_1().bg(hsla(color::MUTED).opacity(0.35))
}

/// A short line between groups of rail buttons. The panel hairline is too faint against the
/// rail's darker background, so it uses the muted text color, softened.
fn rail_separator() -> gpui_kit::Div {
    div().flex_none().h(px(1.0)).w(px(28.0)).my_2().bg(hsla(color::MUTED).opacity(0.35))
}

/// What a rail button shows: an icon, or Iris's face.
enum RailIcon {
    Icon(IconName),
    Face,
}

/// A rail button: an icon in a rounded square. Its name shows in a label beside it once the
/// pointer has rested on it for a moment, so the rail stays a quiet column of icons. `label` is
/// that name; for a command it also carries the shortcut.
#[allow(clippy::too_many_arguments)]
fn rail_button(
    id: &'static str,
    icon: RailIcon,
    label: impl Into<gpui_kit::SharedString>,
    active: bool,
    enabled: bool,
    on_click: impl Fn(&mut Workspace, &mut Window, &mut Context<Workspace>) + 'static,
    workspace: &Workspace,
    cx: &mut Context<Workspace>,
) -> gpui_kit::AnyElement {
    let label: gpui_kit::SharedString = label.into();
    let named = workspace.ui.rail_label == Some(id);
    div()
        .id(id)
        .relative()
        .size(px(RAIL_BUTTON))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(10.0))
        .text_color(hsla(if active { color::TEXT } else { color::MUTED }))
        .when(active, |button| button.bg(hsla(color::HOVER)))
        .when(!enabled, |button| button.opacity(0.35))
        .hover(|style| style.bg(hsla(color::HOVER)).text_color(hsla(color::TEXT)))
        .on_hover(cx.listener(move |this, hovered: &bool, window, cx| {
            if !*hovered {
                // Leaving one button for the next can arrive after "entered the next", so a
                // button only clears the hover if it is still the one under the pointer.
                if this.ui.rail_hover.is_some_and(|(over, _)| over == id) {
                    this.ui.rail_hover = None;
                    this.ui.rail_label = None;
                    cx.notify();
                }
                return;
            }
            // Each hover gets a number, so a wait that outlives its hover does nothing.
            this.ui.rail_hovers += 1;
            let hover = (id, this.ui.rail_hovers);
            this.ui.rail_hover = Some(hover);
            if this.ui.rail_label.take().is_some() {
                cx.notify();
            }
            cx.spawn_in(window, async move |this, cx| {
                cx.background_executor().timer(RAIL_LABEL_WAIT).await;
                this.update(cx, |this, cx| {
                    if this.ui.rail_hover == Some(hover) {
                        this.ui.rail_label = Some(id);
                        cx.notify();
                    }
                })
                .ok();
            })
            .detach();
        }))
        .when(enabled, |button| {
            button.on_click(cx.listener(move |this, _, window, cx| {
                this.commit_edit(window, cx);
                on_click(this, window, cx);
            }))
        })
        .child(match icon {
            RailIcon::Icon(icon) => Icon::new(icon).size(px(19.0)).into_any_element(),
            RailIcon::Face => super::iris::face("rail", 22.0, super::iris::current_look(workspace)).into_any_element(),
        })
        // The label is drawn after everything else, so it sits over the panel beside the rail.
        .when(named, |button| button.child(deferred(rail_label(id, label)).with_priority(1)))
        .test_support()
        .into_any_element()
}

/// The name beside a rail button the pointer has rested on: a small grey bubble to the
/// button's right, centered on it. It pops in, fading up while sliding a few points out from
/// the button and settling with a small bounce.
fn rail_label(id: &'static str, label: gpui_kit::SharedString) -> impl IntoElement {
    use gpui_kit::{Animation, AnimationExt as _};
    let rest = RAIL_BUTTON + 8.0;
    let pill = div()
        .px_2p5()
        .py_1p5()
        .rounded(px(8.0))
        .bg(hsla(color::NAVIGATOR_THUMB))
        .shadow_md()
        .text_size(px(size::TEXT))
        .text_color(hsla(color::TEXT))
        .whitespace_nowrap()
        .child(label);
    div().absolute().top_0().h(px(RAIL_BUTTON)).flex().items_center().child(pill).with_animation(
        gpui_kit::ElementId::Name(format!("rail-label:{id}").into()),
        // Eases out past its resting place and back (a "back out" curve).
        Animation::new(Duration::from_millis(280)).with_easing(|time: f32| {
            let (overshoot, back) = (1.70158, time - 1.0);
            1.0 + (overshoot + 1.0) * back.powi(3) + overshoot * back.powi(2)
        }),
        move |label, progress| label.left(px(rest - 10.0 * (1.0 - progress))).opacity(progress.clamp(0.0, 1.0)),
    )
}

/// The rail down the left: the side panels, then getting photos in and out.
pub fn rail(workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    // A button switches what the side panel shows, and brings the side panel to the front
    // wherever it has been moved to (back on screen, if it was hidden). The button for what it
    // shows is lit; clicking it again puts the side panel away, for more room.
    let show = |panel: LeftPanel| {
        move |this: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>| {
            let open = this.ui.side_panel() == panel && this.ui.showing_panels.contains(&PanelId::Side) && !this.ui.hidden_panels.contains(&PanelId::Side);
            if open {
                this.hide_panel(PanelId::Side, window, cx);
            } else {
                this.show_side(panel);
            }
            cx.notify();
        }
    };
    let side_open = !workspace.ui.hidden_panels.contains(&PanelId::Side);
    let lit = |panel: LeftPanel| side_open && workspace.ui.side_panel() == panel;
    let panel_on = |panel: LeftPanel| workspace.ui.settings.panel_on(panel);
    let open = workspace.state.photo().is_some();
    // Iris's name, with how many things it noticed in the photo.
    let iris_name = super::iris::name(workspace);
    let iris_on = super::iris::is_on(workspace);
    let iris_label = match super::iris::open_suggestions(workspace).len() {
        0 => format!("{iris_name}: photo help"),
        count => format!("{iris_name}: {count} suggestion{}", if count == 1 { "" } else { "s" }),
    };
    div()
        .w(px(RAIL_WIDTH))
        .flex_none()
        .h_full()
        .flex()
        .flex_col()
        .items_center()
        .gap_1p5()
        .py_2()
        .bg(hsla(color::BACKGROUND))
        // At the top: the side panels.
        .when(panel_on(LeftPanel::Presets), |rail| rail.child(rail_button("rail-presets", RailIcon::Icon(IconName::Sparkles), "Presets", lit(LeftPanel::Presets), true, show(LeftPanel::Presets), workspace, cx)))
        .when(panel_on(LeftPanel::Snapshots), |rail| rail.child(rail_button("rail-snapshots", RailIcon::Icon(IconName::Camera), "Snapshots", lit(LeftPanel::Snapshots), true, show(LeftPanel::Snapshots), workspace, cx)))
        .child(rail_button("rail-history", RailIcon::Icon(IconName::Clock), "History", lit(LeftPanel::History), true, show(LeftPanel::History), workspace, cx))
        // At the bottom: getting photos in and out, each named with its shortcut, and Settings last.
        .child(div().flex_1())
        .child(rail_button("rail-import", RailIcon::Icon(IconName::ImagePlus), crate::keymap::with_key("Import a Photo or Open a Project", "open"), false, true, |this, window, cx| this.open(window, cx), workspace, cx))
        .child(rail_button("rail-save", RailIcon::Icon(IconName::Save), crate::keymap::with_key("Save Project", "save"), false, open, |this, window, cx| this.save(window, cx), workspace, cx))
        .child(rail_button("rail-export", RailIcon::Icon(IconName::Download), crate::keymap::with_key("Export", "export"), false, open, super::export::open, workspace, cx))
        .child(rail_separator())
        // Iris sits just above Settings, where she's easy to find without crowding the panels.
        .when(iris_on, |rail| rail.child(rail_button("rail-iris", RailIcon::Face, iris_label, lit(LeftPanel::Iris), true, show(LeftPanel::Iris), workspace, cx)))
        .child(rail_button("rail-settings", RailIcon::Icon(IconName::Settings), crate::keymap::with_key("Settings", "settings"), false, true, super::settings::open, workspace, cx))
}

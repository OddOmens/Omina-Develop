//! The Snapshots panel, as Lightroom's: named copies of the photo's edit to come back to.
//! Clicking one brings its edit back (one undo step); double-clicking renames it; its buttons
//! update it to the current edit or delete it.

use gpui_kit::assets::IconName;
use gpui_kit::component::{Disableable as _, Icon};
use gpui_kit::{AnyElement, Context, ElementId, InteractiveElement, IntoElement, ParentElement, StatefulInteractiveElement, Styled, TestSupportExt as _, div, px};

use super::chrome::{card_list, icon_button, list_row, panel};
use crate::theme::{color, hsla, size};
use crate::workspace::{EditTarget, PanelId, Workspace};

pub fn render(workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let snapshots: Vec<String> = workspace.state.photo().map(|photo| photo.snapshots.iter().map(|snapshot| snapshot.name.clone()).collect()).unwrap_or_default();
    let open = workspace.state.photo().is_some();
    let mut list = card_list("snapshots-list");
    if snapshots.is_empty() {
        let note = if open { "Save the edit as it is now to come back to it later: press + or ⌘N." } else { "Open a photo to keep snapshots of its edit." };
        list = list.child(list_row().text_color(hsla(color::MUTED)).child(note));
    }
    for (index, name) in snapshots.into_iter().enumerate() {
        list = list.child(snapshot_row(index, name, workspace, cx));
    }
    let count = workspace.state.photo().map_or(0, |photo| photo.snapshots.len());
    let footer = div()
        .size_full()
        .flex()
        .items_center()
        .gap_1()
        .child(div().flex_1().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(super::chrome::counted(count, "snapshot", "snapshots")))
        .child(icon_button("snapshot-new", IconName::Plus, crate::keymap::with_key("New Snapshot of the edit as it is now", "new-snapshot"), false, cx.listener(|this, _, window, cx| {
            this.commit_edit(window, cx);
            this.state.add_snapshot(None);
            cx.notify();
        })).disabled(!open));
    panel(PanelId::Side, &[], Some(footer.into_any_element()), list, workspace, cx)
}

fn snapshot_row(index: usize, name: String, workspace: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let label: AnyElement = match workspace.editing_field(EditTarget::SnapshotName(index)) {
        Some(field) => div().flex_1().child(field).into_any_element(),
        None => div().flex_1().min_w_0().truncate().child(name.clone()).into_any_element(),
    };
    div()
        .id(ElementId::Name(format!("snapshot-{index}").into()))
        .h(px(30.0))
        .flex_none()
        .flex()
        .items_center()
        .gap_2()
        .px_2p5()
        .border_b_1()
        .border_color(hsla(color::HAIRLINE))
        .hover(|style| style.bg(hsla(color::HOVER)))
        .cursor_pointer()
        .on_click(cx.listener(move |this, event: &gpui_kit::ClickEvent, window, cx| {
            if event.click_count() >= 2 {
                this.begin_edit(EditTarget::SnapshotName(index), name.clone(), window, cx);
            } else {
                this.commit_edit(window, cx);
                this.state.apply_snapshot(index);
            }
            cx.notify();
        }))
        .child(Icon::new(IconName::Camera).size(px(13.0)).text_color(hsla(color::MUTED)))
        .child(label)
        .child(icon_button(ElementId::Name(format!("snapshot-update-{index}").into()), IconName::RefreshCw, "Update to the edit as it is now", false, cx.listener(move |this, _, _, cx| {
            cx.stop_propagation();
            this.state.update_snapshot(index);
            cx.notify();
        })))
        .child(icon_button(ElementId::Name(format!("snapshot-delete-{index}").into()), IconName::Trash, "Delete this snapshot", false, cx.listener(move |this, _, _, cx| {
            cx.stop_propagation();
            this.state.delete_snapshot(index);
            cx.notify();
        })))
        .test_support()
        .into_any_element()
}

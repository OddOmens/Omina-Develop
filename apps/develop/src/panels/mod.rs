//! The editor's panels. Each module renders one panel (or dialog) from the workspace state.

pub mod chrome;
pub mod curves;
pub mod develop;
pub mod dialogs;
pub mod dock;
pub mod export;
pub mod fields;
pub mod history;
pub mod presets;
pub mod settings;
pub mod snapshots;
pub mod start_screen;
pub mod toolbar;
pub mod iris;
pub mod onboarding;
pub mod viewer;
pub mod views;

use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::{AnyElement, Context, ElementId, IntoElement};

use crate::workspace::Workspace;

/// A checkbox with a label that changes something on the workspace when clicked.
pub(crate) fn properties_checkbox(id: impl Into<ElementId>, label: &'static str, checked: bool, set: impl Fn(&mut Workspace, bool) + 'static, cx: &mut Context<Workspace>) -> AnyElement {
    Checkbox::new(id)
        .label(label)
        .checked(checked)
        .on_click(cx.listener(move |this, checked: &bool, _, cx| {
            set(this, *checked);
            cx.notify();
        }))
        .into_any_element()
}

//! The small dialogs: New Preset and About. Each opens in the middle of the window, looking
//! like the rest of the editor, and changes nothing until OK.

use std::cell::Cell;
use std::rc::Rc;

use develop_core::presets::{Preset, PresetParts};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::dialog::{DialogAction, DialogClose, DialogFooter};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::{AppContext as _, Context, ParentElement, Styled, Window, div, px};

use crate::theme::{color, hsla};
use crate::workspace::Workspace;

/// The most presets of your own kept, so the settings file stays a sensible size.
const MAX_USER_PRESETS: usize = 500;
/// The heading your own presets are listed under.
pub const USER_GROUP: &str = "User Presets";

/// Keeps `preset` among your own presets and saves them. A preset of the same name is
/// replaced, as saving over it would; returns whether one was.
pub fn add_user_preset(workspace: &mut Workspace, preset: Preset) -> bool {
    let presets = &mut workspace.ui.settings.user_presets;
    let before = presets.len();
    presets.retain(|existing| existing.name != preset.name);
    let replaced = presets.len() < before;
    presets.push(preset);
    if presets.len() > MAX_USER_PRESETS {
        presets.remove(0);
    }
    let saved = presets.clone();
    crate::app_settings::update(|settings| settings.user_presets = saved);
    replaced
}

fn footer(ok: &'static str) -> DialogFooter {
    DialogFooter::new()
        .child(DialogClose::new().trigger(|button| button.outline().label("Cancel")))
        .child(DialogAction::new().child(Button::new("dialog-ok").primary().label(ok)))
}

/// A name for a new preset that no preset of your own has yet ("Preset 3").
fn unused_name(existing: &[Preset]) -> String {
    (1..).map(|number| format!("Preset {number}")).find(|name| !existing.iter().any(|preset| &preset.name == name)).unwrap_or_else(|| "Preset".to_owned())
}

/// Settings ▸ New Preset: saves the photo's Develop settings as a preset of your own, with the
/// groups of settings it should set ticked.
pub fn new_preset(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let Some(develop) = workspace.state.edit_settings().map(|edit| edit.develop.clone()) else { return };
    let name = cx.new(|cx| InputState::new(window, cx).default_value(unused_name(&workspace.ui.settings.user_presets)));
    name.update(cx, |state, cx| {
        state.focus(window, cx);
        state.select_all(window, cx);
    });
    let parts = Rc::new(Cell::new(PresetParts::default()));
    let target = cx.weak_entity();
    super::chrome::open_dialog(window, cx, move |dialog, _, _| {
        let current = parts.get();
        let boxes = PresetParts::named().into_iter().enumerate().map(|(index, (label, flag))| {
            let parts = parts.clone();
            let mut probe = current;
            let checked = *flag(&mut probe);
            Checkbox::new(("preset-part", index)).label(label).checked(checked).on_click(move |checked: &bool, window, _| {
                let mut changed = parts.get();
                *flag(&mut changed) = *checked;
                parts.set(changed);
                window.refresh();
            })
        });
        let (name, parts, develop, target) = (name.clone(), parts.clone(), develop.clone(), target.clone());
        dialog
            .title("New Preset")
            .w(px(440.0))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(div().text_color(hsla(color::MUTED)).child("Saves this photo's Develop settings as a look to use on any photo. Only the ticked groups are set when it is applied; the crop and masks are never part of a preset."))
                    .child(div().flex().items_center().gap_3().child(div().w(px(60.0)).flex_none().child("Name")).child(div().flex_1().child(Input::new(&name))))
                    .child(div().grid().grid_cols(2).gap_2().children(boxes)),
            )
            .footer(footer("Create"))
            .on_ok(move |_, window, cx| {
                let typed = name.read(cx).value().trim().chars().take(100).collect::<String>();
                if typed.is_empty() {
                    window.push_notification("Give the preset a name.", cx);
                    return false;
                }
                if parts.get() == PresetParts::NONE {
                    window.push_notification("Tick at least one group of settings for the preset to set.", cx);
                    return false;
                }
                let preset = Preset::new(typed, USER_GROUP, develop.clone(), parts.get());
                target
                    .update(cx, |this, cx| {
                        add_user_preset(this, preset);
                        this.show_side(crate::workspace::LeftPanel::Presets);
                        cx.notify();
                    })
                    .ok();
                true
            })
    });
}

/// Develop ▸ About Develop.
pub fn about(_workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    super::chrome::open_alert_dialog(window, cx, |dialog, _, _| {
        dialog.title(format!("Omina Develop {}", env!("CARGO_PKG_VERSION"))).description(
            "Lightroom-style photo editing, one photo at a time, beside Omina Motion and Omina Edit. Free software under the GNU GPL, version 3 or later. Copyright © 2026 Odd Omens LLC.",
        )
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_presets_get_the_next_free_number() {
        let taken = vec![Preset::new("Preset 1", "User Presets", Default::default(), PresetParts::default()), Preset::new("Preset 3", "User Presets", Default::default(), PresetParts::default())];
        assert_eq!(unused_name(&[]), "Preset 1");
        assert_eq!(unused_name(&taken), "Preset 2");
    }
}

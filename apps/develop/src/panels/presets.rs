//! The Presets panel, as Lightroom's: the built-in looks and your own, grouped, each with a
//! small picture of this photo in that look. Clicking one applies it (one undo step); your own
//! can be deleted with a right-click. Pictures are made off the UI thread whenever the edit
//! settles, so they always show the look on the photo as it is now.

use std::collections::HashMap;
use std::sync::Arc;

use develop_core::Edit;
use develop_core::presets::{Preset, builtin};
use gpui_kit::assets::IconName;
use gpui_kit::component::{Disableable as _, Icon};
use gpui_kit::{
    AnyElement, AppContext as _, Context, ElementId, InteractiveElement, IntoElement, ParentElement, RenderImage, StatefulInteractiveElement, Styled, TestSupportExt as _, Window, div,
    img, prelude::FluentBuilder as _, px,
};

use super::chrome::{card_list, icon_button, list_row, panel};
use crate::theme::{color, hsla, size};
use crate::workspace::{PanelId, Workspace};

/// The longest side of a preset's picture, in pixels.
const PICTURE_SIDE: u32 = 112;

/// Pictures of the photo in each preset's look, and the edit they were made from.
#[derive(Default)]
pub struct PresetPictures {
    /// The photo (its original) and edit the pictures show.
    made_for: Option<(usize, Edit)>,
    pictures: HashMap<String, Arc<RenderImage>>,
    rendering: bool,
}

impl PresetPictures {
}

/// Every preset: the built-in ones, then your own.
pub fn all(workspace: &Workspace) -> Vec<Preset> {
    builtin().into_iter().chain(workspace.ui.settings.user_presets.iter().cloned()).collect()
}

/// A preset's key in the picture map.
fn key(preset: &Preset) -> String {
    format!("{}/{}", preset.group, preset.name)
}

/// Starts making pictures for the current edit when they're out of date and nothing is being
/// dragged (so a slider drag doesn't set off a render of every preset at each step).
fn refresh_pictures(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let Some(photo) = workspace.state.photo() else { return };
    let (source, edit) = (crate::preview::source_id(photo), photo.edit.clone());
    let busy = workspace.ui.slider_pressed || workspace.state.interaction.is_some() || cx.has_active_drag();
    let pictures = &workspace.ui.preset_pictures;
    if busy || pictures.rendering || pictures.made_for.as_ref().is_some_and(|(made_source, made_edit)| *made_source == source && *made_edit == edit) {
        return;
    }
    let Some(pyramid) = workspace.state.photo().and_then(|photo| workspace.ui.preview.pyramid_of(photo)) else { return };
    let presets = all(workspace);
    workspace.ui.preset_pictures.rendering = true;
    cx.spawn_in(window, async move |this, cx| {
        let made_for = (source, edit.clone());
        let made = cx
            .background_spawn(async move {
                presets
                    .iter()
                    .map(|preset| {
                        let looked = Edit { develop: preset.apply_to(&edit.develop), ..edit.clone() };
                        let bitmap = crate::preview::render_small(&pyramid, &looked, PICTURE_SIDE);
                        let longest = bitmap.width.max(bitmap.height).max(1);
                        let scale = (PICTURE_SIDE as f32 / longest as f32).min(1.0);
                        let small = bitmap.resized(((bitmap.width as f32 * scale).round() as u32).max(1), ((bitmap.height as f32 * scale).round() as u32).max(1));
                        (key(preset), crate::preview::to_render_image(&small, 0))
                    })
                    .collect::<HashMap<_, _>>()
            })
            .await;
        this.update_in(cx, |this, window, cx| {
            let pictures = &mut this.ui.preset_pictures;
            pictures.rendering = false;
            pictures.made_for = Some(made_for);
            let previous = std::mem::replace(&mut pictures.pictures, made);
            for image in previous.into_values() {
                cx.drop_image(image, Some(window));
            }
            cx.notify();
        })
        .ok();
    })
    .detach();
}

pub fn render(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    refresh_pictures(workspace, window, cx);
    let presets = all(workspace);
    let open = workspace.state.photo().is_some();
    let mut list = card_list("presets-list");
    if !open {
        list = list.child(list_row().text_color(hsla(color::MUTED)).child("Open a photo to try its looks."));
    }
    let builtin_count = builtin().len();
    // Groups in the order they first appear, your own last.
    let mut groups: Vec<String> = Vec::new();
    for preset in &presets {
        if !groups.contains(&preset.group) {
            groups.push(preset.group.clone());
        }
    }
    for group in groups {
        list = list.child(div().px_2p5().pt_2p5().pb_1().text_size(px(size::SMALL_TEXT)).font_weight(gpui_kit::FontWeight::SEMIBOLD).text_color(hsla(color::MUTED)).child(group.to_uppercase()));
        for (index, preset) in presets.iter().enumerate().filter(|(_, preset)| preset.group == group) {
            let user = index.checked_sub(builtin_count);
            let picture = workspace.ui.preset_pictures.pictures.get(&key(preset)).cloned();
            list = list.child(preset_row(index, preset, picture, user, open, cx));
        }
    }
    let footer = div()
        .size_full()
        .flex()
        .items_center()
        .gap_1()
        .child(div().flex_1().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(super::chrome::counted(presets.len(), "preset", "presets")))
        // A preset is made from a photo's settings, so there is nothing to make one from until a photo is open.
        .child(icon_button("presets-new", IconName::Plus, crate::keymap::with_key("New Preset from this photo's settings", "new-preset"), false, cx.listener(|this, _, window, cx| super::dialogs::new_preset(this, window, cx))).disabled(!open));
    panel(PanelId::Side, &[], Some(footer.into_any_element()), list, workspace, cx)
}

/// One preset: its picture and name. A click applies it; your own offer Delete on a right-click.
fn preset_row(index: usize, preset: &Preset, picture: Option<Arc<RenderImage>>, user: Option<usize>, open: bool, cx: &mut Context<Workspace>) -> AnyElement {
    use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenuItem};
    let (applied, hovered_preset) = (preset.clone(), preset.clone());
    let thumbnail = div()
        .w(px(44.0))
        .h(px(32.0))
        .flex_none()
        .rounded(px(3.0))
        .overflow_hidden()
        .bg(hsla(color::INSET))
        .flex()
        .items_center()
        .justify_center()
        .child(match picture {
            Some(picture) => gpui_kit::StyledImage::object_fit(img(picture).size_full(), gpui_kit::ObjectFit::Cover).into_any_element(),
            None => Icon::new(IconName::Image).size(px(12.0)).text_color(hsla(color::MUTED)).into_any_element(),
        });
    let row = div()
        .id(ElementId::Name(format!("preset-{index}").into()))
        .h(px(40.0))
        .flex_none()
        .flex()
        .items_center()
        .gap_2p5()
        .px_2p5()
        .when(open, |row| row.cursor_pointer().hover(|style| style.bg(hsla(color::HOVER))))
        .when(!open, |row| row.opacity(0.5))
        // Resting the pointer on a preset shows it on the photo, as in Lightroom.
        .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
            this.ui.hover_edit = if *hovered { this.state.photo().map(|photo| (crate::preview::source_id(photo), develop_core::Edit { develop: hovered_preset.apply_to(&photo.edit.develop), ..photo.edit.clone() })) } else { None };
            cx.notify();
        }))
        .on_click(cx.listener(move |this, _, window, cx| {
            this.commit_edit(window, cx);
            this.state.apply_preset(&applied);
            this.ui.hover_edit = None;
            cx.notify();
        }))
        .child(thumbnail)
        .child(div().flex_1().min_w_0().truncate().child(preset.name.clone()))
        .test_support();
    match user {
        Some(user) => {
            let target = cx.weak_entity();
            row.context_menu(move |menu, _, _| {
                let target = target.clone();
                menu.item(PopupMenuItem::new("Delete Preset").on_click(move |_, _, cx| {
                    target
                        .update(cx, |this, cx| {
                            if user < this.ui.settings.user_presets.len() {
                                this.ui.settings.user_presets.remove(user);
                                let presets = this.ui.settings.user_presets.clone();
                                crate::app_settings::update(|settings| settings.user_presets = presets);
                            }
                            cx.notify();
                        })
                        .ok();
                }))
            })
            .into_any_element()
        }
        None => row.into_any_element(),
    }
}

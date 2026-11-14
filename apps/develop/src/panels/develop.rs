//! The Develop panel, laid out as Lightroom's: the histogram, then Masks, Basic (white
//! balance, tone and presence), Tone Curve, Color Mixer, Color Grading, Detail, Effects and
//! Crop, each a folding card of sliders on colored tracks. Nothing here changes the photo's
//! pixels; the viewer renders the result from the original.

use develop_core::adjust::{CurvePoints, Track};
use develop_core::analysis::Histogram;
use develop_core::develop::{Develop, DevelopSetting, MIXER_BANDS};
use develop_core::masks::{MASK_SETTINGS, MaskShape};
use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::{
    AnyElement, Context, ElementId, InteractiveElement, IntoElement, ParentElement, PathBuilder, StatefulInteractiveElement, Styled, TestSupportExt as _, Window, canvas, div, point,
    prelude::FluentBuilder as _, px,
};

use super::chrome::{CardHeading, card, card_list, heading_button, list_row, panel};
use super::curves::{CurveAccess, editor};
use super::fields::{FieldTarget, group_heading, segmented, slider};
use crate::features::Feature;
use crate::state::{MaskKind, Tool};
use crate::theme::{color, hsla, size};
use crate::workspace::{EditTarget, PanelId, Workspace};

/// A folding card whose open state is remembered by `key`.
pub fn section(key: &str, title: &str, icon: Option<IconName>, trailing: Vec<Option<AnyElement>>, body: impl IntoElement, workspace: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let open = workspace.ui.section_open(key, false);
    let toggle_key = key.to_owned();
    card(
        CardHeading { id: ElementId::Name(format!("card-{key}").into()), icon, title: title.to_owned().into(), info: None, open, muted: false, trailing },
        cx.listener(move |this, _, _, cx| {
            this.ui.toggle_section(&toggle_key);
            cx.notify();
        }),
        Some(div().flex().flex_col().gap_1().child(body).into_any_element()),
    )
    .into_any_element()
}

/// The histogram drawn as three overlapping color areas, as Lightroom shows it, with marks at
/// either end that light up when the photo has clipped shadows or highlights.
/// A click on either clipping mark (or J) paints the clipped parts of the photo, so it's plain
/// where detail is lost. The marks go with the Clipping Warning when that is turned off.
fn histogram_view(histogram: &Histogram, showing_clipping: bool, clipping_on: bool, cx: &mut Context<Workspace>) -> AnyElement {
    let channels: Vec<(Vec<f32>, u32)> = vec![
        (histogram.red.iter().map(|&v| v as f32).collect(), 0xff3b30),
        (histogram.green.iter().map(|&v| v as f32).collect(), 0x34c759),
        (histogram.blue.iter().map(|&v| v as f32).collect(), 0x0a84ff),
    ];
    // The tallest bar sets the scale, ignoring the very ends (where clipping piles up).
    let peak = channels.iter().flat_map(|(levels, _)| levels[1..255].iter().copied()).fold(1.0f32, f32::max);
    let total = histogram.total.max(1) as f32;
    let clipped = |level: usize| (histogram.red[level] + histogram.green[level] + histogram.blue[level]) as f32 / (total * 3.0) > 0.002;
    let (shadows_clipped, highlights_clipped) = (clipped(0), clipped(255));
    let picture = canvas(|_, _, _| (), move |area, _, window, _| {
        let (left, top, width, height) = (area.origin.x, area.origin.y, area.size.width, area.size.height);
        for (levels, tint) in &channels {
            let mut shape = PathBuilder::fill();
            shape.move_to(point(left, top + height));
            for (index, count) in levels.iter().enumerate() {
                let x = left + width * (index as f32 / 255.0);
                shape.line_to(point(x, top + height * (1.0 - (count / peak).min(1.0).sqrt())));
            }
            shape.line_to(point(left + width, top + height));
            shape.close();
            if let Ok(path) = shape.build() {
                window.paint_path(path, gpui_kit::Hsla::from(gpui_kit::rgba((tint << 8) | 0x60)));
            }
        }
    })
    .absolute()
    .size_full();
    let clip_mark = |lit: bool, left: bool, tip: &'static str| {
        let tip = format!("{tip}. Click to {} clipping on the photo (J).", if showing_clipping { "stop showing" } else { "show" });
        div()
            .id(if left { "histogram-shadows" } else { "histogram-highlights" })
            .cursor_pointer()
            .when(showing_clipping, |mark| mark.border_color(hsla(color::ACCENT)))
            .on_click(cx.listener(|this, _, _, cx| {
                this.ui.show_clipping = !this.ui.show_clipping;
                cx.notify();
            }))
            .absolute()
            .top(px(4.0))
            .when(left, |mark| mark.left(px(4.0)))
            .when(!left, |mark| mark.right(px(4.0)))
            .size(px(8.0))
            .rounded(px(2.0))
            .border_1()
            .border_color(hsla(color::MUTED))
            .when(lit, |mark| mark.bg(gpui_kit::white()))
            .tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(tip.clone()).build(window, cx))
    };
    div()
        .relative()
        .w_full()
        .h(px(96.0))
        .rounded(px(4.0))
        .bg(hsla(color::INSET))
        .border_1()
        .border_color(hsla(color::HAIRLINE))
        .child(picture)
        .when(clipping_on, |view| {
            view.child(clip_mark(shadows_clipped, true, "Lit when some shadows are pure black")).child(clip_mark(highlights_clipped, false, "Lit when some highlights are pure white"))
        })
        .into_any_element()
}

fn sliders(settings: &[DevelopSetting], workspace: &Workspace, cx: &mut Context<Workspace>) -> Vec<AnyElement> {
    settings
        .iter()
        .map(|setting| {
            let info = setting.slider();
            slider(FieldTarget::Develop(*setting), info.name, info.track, workspace, cx)
        })
        .collect()
}

/// The tone curve's channels: the overall curve, then red, green and blue.
fn tone_curve_access(channel: usize) -> CurveAccess {
    fn pick(develop: &mut Develop, channel: usize) -> &mut CurvePoints {
        match channel {
            0 => &mut develop.curve,
            1 => &mut develop.curve_red,
            2 => &mut develop.curve_green,
            _ => &mut develop.curve_blue,
        }
    }
    CurveAccess::new(
        move |workspace| workspace.state.edit_settings().map(|edit| pick(&mut edit.develop.clone(), channel).clone()),
        move |workspace, points| workspace.state.edit_develop("Tone Curve", |develop| *pick(develop, channel) = points.clone()),
    )
}

/// A plain text button, as on Lightroom's panels ("Auto", "Reset").
fn text_button(id: &'static str, label: &'static str, tip: &'static str, on_click: impl Fn(&mut Workspace, &mut Context<Workspace>) + 'static, cx: &mut Context<Workspace>) -> AnyElement {
    use gpui_kit::component::Sizable as _;
    use gpui_kit::component::button::Button;
    Button::new(id)
        .outline()
        .xsmall()
        .label(label)
        .tooltip(tip)
        .on_click(cx.listener(move |this, _, _, cx| {
            on_click(this, cx);
            cx.notify();
        }))
        .into_any_element()
}

pub fn render(workspace: &mut Workspace, _window: &mut Window, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let body = if workspace.state.photo().is_none() {
        div()
            .child(list_row().flex().flex_col().gap_2().child(div().text_color(hsla(color::TEXT)).child("No photo open")).child(div().text_color(hsla(color::MUTED)).child("Open a photo to adjust its white balance, light, color and detail, crop it, and add local adjustments with masks.")))
            .into_any_element()
    } else {
        card_list("develop-cards").children(cards(workspace, cx)).into_any_element()
    };
    panel(PanelId::Develop, &[], None, body, workspace, cx)
}

/// The cards, leaving out those whose feature is turned off (Settings ▸ Tools & Features). What a
/// hidden card already set stays on the photo.
fn cards(workspace: &mut Workspace, cx: &mut Context<Workspace>) -> Vec<AnyElement> {
    let mut cards: Vec<AnyElement> = Vec::new();
    let settings = workspace.ui.settings.clone();
    let on = |feature: Feature| settings.feature_on(feature);
    let histogram = workspace.state.photo().and_then(|photo| workspace.ui.preview.histogram_of(photo));
    let auto = heading_button("develop-auto", IconName::WandSparkles, crate::keymap::with_key("Auto: set the light from the photo", "auto"), false, cx.listener(|this, _, _, cx| {
        this.auto();
        cx.notify();
    }))
    .into_any_element();
    let reset = heading_button("develop-reset", IconName::RotateCcw, "Reset every Develop setting (the crop and masks stay)", false, cx.listener(|this, _, _, cx| {
        this.state.reset_develop();
        cx.notify();
    }))
    .into_any_element();
    let info = workspace.state.photo().map(|photo| {
        let (width, height) = photo.source.size();
        div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).truncate().child(format!("{} · {width} × {height}", photo.source.file_name))
    });
    let histogram_body = div().flex().flex_col().gap_1p5().child(match histogram.as_deref() {
        Some(histogram) => histogram_view(histogram, workspace.ui.show_clipping, on(Feature::Clipping), cx),
        None => div().h(px(96.0)).rounded(px(4.0)).bg(hsla(color::INSET)).into_any_element(),
    })
    .children(info);
    cards.push(section("develop-histogram", "Histogram", Some(IconName::ChartColumn), vec![Some(auto), Some(reset)], histogram_body, workspace, cx));

    if on(Feature::Masks) {
        cards.push(masks_card(workspace, cx));
    }

    let picker = on(Feature::WhiteBalanceSelector).then(|| heading_button("develop-wb-picker", IconName::Pipette, crate::keymap::with_key("White Balance Selector: click something neutral", "white-balance-tool"), workspace.state.tool == Tool::WhiteBalance, cx.listener(|this, _, _, cx| {
        let next = if this.state.tool == Tool::WhiteBalance { Tool::View } else { Tool::WhiteBalance };
        this.state.set_tool(next);
        cx.notify();
    })));
    let basic = div()
        .flex()
        .flex_col()
        .child(div().flex().items_center().child(div().flex_1().child(group_heading("White Balance"))).children(picker))
        .children(sliders(&DevelopSetting::BASIC[0..2], workspace, cx))
        .child(div().flex().items_center().child(div().flex_1().child(group_heading("Tone"))).child(text_button("develop-tone-auto", "Auto", "Set exposure, contrast, highlights, shadows, whites and blacks from the photo", |this, _| this.auto(), cx)))
        .children(sliders(&DevelopSetting::BASIC[2..8], workspace, cx))
        .child(group_heading("Presence"))
        .children(sliders(&DevelopSetting::BASIC[8..13], workspace, cx));
    cards.push(section("develop-basic", "Basic", Some(IconName::SunMedium), vec![], basic, workspace, cx));
    if on(Feature::ToneCurve) {
        cards.push(tone_curve_card(workspace, histogram.as_deref(), cx));
    }
    if on(Feature::ColorMixer) {
        cards.push(mixer_card(workspace, cx));
    }
    if on(Feature::ColorGrading) {
        cards.push(grading_card(workspace, cx));
    }
    if on(Feature::Detail) {
        let detail = div().flex().flex_col().child(group_heading("Sharpening")).children(sliders(&DevelopSetting::DETAIL[0..2], workspace, cx)).child(group_heading("Noise Reduction")).children(sliders(&DevelopSetting::DETAIL[2..4], workspace, cx));
        cards.push(section("develop-detail", "Detail", Some(IconName::Triangle), vec![], detail, workspace, cx));
    }
    if on(Feature::Effects) {
        let effects = div().flex().flex_col().child(group_heading("Post-Crop Vignette")).children(sliders(&DevelopSetting::EFFECTS[0..3], workspace, cx)).child(group_heading("Grain")).children(sliders(&DevelopSetting::EFFECTS[3..5], workspace, cx));
        cards.push(section("develop-effects", "Effects", Some(IconName::Aperture), vec![], effects, workspace, cx));
    }
    if on(Feature::Crop) {
        cards.push(crop_card(workspace, cx));
    }
    cards
}

/// The Tone Curve card: the curve for the whole photo or one channel, over its histogram.
fn tone_curve_card(workspace: &Workspace, histogram: Option<&Histogram>, cx: &mut Context<Workspace>) -> AnyElement {
    let channel = workspace.ui.curve_channel;
    let channels = [(0usize, "RGB"), (1, "Red"), (2, "Green"), (3, "Blue")];
    let line = [0xf0f0f0, 0xff5050, 0x50e070, 0x5090ff][channel];
    let levels = histogram.map(|histogram| histogram.luma.iter().map(|&count| count as f32).collect());
    let curve = div()
        .flex()
        .flex_col()
        .gap_2()
        .child(segmented("tone-curve-channel", &channels, channel, |this, channel| this.ui.curve_channel = channel, cx))
        .child(editor("tone-curve", tone_curve_access(channel), line, levels, workspace, cx))
        .child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child("Click to add a point, drag to move it, double-click to remove it."));
    let reset_curve = heading_button("develop-curve-reset", IconName::RotateCcw, "Straighten every curve", false, cx.listener(|this, _, _, cx| {
        this.state.edit_develop("Reset Tone Curve", |develop| {
            let straight = Develop::default();
            develop.curve = straight.curve.clone();
            develop.curve_red = straight.curve_red.clone();
            develop.curve_green = straight.curve_green.clone();
            develop.curve_blue = straight.curve_blue;
        });
        cx.notify();
    }))
    .into_any_element();
    section("develop-curve", "Tone Curve", Some(IconName::Spline), vec![Some(reset_curve)], curve, workspace, cx)
}

/// The Color Mixer card: hue, saturation or luminance of each color band.
fn mixer_card(workspace: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let mixer_tab = workspace.ui.mixer_tab;
    let tabs = [(0usize, "Hue"), (1, "Saturation"), (2, "Luminance")];
    let mixer_settings: Vec<DevelopSetting> = (0..MIXER_BANDS.len())
        .map(|band| match mixer_tab {
            0 => DevelopSetting::MixerHue(band),
            1 => DevelopSetting::MixerSaturation(band),
            _ => DevelopSetting::MixerLuminance(band),
        })
        .collect();
    let mixer = div().flex().flex_col().gap_1().child(segmented("mixer-tab", &tabs, mixer_tab, |this, tab| this.ui.mixer_tab = tab, cx)).children(sliders(&mixer_settings, workspace, cx));
    section("develop-mixer", "Color Mixer", Some(IconName::Palette), vec![], mixer, workspace, cx)
}

/// The Color Grading card: a tint for the shadows, midtones and highlights, and their balance.
fn grading_card(workspace: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let mut grading = div().flex().flex_col();
    for (wheel, name) in ["Shadows", "Midtones", "Highlights"].into_iter().enumerate() {
        grading = grading.child(group_heading(name));
        for (value, label, track) in [(0usize, "Hue", Track::Hue), (1, "Saturation", Track::Saturation), (2, "Luminance", Track::Light)] {
            grading = grading.child(slider(FieldTarget::Develop(DevelopSetting::Grade(wheel, value)), label, track, workspace, cx));
        }
    }
    grading = grading.child(group_heading("Balance")).children(sliders(&[DevelopSetting::GradeBlending, DevelopSetting::GradeBalance], workspace, cx));
    section("develop-grading", "Color Grading", Some(IconName::Blend), vec![], grading, workspace, cx)
}

/// The Crop card: straightening and turning without entering Crop mode, and the way into it.
fn crop_card(workspace: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let cropping = workspace.state.tool == Tool::Crop;
    let body = div()
        .flex()
        .flex_col()
        .gap_1()
        .child(slider(FieldTarget::CropAngle, "Straighten", Track::Plain, workspace, cx))
        .child(
            div()
                .pt_1()
                .flex()
                .flex_wrap()
                .gap_1p5()
                .child(text_button("crop-mode", if cropping { "Done Cropping" } else { "Crop & Straighten" }, "Crop & Straighten (R)", |this, _| {
                    let next = if this.state.tool == Tool::Crop { Tool::View } else { Tool::Crop };
                    this.state.set_tool(next);
                }, cx))
                .child(text_button("crop-left", "Rotate Left", "Rotate Left (⌘[)", |this, _| this.state.rotate(false), cx))
                .child(text_button("crop-right", "Rotate Right", "Rotate Right (⌘])", |this, _| this.state.rotate(true), cx))
                .child(text_button("crop-reset-card", "Reset", "Undo the crop and straightening", |this, _| this.state.reset_crop(), cx)),
        );
    section("develop-crop", "Crop", Some(IconName::Crop), vec![], body, workspace, cx)
}

/// The Masks card: the photo's masks, buttons to add one, and the selected mask's settings.
fn masks_card(workspace: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let Some(edit) = workspace.state.edit_settings() else { return div().into_any_element() };
    let masks = &edit.masks;
    let selected = workspace.state.selected_mask.filter(|index| *index < masks.len());
    let masking = workspace.state.tool == Tool::Masks;
    let add = |id: &'static str, icon: IconName, tip: String, kind: MaskKind, cx: &mut Context<Workspace>| {
        heading_button(id, icon, tip, masking && workspace.state.mask_kind == kind, cx.listener(move |this, _, _, cx| {
            this.state.use_mask_tool(kind);
            cx.notify();
        }))
        .into_any_element()
    };
    let linear = add("mask-add-linear", IconName::SquareSplitVertical, crate::keymap::with_key("Linear Gradient: drag across the photo", "linear-mask-tool"), MaskKind::Linear, cx);
    let radial = add("mask-add-radial", IconName::CircleDashed, crate::keymap::with_key("Radial Gradient: drag out an ellipse", "radial-mask-tool"), MaskKind::Radial, cx);
    let rows = masks.iter().enumerate().map(|(index, mask)| {
        let is_selected = selected == Some(index);
        let name: AnyElement = match workspace.editing_field(EditTarget::MaskName(index)) {
            Some(field) => div().flex_1().child(field).into_any_element(),
            None => div().flex_1().min_w_0().truncate().child(mask.name.clone()).into_any_element(),
        };
        let icon = match mask.shape {
            MaskShape::Linear { .. } => IconName::SquareSplitVertical,
            MaskShape::Radial { .. } => IconName::CircleDashed,
        };
        div()
            .id(ElementId::Name(format!("mask-row-{index}").into()))
            .h(px(28.0))
            .flex()
            .items_center()
            .gap_2()
            .px_1p5()
            .rounded(px(4.0))
            .when(is_selected, |row| row.bg(hsla(color::ROW_SELECTED)))
            .hover(|style| style.bg(hsla(color::HOVER)))
            .text_color(hsla(if mask.enabled { color::TEXT } else { color::MUTED }))
            .on_click(cx.listener(move |this, event: &gpui_kit::ClickEvent, window, cx| {
                this.state.selected_mask = Some(index);
                this.state.set_tool(Tool::Masks);
                if event.click_count() >= 2 {
                    let name = this.state.edit_settings().and_then(|edit| edit.masks.get(index)).map(|mask| mask.name.clone()).unwrap_or_default();
                    this.begin_edit(EditTarget::MaskName(index), name, window, cx);
                }
                cx.notify();
            }))
            .child(Icon::new(icon).size(px(13.0)).text_color(hsla(color::MUTED)))
            .child(name)
            .child(super::chrome::icon_button(ElementId::Name(format!("mask-visible-{index}").into()), if mask.enabled { IconName::Eye } else { IconName::EyeOff }, if mask.enabled { "Turn this mask off" } else { "Turn this mask on" }, false, cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.state.edit_mask(index, "Show Mask", |mask| mask.enabled = !mask.enabled);
                cx.notify();
            })))
            .child(super::chrome::icon_button(ElementId::Name(format!("mask-duplicate-{index}").into()), IconName::Copy, "Duplicate this mask", false, cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.state.duplicate_mask(index);
                cx.notify();
            })))
            .child(super::chrome::icon_button(ElementId::Name(format!("mask-delete-{index}").into()), IconName::Trash, "Delete this mask (⌫)", false, cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.state.delete_mask(index);
                cx.notify();
            })))
            .test_support()
    });
    let mut body = div().flex().flex_col().gap_1();
    if masks.is_empty() {
        body = body.child(div().py_1().text_color(hsla(color::MUTED)).child("Adjust part of the photo: add a Linear Gradient (to darken a sky, say) or a Radial Gradient (to light a face), then drag on the photo."));
    }
    body = body.children(rows);
    // A mask's own sliders show only while masking, as in Lightroom; otherwise they would push
    // the photo's own sliders out of sight and look like them.
    if let Some(index) = selected.filter(|_| masking) {
        let mask = &masks[index];
        let mut settings = div().flex().flex_col().pt_1();
        settings = settings
            .child(div().flex().items_center().child(div().flex_1().child(group_heading(mask.shape.name()))).child(super::properties_checkbox(ElementId::Name(format!("mask-invert-{index}").into()), "Invert", mask.invert, move |this, invert| this.state.edit_mask(index, "Invert Mask", |mask| mask.invert = invert), cx)))
            .child(slider(FieldTarget::MaskAmount(index), "Amount", Track::Plain, workspace, cx));
        if matches!(mask.shape, MaskShape::Radial { .. }) {
            settings = settings.child(slider(FieldTarget::MaskFeather(index), "Feather", Track::Plain, workspace, cx)).child(slider(FieldTarget::MaskAngle(index), "Angle", Track::Plain, workspace, cx));
        }
        let groups: [(&str, std::ops::Range<usize>); 3] = [("Light", 2..8), ("Color", 0..2), ("Presence", 8..12)];
        for (title, range) in groups {
            settings = settings.child(group_heading(title));
            for setting in &MASK_SETTINGS[range] {
                let info = setting.slider();
                settings = settings.child(slider(FieldTarget::Mask(index, *setting), info.name, info.track, workspace, cx));
            }
        }
        let reset = div().pt_1().child(text_button("mask-reset", "Reset Mask Settings", "Every slider of this mask back to zero", move |this, _| {
            this.state.edit_mask(index, "Reset Mask", |mask| mask.settings = Develop::default());
        }, cx));
        body = body.child(div().mt_1().border_t_1().border_color(hsla(color::HAIRLINE)).child(settings).child(reset));
    }
    section("develop-masks", "Masks", Some(IconName::VenetianMask), vec![Some(linear), Some(radial)], body, workspace, cx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_slider_groups_cover_every_mask_setting_once() {
        let mut seen: Vec<usize> = [2..8, 0..2, 8..12].into_iter().flatten().collect();
        seen.sort();
        assert_eq!(seen, (0..MASK_SETTINGS.len()).collect::<Vec<_>>());
    }
}

//! Editable numbers, as in Studio and Canvas: the underlined values beside every slider. Drag
//! one sideways to scrub it (Shift for ten times faster, Alt for ten times finer), or click to
//! type a value. Sliders on colored tracks (as in Lightroom's Develop panel) edit the same
//! targets; double-click a slider or its name to put it back to its default.

use develop_core::adjust::{Slider, Track};
use develop_core::develop::DevelopSetting;
use develop_core::masks::MaskShape;
use gpui_kit::{
    AnyElement, AppContext as _, ClickEvent, Context, DragMoveEvent, ElementId, InteractiveElement, IntoElement, MouseButton, MouseDownEvent, ParentElement, StatefulInteractiveElement,
    Styled, TestSupportExt as _, div, linear_color_stop, linear_gradient, px, relative,
};

use super::chrome::EmptyDragView;
use crate::state::EditorState;
use crate::theme::{color, hsla, size};
use crate::workspace::{EditTarget, Workspace};

/// A number a field edits.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FieldTarget {
    /// One of the photo's Develop settings.
    Develop(DevelopSetting),
    /// One of mask `index`'s settings.
    Mask(usize, DevelopSetting),
    /// How much of mask `index`'s settings apply.
    MaskAmount(usize),
    /// A radial mask's soft edge.
    MaskFeather(usize),
    /// A radial mask's turn.
    MaskAngle(usize),
    /// The straighten angle.
    CropAngle,
}

/// How a field shows and limits its number.
#[derive(Clone, Copy, Debug)]
pub struct FieldFormat {
    /// How much the value changes per pixel of scrubbing.
    pub step: f32,
    pub decimals: usize,
    pub unit: &'static str,
    pub min: f32,
    pub max: f32,
}

const fn format(step: f32, decimals: usize, unit: &'static str, min: f32, max: f32) -> FieldFormat {
    FieldFormat { step, decimals, unit, min, max }
}

impl From<Slider> for FieldFormat {
    fn from(slider: Slider) -> Self {
        // A full drag across a 400-pixel stretch covers the range.
        let step = ((slider.max - slider.min) / 400.0).max(10f32.powi(-(slider.decimals as i32)));
        FieldFormat { step, decimals: slider.decimals, unit: slider.unit, min: slider.min, max: slider.max }
    }
}

/// Drag payload while scrubbing a value field.
#[derive(Clone, Copy, Debug)]
pub struct FieldDrag(pub FieldTarget);

impl FieldTarget {
    /// The current value and how to show it, or `None` when there is nothing to edit.
    pub fn read(self, state: &EditorState) -> Option<(f32, FieldFormat)> {
        let edit = state.edit_settings()?;
        Some(match self {
            Self::Develop(setting) => (setting.get(&edit.develop), setting.slider().into()),
            Self::Mask(index, setting) => (setting.get(&edit.masks.get(index)?.settings), setting.slider().into()),
            Self::MaskAmount(index) => (edit.masks.get(index)?.amount, format(0.5, 0, "", 0.0, 100.0)),
            Self::MaskFeather(index) => match edit.masks.get(index)?.shape {
                MaskShape::Radial { feather, .. } => (feather, format(0.5, 0, "", 0.0, 100.0)),
                MaskShape::Linear { .. } => return None,
            },
            Self::MaskAngle(index) => match edit.masks.get(index)?.shape {
                MaskShape::Radial { angle, .. } => (if angle > 180.0 { angle - 360.0 } else { angle }, format(0.5, 0, "°", -180.0, 180.0)),
                MaskShape::Linear { .. } => return None,
            },
            Self::CropAngle => (edit.crop.angle, format(0.05, 2, "°", -develop_core::crop::MAX_ANGLE, develop_core::crop::MAX_ANGLE)),
        })
    }

    /// Sets the value, kept within the field's range.
    pub fn write(self, state: &mut EditorState, value: f32) {
        let Some((current, format)) = self.read(state) else { return };
        let value = if value.is_finite() { value.clamp(format.min, format.max) } else { current };
        match self {
            Self::Develop(setting) => state.set_develop(setting, value),
            Self::Mask(index, setting) => state.edit_mask(index, setting.slider().name, |mask| setting.set(&mut mask.settings, value)),
            Self::MaskAmount(index) => state.edit_mask(index, "Amount", |mask| mask.amount = value),
            Self::MaskFeather(index) => state.edit_mask(index, "Feather", |mask| {
                if let MaskShape::Radial { feather, .. } = &mut mask.shape {
                    *feather = value;
                }
            }),
            Self::MaskAngle(index) => state.edit_mask(index, "Mask Angle", |mask| {
                if let MaskShape::Radial { angle, .. } = &mut mask.shape {
                    *angle = value.rem_euclid(360.0);
                }
            }),
            Self::CropAngle => state.set_angle(value),
        }
    }

    /// The value a double-click puts back.
    pub fn default_value(self, _state: &EditorState) -> Option<f32> {
        match self {
            Self::Develop(setting) | Self::Mask(_, setting) => Some(setting.slider().default),
            Self::MaskAmount(_) => Some(100.0),
            Self::MaskFeather(_) => Some(50.0),
            Self::MaskAngle(_) | Self::CropAngle => Some(0.0),
        }
    }

    fn element_id(self, prefix: &str) -> ElementId {
        ElementId::Name(format!("{prefix}:{self:?}").into())
    }
}

pub fn format_value(value: f32, format: FieldFormat) -> String {
    format!("{:.*}{}", format.decimals, value, format.unit)
}

/// Reads a typed value, ignoring units and spaces ("120 px", "45°", "50%").
pub fn parse_value(text: &str) -> Option<f32> {
    let cleaned: String = text.chars().filter(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '+')).collect();
    cleaned.parse::<f32>().ok().filter(|value| value.is_finite())
}

/// A value field. While it is being typed into, it shows a text field instead.
pub fn value_field(target: FieldTarget, workspace: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let Some((value, format)) = target.read(&workspace.state) else { return div().into_any_element() };
    if let Some(input) = workspace.ui.editing.as_ref().filter(|editing| editing.target == EditTarget::Number(target)).map(|editing| &editing.input) {
        use gpui_kit::component::Sizable as _;
        return div().w(px(72.0)).child(gpui_kit::component::input::Input::new(input).xsmall()).into_any_element();
    }
    div()
        .id(target.element_id("field"))
        .min_w(px(44.0))
        .px_0p5()
        .border_b_1()
        .border_color(hsla(color::HAIRLINE))
        .font_family(crate::theme::MONO_FONT)
        .text_color(hsla(color::VALUE))
        .cursor_ew_resize()
        .hover(|style| style.border_color(hsla(color::VALUE)))
        .on_mouse_down(MouseButton::Left, cx.listener(move |this, event: &MouseDownEvent, _, _| {
            if let Some((start, _)) = target.read(&this.state) {
                this.ui.field_anchor = Some((f32::from(event.position.x), start));
            }
        }))
        .on_drag(FieldDrag(target), |_, _, _, cx| cx.new(|_| EmptyDragView))
        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
            let text = target.read(&this.state).map(|(value, format)| format!("{:.*}", format.decimals, value)).unwrap_or_default();
            this.begin_edit(EditTarget::Number(target), text, window, cx);
        }))
        .child(format_value(value, format))
        .test_support()
        .into_any_element()
}


/// Drag payload while dragging along a slider.
#[derive(Clone, Copy, Debug)]
pub struct SliderDrag(pub FieldTarget);

/// The colors behind a slider's track.
fn track_bar(track: Track) -> gpui_kit::Div {
    let bar = div().absolute().left_0().right_0().top(px(6.0)).h(px(4.0)).rounded_full().overflow_hidden();
    let gradient = |from: u32, to: u32| linear_gradient(90.0, linear_color_stop(gpui_kit::rgb(from), 0.0), linear_color_stop(gpui_kit::rgb(to), 1.0));
    match track {
        Track::Hue => {
            const STOPS: [u32; 7] = [0xff0000, 0xffff00, 0x00ff00, 0x00ffff, 0x0000ff, 0xff00ff, 0xff0000];
            bar.flex().children(STOPS.windows(2).map(|pair| div().flex_1().h_full().bg(linear_gradient(90.0, linear_color_stop(gpui_kit::rgb(pair[0]), 0.0), linear_color_stop(gpui_kit::rgb(pair[1]), 1.0)))))
        }
        Track::Plain => bar.bg(hsla(color::HAIRLINE)),
        Track::Light => bar.bg(gradient(0x050505, 0xf2f2f2)),
        Track::Temperature => bar.bg(gradient(0x3f7fd9, 0xe3a33b)),
        Track::Tint => bar.bg(gradient(0x3fae5a, 0xc04fb8)),
        Track::Saturation => bar.bg(gradient(0x7a7a7a, 0xe2453c)),
    }
}

/// Sets a slider's target from where `x` falls along its track.
fn drag_slider(workspace: &mut Workspace, target: FieldTarget, x: f32) {
    let Some((_, format)) = target.read(&workspace.state) else { return };
    let key = format!("{target:?}");
    let track = workspace.ui.slider_bounds.borrow().get(&key).map(super::chrome::MeasuredBounds::get).unwrap_or_default();
    if track.width <= 0.0 {
        return;
    }
    let fraction = ((x - track.left) / track.width).clamp(0.0, 1.0);
    let scale = 10f32.powi(format.decimals as i32);
    let value = ((format.min + fraction * (format.max - format.min)) * scale).round() / scale;
    target.write(&mut workspace.state, value);
}

/// A slider: its name (brighter once changed; double-click resets), its value (scrub or type)
/// and its track (click or drag along it).
pub fn slider(target: FieldTarget, name: &str, track: Track, workspace: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let Some((value, format)) = target.read(&workspace.state) else { return div().into_any_element() };
    let default = target.default_value(&workspace.state);
    let changed = default.is_some_and(|default| (value - default).abs() > 1e-4);
    let span = (format.max - format.min).max(f32::EPSILON);
    let at = |value: f32| ((value - format.min) / span).clamp(0.0, 1.0);
    let key = format!("{target:?}");
    let bounds = workspace.ui.slider_bounds.borrow_mut().entry(key).or_default().clone();
    let label = div()
        .id(target.element_id("slider-label"))
        .flex_1()
        .min_w_0()
        .truncate()
        .text_color(hsla(if changed || default.is_none() { color::TEXT } else { color::MUTED }))
        .on_mouse_down(MouseButton::Left, cx.listener(move |this, event: &MouseDownEvent, _, cx| {
            if event.click_count >= 2
                && let Some(default) = target.default_value(&this.state)
            {
                target.write(&mut this.state, default);
                cx.notify();
            }
        }))
        .child(name.to_owned())
        .test_support();
    let default_mark = default.filter(|default| *default > format.min && *default < format.max).map(|default| div().absolute().top(px(3.0)).h(px(10.0)).w(px(1.0)).left(relative(at(default))).bg(hsla(color::MUTED)));
    let track_element = div()
        .id(target.element_id("slider"))
        .relative()
        .w_full()
        .h(px(16.0))
        .cursor_pointer()
        .child(bounds.probe())
        .child(track_bar(track))
        .children(default_mark)
        .child(div().absolute().top(px(2.0)).left(relative(at(value))).ml(px(-6.0)).size(px(12.0)).rounded_full().bg(gpui_kit::rgb(0xffffff)).border_1().border_color(gpui_kit::rgb(0x000000)).shadow_md())
        .on_mouse_down(MouseButton::Left, cx.listener(move |this, event: &MouseDownEvent, _, cx| {
            cx.stop_propagation();
            if event.click_count >= 2 {
                if let Some(default) = target.default_value(&this.state) {
                    target.write(&mut this.state, default);
                }
            } else {
                this.ui.slider_pressed = true;
                drag_slider(this, target, f32::from(event.position.x));
            }
            cx.notify();
        }))
        .on_drag(SliderDrag(target), |_, _, _, cx| cx.new(|_| EmptyDragView))
        .on_drag_move(cx.listener(move |this, event: &DragMoveEvent<SliderDrag>, _, cx| {
            if event.drag(cx).0 == target {
                drag_slider(this, target, f32::from(event.event.position.x));
                cx.notify();
            }
        }))
        .test_support();
    div()
        .flex()
        .flex_col()
        .py_0p5()
        .child(div().h(px(22.0)).flex().items_center().gap_2().child(label).child(value_field(target, workspace, cx)))
        .child(track_element)
        .into_any_element()
}

/// A small heading over a group of sliders.
pub fn group_heading(title: &str) -> gpui_kit::Div {
    div().flex().items_center().h(px(24.0)).mt_1().child(div().flex_1().text_size(px(size::SMALL_TEXT)).font_weight(gpui_kit::FontWeight::SEMIBOLD).text_color(hsla(color::MUTED)).child(title.to_owned()))
}

/// A row of small toggle buttons where one is picked (alignment, blend groups and so on).
pub fn segmented<T: Copy + PartialEq + 'static>(id: &str, choices: &[(T, &'static str)], current: T, pick: impl Fn(&mut Workspace, T) + 'static + Clone, cx: &mut Context<Workspace>) -> gpui_kit::Div {
    div().flex().items_center().gap_0p5().p_0p5().rounded(px(5.0)).bg(hsla(color::INSET)).children(choices.iter().enumerate().map(|(index, (value, label))| {
        let value = *value;
        let pick = pick.clone();
        div()
            .id(ElementId::Name(format!("{id}:{index}").into()))
            .px_1p5()
            .h(px(20.0))
            .flex()
            .items_center()
            .rounded(px(4.0))
            .text_size(px(size::SMALL_TEXT))
            .text_color(hsla(if value == current { color::TEXT } else { color::MUTED }))
            .when_some((value == current).then_some(()), |chip, _| chip.bg(hsla(color::RAISED)))
            .hover(|style| style.text_color(hsla(color::TEXT)))
            .on_click(cx.listener(move |this, _, _, cx| {
                pick(this, value);
                cx.notify();
            }))
            .child(*label)
            .test_support()
    }))
}

use gpui_kit::prelude::FluentBuilder as _;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_values_ignore_units() {
        assert_eq!(parse_value("120 px"), Some(120.0));
        assert_eq!(parse_value("-45°"), Some(-45.0));
        assert_eq!(parse_value("50%"), Some(50.0));
        assert_eq!(parse_value("abc"), None);
        assert_eq!(format_value(12.345, format(1.0, 1, " px", 0.0, 100.0)), "12.3 px");
    }

    #[test]
    fn fields_read_and_write_the_photo() {
        let mut state = crate::state::test_support::state();
        FieldTarget::Develop(DevelopSetting::Exposure).write(&mut state, 1.5);
        assert_eq!(FieldTarget::Develop(DevelopSetting::Exposure).read(&state).unwrap().0, 1.5);
        FieldTarget::Develop(DevelopSetting::Exposure).write(&mut state, 99.0);
        assert_eq!(state.edit_settings().unwrap().develop.exposure, 5.0, "kept in range");
        FieldTarget::CropAngle.write(&mut state, 12.0);
        assert_eq!(state.edit_settings().unwrap().crop.angle, 12.0);
        // Mask fields only exist while the mask does, and feather only for radial masks.
        assert!(FieldTarget::MaskAmount(0).read(&state).is_none());
        state.add_mask(MaskShape::radial(develop_core::Vec2::new(0.5, 0.5), develop_core::Vec2::new(0.2, 0.2)));
        FieldTarget::Mask(0, DevelopSetting::Shadows).write(&mut state, 40.0);
        FieldTarget::MaskAngle(0).write(&mut state, -30.0);
        let mask = &state.edit_settings().unwrap().masks[0];
        assert_eq!(mask.settings.shadows, 40.0);
        assert_eq!(FieldTarget::MaskAngle(0).read(&state).unwrap().0, -30.0);
        assert!(matches!(mask.shape, MaskShape::Radial { angle, .. } if angle == 330.0));
    }
}

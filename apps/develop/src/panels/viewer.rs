//! The photo panel: the edited photo, rendered off the UI thread by `preview` at the screen's
//! resolution, with the tools' marks on top (the crop box and its grid, masks and their
//! handles, the before / after divider), the tool's options above it and a status bar below.
//!
//! Everything here is placed in frame points (see `develop_core::crop`), the space the crop
//! rectangle lives in. Crop mode shows the whole straightened frame; otherwise the view shows
//! just the crop.

use develop_core::crop::ASPECTS;
use develop_core::masks::MaskShape;
use develop_core::{Rect, Vec2, View};
use gpui_kit::assets::IconName;
use gpui_kit::{
    AnyElement, AppContext as _, Context, CursorStyle, DragMoveEvent, Hsla, ImageSource, InteractiveElement, IntoElement, MouseButton, MouseDownEvent, ParentElement, PathBuilder,
    StatefulInteractiveElement, Styled, TestSupportExt as _, Window, canvas, div, img, point, prelude::FluentBuilder as _, px,
};

use super::chrome::{EmptyDragView, MeasuredBounds, icon_button, panel};
use super::fields::{FieldTarget, segmented, value_field};
use crate::preview::{self, Slot};
use crate::state::tools::{CropGrip, Keys, mask_pin, radial_handles};
use crate::state::{Compare, MaskKind, Tool};
use crate::theme::{color, hsla, size};
use crate::workspace::{PanelId, Workspace};

/// Space kept between the photo and the panel's edge when fitting.
const FIT_PADDING: f32 = 28.0;
/// The magnifications the zoom steps go through.
const ZOOM_LEVELS: [f32; 13] = [0.0625, 0.125, 0.1667, 0.25, 0.3333, 0.5, 0.6667, 1.0, 1.5, 2.0, 3.0, 4.0, 8.0];
/// How far (in screen points) a click may miss a handle.
const GRAB: f32 = 9.0;

/// The viewer's view and what is going on in it.
pub struct ViewerUi {
    pub bounds: MeasuredBounds,
    /// Screen points per image pixel; `None` fits the photo in the panel.
    pub zoom: Option<f32>,
    /// How far the photo's center sits from the panel's center, in screen points.
    pub pan: Vec2,
    /// Where the pointer is over the photo, in frame points.
    pub pointer: Option<Vec2>,
    /// A pan in progress: the pointer and the pan at its start.
    pub hand: Option<(Vec2, Vec2)>,
    /// Whether the before / after divider is being dragged.
    pub dragging_split: bool,
}

impl Default for ViewerUi {
    fn default() -> Self {
        Self { bounds: MeasuredBounds::default(), zoom: None, pan: Vec2::ZERO, pointer: None, hand: None, dragging_split: false }
    }
}

/// Drag payload while dragging on the photo.
#[derive(Clone, Copy, Debug)]
struct PhotoDrag;

/// What the view shows: the whole straightened frame while cropping, else the crop.
fn view_kind(workspace: &Workspace) -> View {
    if workspace.state.tool == Tool::Crop { View::Uncropped } else { View::Result }
}

/// The frame area on show, if a photo is open.
fn area(workspace: &Workspace) -> Option<Rect> {
    let size = workspace.state.image_size()?;
    Some(workspace.state.edit_settings()?.area(view_kind(workspace), size))
}

/// Screen points per frame pixel that fit the shown area in the panel.
fn fit_scale(workspace: &Workspace) -> f32 {
    let Some(area) = area(workspace) else { return 1.0 };
    let bounds = workspace.ui.viewer.bounds.get();
    ((bounds.width - FIT_PADDING * 2.0) / area.width.max(1.0)).min((bounds.height - FIT_PADDING * 2.0) / area.height.max(1.0)).clamp(0.01, 4.0)
}

/// Screen points per frame pixel.
pub fn scale(workspace: &Workspace) -> f32 {
    workspace.ui.viewer.zoom.unwrap_or_else(|| fit_scale(workspace))
}

/// Where the shown area's top-left corner is, in window coordinates.
fn origin(workspace: &Workspace) -> Vec2 {
    let bounds = workspace.ui.viewer.bounds.get();
    let area = area(workspace).unwrap_or_default();
    let s = scale(workspace);
    Vec2::new(bounds.left + bounds.width * 0.5, bounds.top + bounds.height * 0.5) + workspace.ui.viewer.pan - area.size() * (s * 0.5)
}

/// A window position as a frame point.
pub fn to_frame(workspace: &Workspace, x: f32, y: f32) -> Vec2 {
    let s = scale(workspace).max(1e-6);
    let area = area(workspace).unwrap_or_default();
    area.min() + (Vec2::new(x, y) - origin(workspace)) * (1.0 / s)
}

/// A frame point as a position inside the panel's body (for absolutely placed marks).
fn to_local(workspace: &Workspace, point: Vec2) -> Vec2 {
    let bounds = workspace.ui.viewer.bounds.get();
    let area = area(workspace).unwrap_or_default();
    origin(workspace) + (point - area.min()) * scale(workspace) - Vec2::new(bounds.left, bounds.top)
}

/// The frame point in the middle of the panel.
fn view_center(workspace: &Workspace) -> Vec2 {
    let bounds = workspace.ui.viewer.bounds.get();
    to_frame(workspace, bounds.left + bounds.width * 0.5, bounds.top + bounds.height * 0.5)
}

/// Sets the zoom (`None` fits), centering the photo.
pub fn set_zoom(workspace: &mut Workspace, zoom: Option<f32>) {
    workspace.ui.viewer.zoom = zoom;
    workspace.ui.viewer.pan = Vec2::ZERO;
}

/// Zooms to `next`, keeping the frame point `at` where it is on screen.
fn zoom_to(workspace: &mut Workspace, at: Vec2, next: f32) {
    let current = scale(workspace);
    let next = next.clamp(ZOOM_LEVELS[0], ZOOM_LEVELS[ZOOM_LEVELS.len() - 1]);
    if !next.is_finite() || (next - current).abs() < 1e-6 {
        return;
    }
    let Some(area) = area(workspace) else { return };
    let middle = area.center();
    workspace.ui.viewer.pan += (at - middle) * (current - next);
    workspace.ui.viewer.zoom = Some(next);
}

/// ⌘= and ⌘-: zooms one step around the middle of the view.
pub fn zoom_step(workspace: &mut Workspace, zoom_in: bool) {
    let current = scale(workspace);
    let next = if zoom_in { ZOOM_LEVELS.iter().copied().find(|&level| level > current * 1.01) } else { ZOOM_LEVELS.iter().rev().copied().find(|&level| level < current * 0.99) };
    let center = view_center(workspace);
    if let Some(next) = next {
        zoom_to(workspace, center, next);
    }
}

/// Z: between fitting the photo and 100%, as Lightroom's Z does.
pub fn toggle_zoom(workspace: &mut Workspace) {
    if workspace.ui.viewer.zoom.is_some() {
        set_zoom(workspace, None);
    } else {
        let center = view_center(workspace);
        zoom_to(workspace, center, 1.0);
    }
}

/// The keys held, from a GPUI event's modifiers.
fn keys(modifiers: gpui_kit::Modifiers) -> Keys {
    Keys { shift: modifiers.shift, alt: modifiers.alt }
}

/// The before / after divider's place in the panel, in points from the panel's left edge.
fn split_x(workspace: &Workspace) -> Option<f32> {
    let area = area(workspace)?;
    let left = to_local(workspace, area.min()).x;
    let right = to_local(workspace, area.max()).x;
    Some(left + (right - left) * workspace.state.split_at)
}

/// Whether the photo is compared in two halves now (never while cropping).
fn splitting(workspace: &Workspace) -> bool {
    workspace.state.compare == Compare::Split && workspace.state.tool != Tool::Crop
}

/// A press on the photo.
fn press(workspace: &mut Workspace, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Workspace>) {
    workspace.commit_edit(window, cx);
    let position = Vec2::new(f32::from(event.position.x), f32::from(event.position.y));
    let point = to_frame(workspace, position.x, position.y);
    let s = scale(workspace).max(1e-6);
    // The before / after divider.
    if splitting(workspace)
        && let Some(x) = split_x(workspace)
        && (position.x - workspace.ui.viewer.bounds.get().left - x).abs() <= GRAB
    {
        workspace.ui.viewer.dragging_split = true;
        return;
    }
    // A double-click switches between fitting and 100%.
    if event.click_count >= 2 && matches!(workspace.state.tool, Tool::View) {
        if workspace.ui.viewer.zoom.is_some() {
            set_zoom(workspace, None);
        } else {
            zoom_to(workspace, point, 1.0);
        }
        workspace.notify_canvas(cx);
        return;
    }
    match workspace.state.tool {
        Tool::WhiteBalance => {
            let color = workspace.state.frame_to_image(point).and_then(|at| workspace.ui.preview.pyramid_of(workspace.state.photo()?)?.sample(at));
            if let Some(color) = color {
                workspace.state.pick_white_balance(color);
                // As in Lightroom, the selector puts itself away after a pick.
                workspace.state.set_tool(Tool::View);
            }
            cx.notify();
        }
        Tool::Crop | Tool::Masks => {
            if workspace.state.press(point, GRAB / s) {
                cx.notify();
            } else {
                workspace.ui.viewer.hand = Some((position, workspace.ui.viewer.pan));
            }
        }
        Tool::View => workspace.ui.viewer.hand = Some((position, workspace.ui.viewer.pan)),
    }
}

pub fn render(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let body: AnyElement = if workspace.state.photo().is_some() { photo_view(workspace, window, cx) } else { empty_view(cx) };
    let footer = workspace.state.photo().is_some().then(|| status_bar(workspace, cx).into_any_element());
    panel(PanelId::Canvas, &[], footer, div().size_full().flex().flex_col().children(options_bar(workspace, cx)).child(body), workspace, cx)
}

/// With no photo open: an invitation to open one, which also takes a dropped file.
fn empty_view(cx: &mut Context<Workspace>) -> AnyElement {
    use gpui_kit::component::button::{Button, ButtonVariants as _};
    div()
        .id("viewer-empty")
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_3()
        .bg(hsla(color::PASTEBOARD))
        .drag_over::<gpui_kit::ExternalPaths>(|style, _, _, _| style.bg(hsla(color::HOVER)))
        .on_drop(cx.listener(|this, paths: &gpui_kit::ExternalPaths, window, cx| {
            this.open_paths(paths.paths().to_vec(), window, cx);
        }))
        .child(gpui_kit::component::Icon::new(IconName::ImagePlus).size(px(36.0)).text_color(hsla(color::MUTED)))
        .child(div().text_color(hsla(color::MUTED)).child("Open a photo, or drop one here."))
        .child(Button::new("viewer-open").primary().label("Open Photo…").on_click(cx.listener(|this, _, window, cx| this.open(window, cx))))
        .test_support()
        .into_any_element()
}

/// The photo, its marks, and everything the pointer does over it.
fn photo_view(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) -> AnyElement {
    let backdrop = workspace.ui.settings.backdrop.color().unwrap_or_else(|| crate::theme::themed(color::PASTEBOARD));
    // Ask for the renders this view needs; the last finished ones show meanwhile.
    if let (Some(pyramid), Some(area)) = (preview::pyramid(workspace, window, cx), area(workspace)) {
        let interactive = workspace.ui.slider_pressed || workspace.state.interaction.is_some() || cx.has_active_drag();
        let level = pyramid.level_for(scale(workspace) * window.scale_factor(), area, interactive);
        preview::request(workspace, Slot::Main, view_kind(workspace), level, backdrop, window, cx);
        if workspace.state.compare != Compare::Off && workspace.state.tool != Tool::Crop {
            preview::request(workspace, Slot::Before, View::Before, level, backdrop, window, cx);
        }
    }
    let s = scale(workspace);
    let current_orient = workspace.state.photo().map(preview::OrientKey::of);
    // A render placed by the frame area it shows, so it sits right even while a newer one is coming.
    let place = |workspace: &Workspace, slot: Slot| {
        let shown = workspace.ui.preview.shown(slot).filter(|shown| Some(shown.key.orient) == current_orient)?;
        let at = to_local(workspace, shown.area.min());
        let size = shown.area.size() * s;
        Some((shown.image.clone(), at, size))
    };
    let showing_before = workspace.state.compare == Compare::Before && workspace.state.tool != Tool::Crop;
    let main = place(workspace, if showing_before { Slot::Before } else { Slot::Main });
    let picture = main.map(|(image, at, size)| img(ImageSource::Render(image)).absolute().left(px(at.x)).top(px(at.y)).w(px(size.x)).h(px(size.y)));
    // The split: the photo as it came, cut off at the divider, over the left of the edited one.
    let split = splitting(workspace)
        .then(|| {
            let x = split_x(workspace)?;
            let (image, at, size) = place(workspace, Slot::Before)?;
            let bounds = workspace.ui.viewer.bounds.get();
            let before = div()
                .absolute()
                .left_0()
                .top_0()
                .w(px(x.max(0.0)))
                .h(px(bounds.height))
                .overflow_hidden()
                .child(img(ImageSource::Render(image)).absolute().left(px(at.x)).top(px(at.y)).w(px(size.x)).h(px(size.y)));
            let label = |text: &'static str, left: bool| {
                div()
                    .absolute()
                    .top(px(at.y.max(0.0) + 10.0))
                    .when(left, |label| label.right(px(bounds.width - x + 10.0)))
                    .when(!left, |label| label.left(px(x + 10.0)))
                    .px_2()
                    .py_0p5()
                    .rounded(px(4.0))
                    .bg(gpui_kit::black().opacity(0.55))
                    .text_color(gpui_kit::white())
                    .text_size(px(size::SMALL_TEXT))
                    .child(text)
            };
            let divider = div().absolute().left(px(x - 1.0)).top(px(at.y.max(0.0))).w(px(2.0)).h(px(size.y.min(bounds.height))).bg(gpui_kit::white().opacity(0.9));
            let grip = div().absolute().left(px(x - 9.0)).top(px(at.y.max(0.0) + size.y.min(bounds.height) * 0.5 - 9.0)).size(px(18.0)).rounded_full().bg(gpui_kit::white()).border_1().border_color(gpui_kit::black().opacity(0.4));
            Some(div().absolute().inset_0().child(before).child(divider).child(grip).child(label("Before", true)).child(label("After", false)).into_any_element())
        })
        .flatten();
    let marks = overlays(workspace);
    let tool = workspace.state.tool;
    let pointer = workspace.ui.viewer.pointer;
    let grip = pointer.and_then(|point| match tool {
        Tool::Crop => workspace.state.crop_grip_at(point, GRAB / s.max(1e-6)),
        _ => None,
    });
    let over_split = splitting(workspace)
        && pointer.is_some_and(|point| split_x(workspace).is_some_and(|x| (to_local(workspace, point).x - x).abs() <= GRAB));
    let cursor = match (tool, grip) {
        _ if workspace.ui.viewer.hand.is_some() => CursorStyle::ClosedHand,
        _ if over_split || workspace.ui.viewer.dragging_split => CursorStyle::ResizeLeftRight,
        (Tool::Crop, Some(CropGrip::Corner(0 | 2))) => CursorStyle::ResizeUpLeftDownRight,
        (Tool::Crop, Some(CropGrip::Corner(_))) => CursorStyle::ResizeUpRightDownLeft,
        (Tool::Crop, Some(CropGrip::Edge(0 | 2))) => CursorStyle::ResizeUpDown,
        (Tool::Crop, Some(CropGrip::Edge(_))) => CursorStyle::ResizeLeftRight,
        (Tool::Crop, Some(CropGrip::Inside)) => CursorStyle::OpenHand,
        (Tool::Crop, _) => CursorStyle::Crosshair,
        (Tool::WhiteBalance | Tool::Masks, _) => CursorStyle::Crosshair,
        (Tool::View, _) => CursorStyle::OpenHand,
    };
    let backdrop_color: Hsla = match workspace.ui.settings.backdrop.color() {
        Some(value) => gpui_kit::rgb(value).into(),
        None => hsla(color::PASTEBOARD),
    };
    div()
        .id("photo-view")
        .flex_1()
        .min_h_0()
        .relative()
        .overflow_hidden()
        .bg(backdrop_color)
        .cursor(cursor)
        .child(workspace.ui.viewer.bounds.probe())
        .children(picture)
        .children(split)
        .child(marks)
        .children(super::iris::badge(workspace, cx))
        .on_mouse_down(MouseButton::Left, cx.listener(|this, event: &MouseDownEvent, window, cx| press(this, event, window, cx)))
        .on_mouse_down(MouseButton::Middle, cx.listener(|this, event: &MouseDownEvent, _, cx| {
            this.ui.viewer.hand = Some((Vec2::new(f32::from(event.position.x), f32::from(event.position.y)), this.ui.viewer.pan));
            this.notify_canvas(cx);
        }))
        .on_mouse_move(cx.listener(|this, event: &gpui_kit::MouseMoveEvent, _, cx| {
            let position = Vec2::new(f32::from(event.position.x), f32::from(event.position.y));
            if let Some((start, pan)) = this.ui.viewer.hand
                && event.pressed_button == Some(MouseButton::Middle)
            {
                this.ui.viewer.pan = pan + (position - start);
            }
            this.ui.viewer.pointer = Some(to_frame(this, position.x, position.y));
            this.notify_canvas(cx);
        }))
        .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
            if !hovered {
                this.ui.viewer.pointer = None;
                this.notify_canvas(cx);
            }
        }))
        // Scrolling with a trackpad pans; a mouse wheel, or ⌘ or ⌥ with a trackpad, zooms around the pointer.
        .on_scroll(cx.listener(|this, event: &gpui_kit::ScrollEvent, _, cx| {
            let zooms = matches!(event.delta, gpui_kit::ScrollDelta::Lines(_)) || event.modifiers.platform || event.modifiers.alt;
            let delta = event.delta.pixel_delta(px(16.0));
            let (dx, dy) = (f32::from(delta.x), f32::from(delta.y));
            if zooms {
                let at = to_frame(this, f32::from(event.position.x), f32::from(event.position.y));
                let next = scale(this) * (dy * 0.004).exp();
                zoom_to(this, at, next);
            } else {
                this.ui.viewer.pan += Vec2::new(dx, dy);
            }
            cx.stop_propagation();
            this.notify_canvas(cx);
        }))
        .on_drag(PhotoDrag, |_, _, _, cx| cx.new(|_| EmptyDragView))
        .on_drag_move(cx.listener(|this, event: &DragMoveEvent<PhotoDrag>, _, cx| {
            let position = Vec2::new(f32::from(event.event.position.x), f32::from(event.event.position.y));
            if let Some((start, pan)) = this.ui.viewer.hand {
                this.ui.viewer.pan = pan + (position - start);
                this.notify_canvas(cx);
                return;
            }
            if this.ui.viewer.dragging_split {
                if let Some(area) = area(this) {
                    let left = to_local(this, area.min()).x;
                    let right = to_local(this, area.max()).x;
                    let local = position.x - this.ui.viewer.bounds.get().left;
                    this.state.split_at = ((local - left) / (right - left).max(1.0)).clamp(0.0, 1.0);
                }
                this.notify_canvas(cx);
                return;
            }
            let point = to_frame(this, position.x, position.y);
            this.ui.viewer.pointer = Some(point);
            let tolerance = GRAB / scale(this).max(1e-6);
            this.state.drag(point, keys(event.event.modifiers), tolerance);
            cx.notify();
        }))
        // Photos or projects dropped from the Finder open, each in a tab of its own.
        .drag_over::<gpui_kit::ExternalPaths>(|style, _, _, _| style.bg(hsla(color::HOVER)))
        .on_drop(cx.listener(|this, paths: &gpui_kit::ExternalPaths, window, cx| {
            this.open_paths(paths.paths().to_vec(), window, cx);
        }))
        .test_support()
        .into_any_element()
}

/// Filled shapes (each a closed outline) drawn over the photo, in the panel's points.
fn fill_canvas(shapes: Vec<Vec<Vec2>>, fill: Hsla) -> AnyElement {
    canvas(|_, _, _| (), move |bounds, _, window, _| {
        let at = |v: Vec2| point(bounds.origin.x + px(v.x), bounds.origin.y + px(v.y));
        for shape in &shapes {
            let Some(first) = shape.first() else { continue };
            let mut path = PathBuilder::fill();
            path.move_to(at(*first));
            for corner in &shape[1..] {
                path.line_to(at(*corner));
            }
            path.close();
            if let Ok(path) = path.build() {
                window.paint_path(path, fill);
            }
        }
    })
    .absolute()
    .size_full()
    .into_any_element()
}

/// Line segments drawn over the photo, in the panel's points.
fn lines_canvas(segments: Vec<(Vec2, Vec2)>, width: f32, line: Hsla) -> AnyElement {
    canvas(|_, _, _| (), move |bounds, _, window, _| {
        if segments.is_empty() {
            return;
        }
        let at = |v: Vec2| point(bounds.origin.x + px(v.x), bounds.origin.y + px(v.y));
        let mut path = PathBuilder::stroke(px(width));
        for (a, b) in &segments {
            path.move_to(at(*a));
            path.line_to(at(*b));
        }
        if let Ok(path) = path.build() {
            window.paint_path(path, line);
        }
    })
    .absolute()
    .size_full()
    .into_any_element()
}

/// Closed or open polylines, as segments.
fn polyline(points: &[Vec2], closed: bool) -> Vec<(Vec2, Vec2)> {
    let mut segments: Vec<(Vec2, Vec2)> = points.windows(2).map(|pair| (pair[0], pair[1])).collect();
    if closed && points.len() > 2 {
        segments.push((points[points.len() - 1], points[0]));
    }
    segments
}

/// A round handle at a panel position: filled with the accent when it's the active one.
fn handle(at: Vec2, filled: bool) -> AnyElement {
    div()
        .absolute()
        .left(px(at.x - 5.0))
        .top(px(at.y - 5.0))
        .size(px(10.0))
        .rounded_full()
        .border_1()
        .border_color(gpui_kit::black().opacity(0.6))
        .bg(if filled { hsla(color::ACCENT) } else { gpui_kit::white() })
        .into_any_element()
}

/// A mask's pin: a ring, filled when it's the selected mask.
fn pin(at: Vec2, selected: bool) -> AnyElement {
    div()
        .absolute()
        .left(px(at.x - 7.0))
        .top(px(at.y - 7.0))
        .size(px(14.0))
        .rounded_full()
        .border_2()
        .border_color(gpui_kit::white())
        .bg(if selected { hsla(color::ACCENT) } else { gpui_kit::black().opacity(0.5) })
        .shadow_md()
        .into_any_element()
}

/// Everything the tools draw over the photo.
fn overlays(workspace: &Workspace) -> AnyElement {
    let mut elements: Vec<AnyElement> = Vec::new();
    let (Some(size), Some(edit)) = (workspace.state.image_size(), workspace.state.edit_settings()) else { return div().into_any_element() };
    let local = |point: Vec2| to_local(workspace, point);
    match workspace.state.tool {
        Tool::Crop => {
            let kept = edit.crop.kept(size);
            let corners = kept.corners().map(local);
            let frame = edit.crop.frame_bounds(size);
            let [outer_min, outer_max] = [local(frame.min()), local(frame.max())];
            // The parts of the frame outside the crop are dimmed: four bands around it.
            let (top_left, bottom_right) = (corners[0], corners[2]);
            let band = |a: Vec2, b: Vec2| vec![a, Vec2::new(b.x, a.y), b, Vec2::new(a.x, b.y)];
            let bands = vec![
                band(outer_min, Vec2::new(outer_max.x, top_left.y)),
                band(Vec2::new(outer_min.x, bottom_right.y), outer_max),
                band(Vec2::new(outer_min.x, top_left.y), Vec2::new(top_left.x, bottom_right.y)),
                band(Vec2::new(bottom_right.x, top_left.y), Vec2::new(outer_max.x, bottom_right.y)),
            ];
            elements.push(fill_canvas(bands, gpui_kit::black().opacity(0.55)));
            // The rule of thirds while cropping, as Lightroom's grid overlay.
            if workspace.ui.settings.crop_grid {
                let mut grid = Vec::new();
                for third in [1.0 / 3.0, 2.0 / 3.0] {
                    grid.push((top_left.lerp(Vec2::new(bottom_right.x, top_left.y), third), Vec2::new(top_left.x, bottom_right.y).lerp(bottom_right, third)));
                    grid.push((top_left.lerp(Vec2::new(top_left.x, bottom_right.y), third), Vec2::new(bottom_right.x, top_left.y).lerp(bottom_right, third)));
                }
                elements.push(lines_canvas(grid, 1.0, gpui_kit::white().opacity(0.45)));
            }
            elements.push(lines_canvas(polyline(&corners, true), 1.5, gpui_kit::white()));
            let edges = [corners[0].lerp(corners[1], 0.5), corners[1].lerp(corners[2], 0.5), corners[2].lerp(corners[3], 0.5), corners[3].lerp(corners[0], 0.5)];
            elements.extend(corners.iter().chain(edges.iter()).map(|at| handle(*at, false)));
        }
        Tool::Masks => {
            let image = |fraction: Vec2| workspace.state.image_point(fraction).and_then(|at| workspace.state.image_to_frame(at)).map(local).unwrap_or_default();
            for (index, mask) in edit.masks.iter().enumerate() {
                let selected = workspace.state.selected_mask == Some(index);
                if selected {
                    elements.push(mask_outline(workspace, &mask.shape));
                }
                elements.push(pin(image(mask_pin(&mask.shape)), selected));
            }
        }
        Tool::View | Tool::WhiteBalance => {}
    }
    div().absolute().inset_0().children(elements).into_any_element()
}

/// The selected mask's shape and handles: a linear gradient's three lines (full strength,
/// middle, and where it fades out) across the photo, or a radial gradient's ellipse.
fn mask_outline(workspace: &Workspace, shape: &MaskShape) -> AnyElement {
    let Some((width, height)) = workspace.state.image_size() else { return div().into_any_element() };
    let size = Vec2::new(width as f32, height as f32);
    let pixels = |fraction: Vec2| Vec2::new(fraction.x * size.x, fraction.y * size.y);
    let local = |image_point: Vec2| workspace.state.image_to_frame(image_point).map(|frame| to_local(workspace, frame)).unwrap_or_default();
    let mut elements = Vec::new();
    match *shape {
        MaskShape::Linear { start, end } => {
            let (start, end) = (pixels(start), pixels(end));
            // Each line runs across the photo, square to the gradient's direction.
            let across = (end - start).perpendicular().normalized() * (size.x + size.y);
            let line = |at: Vec2| (local(at - across), local(at + across));
            let lines = vec![line(start), line(end)];
            elements.push(lines_canvas(lines, 1.5, gpui_kit::white().opacity(0.9)));
            elements.push(lines_canvas(vec![line(start.lerp(end, 0.5))], 1.0, gpui_kit::white().opacity(0.5)));
            elements.push(handle(local(start), true));
            elements.push(handle(local(end), false));
        }
        MaskShape::Radial { center, radius, angle, feather } => {
            let (center, radius) = (pixels(center), pixels(radius));
            let ellipse = |scale: f32| -> Vec<Vec2> {
                let (sin, cos) = angle.to_radians().sin_cos();
                (0..72)
                    .map(|step| {
                        let t = step as f32 / 72.0 * std::f32::consts::TAU;
                        let (x, y) = (radius.x * scale * t.cos(), radius.y * scale * t.sin());
                        local(center + Vec2::new(x * cos - y * sin, x * sin + y * cos))
                    })
                    .collect()
            };
            elements.push(lines_canvas(polyline(&ellipse(1.0), true), 1.5, gpui_kit::white().opacity(0.9)));
            // The inner ring: where the fade begins.
            let inner = (1.0 - feather / 100.0).clamp(0.0, 1.0);
            if inner > 0.02 && inner < 0.98 {
                elements.push(lines_canvas(polyline(&ellipse(inner), true), 1.0, gpui_kit::white().opacity(0.4)));
            }
            elements.extend(radial_handles(center, radius, angle).into_iter().map(|at| handle(local(at), false)));
        }
    }
    div().absolute().inset_0().children(elements).into_any_element()
}

/// The strip above the photo: what the tool in use offers.
fn options_bar(workspace: &Workspace, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    use gpui_kit::component::Sizable as _;
    use gpui_kit::component::button::{Button, ButtonVariants as _};
    use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
    workspace.state.photo()?;
    let hint = |text: &'static str| div().text_color(hsla(color::MUTED)).truncate().child(text);
    let bar = div().h(px(36.0)).flex_none().flex().items_center().gap_2().px_2p5().border_b_1().border_color(hsla(color::HAIRLINE)).text_size(px(size::SMALL_TEXT));
    let done = Button::new("tool-done").primary().small().label("Done").on_click(cx.listener(|this, _, _, cx| {
        this.state.set_tool(Tool::View);
        cx.notify();
    }));
    let bar = match workspace.state.tool {
        Tool::View => return None,
        Tool::Crop => {
            let current = workspace.state.crop_aspect;
            let target = cx.weak_entity();
            let aspect = crate::panels::chrome::dropdown("crop-aspect", ASPECTS[current].0).tooltip("Hold the crop to a shape").dropdown_menu(move |menu, _, _| {
                ASPECTS.iter().enumerate().fold(menu, |menu, (index, (name, _))| {
                    let target = target.clone();
                    menu.item(PopupMenuItem::new(*name).checked(index == current).on_click(move |_, _, cx| {
                        target
                            .update(cx, |this, cx| {
                                this.state.set_crop_aspect(index);
                                cx.notify();
                            })
                            .ok();
                    }))
                })
            });
            bar.child(div().text_color(hsla(color::MUTED)).child("Aspect"))
                .child(aspect)
                .child(icon_button("crop-swap", IconName::ArrowUpDown, "Swap portrait and landscape", false, cx.listener(|this, _, _, cx| {
                    this.state.swap_crop_orientation();
                    cx.notify();
                })))
                .child(div().w(px(10.0)))
                .child(div().text_color(hsla(color::MUTED)).child("Angle"))
                .child(value_field(FieldTarget::CropAngle, workspace, cx))
                .child(Button::new("crop-level").outline().small().label("Level").tooltip("Straighten by the photo's own horizon or uprights, when they are clear").on_click(cx.listener(|_, _, window, cx| {
                    window.dispatch_action(Box::new(crate::workspace::LevelPhoto), cx);
                })))
                .child(icon_button("crop-rotate-left", IconName::RotateCcw, crate::keymap::with_key("Rotate Left", "rotate-left"), false, cx.listener(|this, _, _, cx| {
                    this.state.rotate(false);
                    cx.notify();
                })))
                .child(icon_button("crop-rotate-right", IconName::RotateCw, crate::keymap::with_key("Rotate Right", "rotate-right"), false, cx.listener(|this, _, _, cx| {
                    this.state.rotate(true);
                    cx.notify();
                })))
                .child(icon_button("crop-flip", IconName::FlipHorizontal2, "Flip Horizontal", false, cx.listener(|this, _, _, cx| {
                    this.state.flip(true);
                    cx.notify();
                })))
                .child(icon_button("crop-flip-vertical", IconName::FlipVertical2, "Flip Vertical", false, cx.listener(|this, _, _, cx| {
                    this.state.flip(false);
                    cx.notify();
                })))
                .child(div().flex_1().min_w_0().child(hint("Drag corners to crop · drag outside to straighten (⇧ in whole degrees)")))
                .child(Button::new("crop-reset").ghost().small().label("Reset").on_click(cx.listener(|this, _, _, cx| {
                    this.state.reset_crop();
                    cx.notify();
                })))
                .child(done)
        }
        Tool::WhiteBalance => bar.child(hint("Click something that should be neutral gray or white, like a wall, a cloud or a shirt.")).child(div().flex_1()).child(done),
        Tool::Masks => {
            let kinds = [(MaskKind::Linear, "Linear"), (MaskKind::Radial, "Radial")];
            let about = match workspace.state.mask_kind {
                MaskKind::Linear => "Drag across the photo to fade an adjustment in, from full strength where you start to none where you stop.",
                MaskKind::Radial => "Drag out an ellipse to adjust inside it (⇧ for a circle). Invert it in the Masks card to adjust outside.",
            };
            bar.child(segmented("mask-kind", &kinds, workspace.state.mask_kind, |this, kind| this.state.mask_kind = kind, cx)).child(div().flex_1().min_w_0().child(hint(about))).child(done)
        }
    };
    Some(bar.into_any_element())
}

/// The bar under the photo: zoom, the result's size, the comparison and the backdrop.
fn status_bar(workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
    let s = scale(workspace);
    let label = match workspace.ui.viewer.zoom {
        None => format!("Fit ({}%)", (s * 100.0).round()),
        Some(zoom) => format!("{}%", (zoom * 100.0 * 10.0).round() / 10.0),
    };
    let target = cx.weak_entity();
    let zoom_menu = crate::panels::chrome::bar_dropdown("viewer-zoom", label).dropdown_menu(move |menu, _, _| {
        std::iter::once(None).chain([0.25f32, 0.5, 1.0, 2.0, 4.0].into_iter().map(Some)).fold(menu, |menu, zoom| {
            let target = target.clone();
            let name = zoom.map_or_else(|| "Fit on Screen".to_owned(), |zoom| format!("{}%", (zoom * 100.0).round()));
            menu.item(PopupMenuItem::new(name).on_click(move |_, _, cx| {
                target
                    .update(cx, |this, cx| {
                        set_zoom(this, zoom);
                        cx.notify();
                    })
                    .ok();
            }))
        })
    });
    let current = workspace.ui.settings.backdrop;
    let target = cx.weak_entity();
    let backdrop = crate::panels::chrome::bar_dropdown("viewer-backdrop", current.name()).tooltip("What shows around the photo").dropdown_menu(move |menu, _, _| {
        crate::app_settings::Backdrop::ALL.into_iter().fold(menu, |menu, choice| {
            let target = target.clone();
            menu.item(PopupMenuItem::new(choice.name()).checked(choice == current).on_click(move |_, _, cx| {
                target
                    .update(cx, |this, cx| {
                        this.ui.settings.backdrop = choice;
                        crate::app_settings::update(|settings| settings.backdrop = choice);
                        cx.notify();
                    })
                    .ok();
            }))
        })
    });
    let compare = workspace.state.compare;
    let compare_button = |id: &'static str, icon: IconName, tip: String, mode: Compare, cx: &mut Context<Workspace>| {
        icon_button(id, icon, tip, compare == mode, cx.listener(move |this, _, _, cx| {
            this.state.compare = if this.state.compare == mode { Compare::Off } else { mode };
            cx.notify();
        }))
    };
    let size_text = workspace.state.photo().map(|photo| {
        let (width, height) = photo.edit.output_size(photo.source.size());
        format!("{width} × {height} px")
    });
    div()
        .size_full()
        .flex()
        .items_center()
        .gap_3()
        .px_1()
        .text_size(px(size::SMALL_TEXT))
        .text_color(hsla(color::MUTED))
        .child(zoom_menu)
        .children(size_text)
        .child(div().flex_1())
        // The comparison buttons go with Before / After when that is turned off.
        .when(workspace.ui.settings.feature_on(crate::features::Feature::BeforeAfter), |bar| {
            bar.child(compare_button("compare-before", IconName::Eye, crate::keymap::with_key("Show Before", "before"), Compare::Before, cx))
                .child(compare_button("compare-split", IconName::Columns2, crate::keymap::with_key("Before / After Split", "split"), Compare::Split, cx))
        })
        .child(backdrop)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_close_shapes_only_when_asked() {
        let square = [Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0), Vec2::new(1.0, 1.0)];
        assert_eq!(polyline(&square, false).len(), 2);
        assert_eq!(polyline(&square, true).len(), 3);
    }
}

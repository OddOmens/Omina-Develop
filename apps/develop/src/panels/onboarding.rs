//! The welcome setup, shown over the start screen the first time Develop opens (and again from
//! Settings ▸ General), laid out as Studio's. Four short pages:
//!
//! 1. Look: the theme and the accent color.
//! 2. Tools & features: a starting point ("How do you edit?") and every tool and feature to
//!    keep or hide (the same switches as Settings ▸ Tools & Features).
//! 3. Iris: whether to use the assistant, and what she looks for. She is plain rules, not AI,
//!    so this page says so; leaving her off is a first-class choice, not a buried setting.
//! 4. Ready: what was picked, and where to change it later.
//!
//! Every choice applies and is saved as it is made, so the editor behind already looks the way
//! it was set up. Skipping keeps whatever was chosen so far (Develop's defaults if nothing was).

use develop_core::assistant::Check;
use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::{
    AnyElement, Context, ElementId, InteractiveElement, IntoElement, ParentElement, StatefulInteractiveElement, Styled, TestSupportExt as _, div, prelude::FluentBuilder as _, px,
    relative,
};

use crate::app_settings::{self, AppSettings};
use crate::features::{Focus, GROUPS, IRIS_GROUP, tool_features};
use crate::theme::{Accent, ThemeName, color, hsla, size};
use crate::workspace::Workspace;

/// The setup's pages, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Look,
    Features,
    Iris,
    Done,
}

impl Step {
    const ALL: [Step; 4] = [Self::Look, Self::Features, Self::Iris, Self::Done];

    fn index(self) -> usize {
        Self::ALL.iter().position(|step| *step == self).unwrap_or(0)
    }

    fn next(self) -> Option<Self> {
        Self::ALL.get(self.index() + 1).copied()
    }

    fn previous(self) -> Option<Self> {
        self.index().checked_sub(1).map(|index| Self::ALL[index])
    }

    /// Its name in the side panel's list of steps.
    fn name(self) -> &'static str {
        match self {
            Self::Look => "Look",
            Self::Features => "Tools & features",
            Self::Iris => "Iris",
            Self::Done => "Ready",
        }
    }
}

/// The width of the setup's column.
const COLUMN_WIDTH: f32 = 720.0;
/// The width of the side panel listing the steps.
const SIDE_WIDTH: f32 = 260.0;

/// Shows the setup from its first page.
pub fn show(workspace: &mut Workspace, cx: &mut Context<Workspace>) {
    workspace.ui.onboarding = Some(Step::Look);
    // A panel's floating menu would draw over it, so it closes.
    workspace.ui.panel_menu = None;
    cx.notify();
}

/// Closes the setup and remembers it has been through, so it doesn't show on the next launch.
fn finish(workspace: &mut Workspace, cx: &mut Context<Workspace>) {
    change(workspace, cx, |settings| settings.onboarded = true);
    workspace.ui.onboarding = None;
    cx.notify();
}

/// Saves a choice and applies it to the editor behind (its theme, shortcuts, menus and panels).
fn change(workspace: &mut Workspace, cx: &mut Context<Workspace>, edit: impl Fn(&mut AppSettings)) {
    app_settings::update(&edit);
    let mut settings = workspace.ui.settings.clone();
    edit(&mut settings);
    workspace.use_settings(settings, cx);
}

/// Moves to page `step`.
fn go_to(workspace: &mut Workspace, step: Step, cx: &mut Context<Workspace>) {
    workspace.ui.onboarding = Some(step);
    cx.notify();
}

/// The setup over the whole window, while it is showing.
pub fn render(workspace: &Workspace, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    let step = workspace.ui.onboarding?;
    let settings = &workspace.ui.settings;
    let (title, about, body) = match step {
        Step::Look => ("Welcome to Omina Develop", "Let's set it up the way you like. First, pick a theme and an accent color.", look_page(settings, cx).into_any_element()),
        Step::Features => (
            "How do you edit?",
            "Start from a set that fits, then fine-tune. What you turn off is hidden everywhere: tools, panels, cards, menus and shortcuts.",
            features_page(settings, cx).into_any_element(),
        ),
        Step::Iris => (
            "Iris, if you want her",
            "Iris is Omina Develop's assistant. She looks over each photo with plain rules, not AI, and nothing leaves this Mac. Omina Develop works fully without her.",
            iris_page(settings, cx).into_any_element(),
        ),
        Step::Done => ("You're all set", "Everything here can be changed later in Settings.", done_page(settings).into_any_element()),
    };
    let column = div()
        .w(px(COLUMN_WIDTH))
        .max_w_full()
        .h_full()
        .flex()
        .flex_col()
        .child(
            div()
                .flex_none()
                .pt_10()
                .pb_5()
                .flex()
                .flex_col()
                .gap_2()
                .child(step_dots(step))
                .child(div().id("onboarding-title").text_size(px(size::DISPLAY_TEXT)).font_weight(gpui_kit::FontWeight::SEMIBOLD).child(title).test_support())
                .child(div().text_color(hsla(color::MUTED)).child(about)),
        )
        .child(div().id("onboarding-body").flex_1().min_h_0().overflow_y_scroll().pb_4().child(body))
        .child(footer(step, cx));
    let content = div().relative().flex_1().min_w_0().h_full().flex().justify_center().px_8().child(super::start_screen::accent_glow(360.0)).child(column);
    Some(
        div()
            .id("onboarding")
            .absolute()
            .inset_0()
            .flex()
            .bg(hsla(color::BACKGROUND))
            .text_color(hsla(color::TEXT))
            .text_size(px(size::TEXT))
            .occlude()
            .child(side_panel(step, cx))
            .child(content)
            .test_support()
            .into_any_element(),
    )
}

/// The panel down the left: Develop's mark, the steps (done ones ticked, the current one
/// highlighted, earlier ones clickable to go back), and a reassuring line at the bottom.
fn side_panel(step: Step, cx: &mut Context<Workspace>) -> impl IntoElement {
    let steps = Step::ALL.into_iter().map(|each| {
        let (done, current) = (each.index() < step.index(), each == step);
        let badge = div()
            .size(px(24.0))
            .flex_none()
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(size::SMALL_TEXT))
            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
            .map(|badge| match (done, current) {
                (true, _) => badge.bg(hsla(color::ACCENT)).child(Icon::new(IconName::Check).size(px(13.0)).text_color(crate::theme::on_accent())),
                (_, true) => badge.border_2().border_color(hsla(color::ACCENT)).text_color(hsla(color::TEXT)).child((each.index() + 1).to_string()),
                _ => badge.border_1().border_color(hsla(color::MUTED).opacity(0.5)).text_color(hsla(color::MUTED)).child((each.index() + 1).to_string()),
            });
        div()
            .id(("onboarding-step", each.index()))
            .h(px(40.0))
            .px_2()
            .flex()
            .items_center()
            .gap_3()
            .rounded(px(8.0))
            .text_color(hsla(if current || done { color::TEXT } else { color::MUTED }))
            .when(current, |row| row.bg(hsla(color::ACCENT).opacity(0.14)).font_weight(gpui_kit::FontWeight::SEMIBOLD))
            .when(done, |row| row.cursor_pointer().hover(|style| style.bg(hsla(color::HOVER))).on_click(cx.listener(move |this, _, _, cx| go_to(this, each, cx))))
            .child(badge)
            .child(each.name())
            .test_support()
    });
    div()
        .relative()
        .w(px(SIDE_WIDTH))
        .flex_none()
        .h_full()
        .flex()
        .flex_col()
        .px_5()
        .pt_10()
        .pb_6()
        .overflow_hidden()
        .bg(hsla(color::PANEL))
        .border_r_1()
        .border_color(hsla(color::HAIRLINE))
        .child(
            // A deeper wash of the accent at the top of the panel.
            div().absolute().top_0().left_0().right_0().h(px(260.0)).bg(gpui_kit::linear_gradient(
                180.0,
                gpui_kit::linear_color_stop(hsla(color::ACCENT).opacity(0.28), 0.0),
                gpui_kit::linear_color_stop(hsla(color::ACCENT).opacity(0.0), 1.0),
            )),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(super::start_screen::brand_mark(40.0))
                .child(div().flex().flex_col().child(div().text_size(px(size::TITLE_TEXT)).font_weight(gpui_kit::FontWeight::SEMIBOLD).child("Omina Develop")).child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child("Setup"))),
        )
        .child(div().pt_8().flex().flex_col().gap_1().children(steps))
        .child(div().flex_1())
        .child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child("About a minute. Everything here can be changed later in Settings."))
}

/// "Step 2 of 4", with a dot for each page.
fn step_dots(step: Step) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap_1p5()
        .children(Step::ALL.iter().map(|each| {
            let reached = each.index() <= step.index();
            div().h(px(6.0)).w(px(if *each == step { 22.0 } else { 6.0 })).rounded_full().bg(if reached { hsla(color::ACCENT) } else { hsla(color::MUTED).opacity(0.35) })
        }))
        .child(div().pl_1().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(format!("Step {} of {}", step.index() + 1, Step::ALL.len())))
}

/// Skip on the left; Back and Continue (or Start Editing, at the end) on the right.
fn footer(step: Step, cx: &mut Context<Workspace>) -> impl IntoElement {
    let next = match step.next() {
        Some(next) => Button::new("onboarding-next").primary().label("Continue").on_click(cx.listener(move |this, _, _, cx| go_to(this, next, cx))),
        None => Button::new("onboarding-next").primary().label("Start Editing").on_click(cx.listener(|this, _, _, cx| finish(this, cx))),
    };
    div()
        .flex_none()
        .py_4()
        .flex()
        .items_center()
        .justify_between()
        .border_t_1()
        .border_color(hsla(color::HAIRLINE))
        .child(div().when(step != Step::Done, |left| {
            left.child(Button::new("onboarding-skip").ghost().label("Skip Setup").tooltip("Keep what's picked so far. Settings can change it any time.").on_click(cx.listener(|this, _, _, cx| finish(this, cx))))
        }))
        .child(
            div()
                .flex()
                .gap_2()
                .children(step.previous().map(|previous| Button::new("onboarding-back").outline().label("Back").on_click(cx.listener(move |this, _, _, cx| go_to(this, previous, cx)))))
                .child(next),
        )
}

/// A small heading over a group of choices.
fn heading(text: &'static str) -> impl IntoElement {
    div().pt_4().pb_2().text_size(px(size::SMALL_TEXT)).font_weight(gpui_kit::FontWeight::SEMIBOLD).text_color(hsla(color::MUTED)).child(text.to_uppercase())
}

/// A choice drawn as a card: a mark (an icon, or Iris's face), a name and a line about it,
/// outlined in the accent when picked.
fn choice_card(id: impl Into<ElementId>, mark: Option<AnyElement>, name: &'static str, about: &'static str, picked: bool) -> gpui_kit::Stateful<gpui_kit::Div> {
    div()
        .id(id)
        .flex_1()
        .min_w_0()
        .p_3()
        .flex()
        .flex_col()
        .gap_1()
        .rounded(px(10.0))
        .border_2()
        .border_color(hsla(if picked { color::ACCENT } else { color::HAIRLINE }))
        .bg(hsla(color::PANEL))
        .cursor_pointer()
        .hover(|style| style.bg(hsla(color::HOVER)))
        .children(mark)
        .child(div().font_weight(gpui_kit::FontWeight::SEMIBOLD).child(name))
        .child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(about))
}

/// An icon for a choice card, in the accent when the card is picked.
fn card_icon(icon: IconName, picked: bool) -> AnyElement {
    Icon::new(icon).size(px(18.0)).text_color(hsla(if picked { color::ACCENT } else { color::MUTED })).into_any_element()
}

/// Page 1: the theme and the accent color, applied as they are picked.
fn look_page(settings: &AppSettings, cx: &mut Context<Workspace>) -> impl IntoElement {
    let themes = ThemeName::ALL.into_iter().map(|theme| {
        super::settings::theme_choice(theme, settings.accent, settings.theme == theme).on_click(cx.listener(move |this, _, _, cx| change(this, cx, |settings| settings.theme = theme))).test_support()
    });
    let accents = Accent::ALL.into_iter().map(|accent| {
        super::settings::accent_choice(accent, settings.accent == accent).on_click(cx.listener(move |this, _, _, cx| change(this, cx, |settings| settings.accent = accent))).test_support()
    });
    div()
        .child(heading("Theme"))
        .child(div().flex().gap_3().children(themes))
        .child(heading("Accent color"))
        .child(div().flex().items_center().gap_3().child(div().flex().gap_1().children(accents)).child(div().text_color(hsla(color::MUTED)).child(settings.accent.name())))
        .child(heading("Preview"))
        .child(editor_preview())
}

/// A small picture of the editor in the theme and accent picked so far (it is drawn in the live
/// palette, so it changes as they are picked): the top bar and rail, the Presets panel, the
/// photo in its tab with a crop box, the Develop panel's histogram and sliders, and the status
/// bar under the photo.
fn editor_preview() -> impl IntoElement {
    let bar = |width: f32, tone: u32, alpha: f32| div().h(px(5.0)).w(px(width)).rounded(px(2.0)).bg(hsla(tone).opacity(alpha));
    // A slider: its name, a track with the knob at `at` (0–1), and its value.
    let slider = |at: f32| {
        div()
            .flex()
            .items_center()
            .gap_1()
            .child(bar(22.0, color::MUTED, 0.4))
            .child(div().relative().flex_1().h(px(3.0)).rounded(px(2.0)).bg(hsla(color::INSET)).child(div().absolute().top(px(-2.5)).left(relative(at)).size(px(8.0)).rounded_full().bg(hsla(color::TEXT).opacity(0.85))))
            .child(bar(12.0, color::VALUE, 0.8))
    };
    // The histogram: three soft hills of red, green and blue.
    let hill = |left: f32, width: f32, tint: u32| div().absolute().bottom_0().left(relative(left)).w(relative(width)).h(relative(0.8)).rounded_t(px(14.0)).bg(gpui_kit::rgb(tint)).opacity(0.35);
    // A preset in the Presets panel: a small picture and a name.
    let preset = |alpha: f32| div().flex().items_center().gap_1().child(div().size(px(14.0)).rounded(px(2.0)).bg(hsla(color::ACCENT).opacity(alpha))).child(bar(40.0, color::MUTED, 0.35));
    div()
        .id("onboarding-preview")
        .h(px(232.0))
        .rounded(px(12.0))
        .overflow_hidden()
        .border_1()
        .border_color(hsla(color::HAIRLINE))
        .bg(hsla(color::BACKGROUND))
        .flex()
        .flex_col()
        // The top bar: the mark, the tools (the one in use in the accent) and Export.
        .child(
            div()
                .h(px(24.0))
                .flex_none()
                .px_2()
                .flex()
                .items_center()
                .gap_1p5()
                .child(div().size(px(12.0)).rounded(px(3.0)).bg(hsla(color::ACCENT)))
                .child(div().size(px(10.0)).rounded(px(2.0)).bg(hsla(color::ACCENT).opacity(0.35)))
                .children((0..6).map(|_| div().size(px(10.0)).rounded(px(2.0)).bg(hsla(color::MUTED).opacity(0.35))))
                .child(div().flex_1())
                .child(bar(50.0, color::MUTED, 0.4))
                .child(div().h(px(12.0)).w(px(34.0)).rounded(px(3.0)).bg(hsla(color::ACCENT))),
        )
        .child(
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .gap_1()
                .px_1()
                .pb_1()
                // The rail.
                .child(div().w(px(18.0)).flex_none().flex().flex_col().items_center().gap_1p5().pt_1().children((0..4).map(|index| div().size(px(10.0)).rounded(px(3.0)).bg(if index == 0 { hsla(color::ACCENT) } else { hsla(color::MUTED).opacity(0.3) }))))
                // The Presets panel.
                .child(div().w(px(84.0)).flex_none().rounded(px(4.0)).bg(hsla(color::PANEL)).p_1p5().flex().flex_col().gap_1p5().child(bar(40.0, color::TEXT, 0.7)).children([0.8, 0.5, 0.35, 0.6, 0.25].map(preset)))
                // The photo in its tab, with a crop box over it, and the status bar under it.
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .rounded(px(4.0))
                        .bg(hsla(color::PANEL))
                        .flex()
                        .flex_col()
                        .child(div().h(px(14.0)).flex_none().px_1p5().flex().items_center().child(div().h(px(9.0)).px_1().rounded(px(2.0)).bg(hsla(color::HOVER)).flex().items_center().child(bar(32.0, color::TEXT, 0.7))))
                        .child(
                            div().flex_1().min_h_0().bg(hsla(color::PASTEBOARD)).flex().items_center().justify_center().child(
                                div()
                                    .relative()
                                    .w(px(150.0))
                                    .h(px(100.0))
                                    .rounded(px(2.0))
                                    // A landscape: sky in the accent fading to dark ground.
                                    .bg(gpui_kit::linear_gradient(180.0, gpui_kit::linear_color_stop(hsla(color::ACCENT).opacity(0.6), 0.0), gpui_kit::linear_color_stop(hsla(color::INSET), 1.0)))
                                    .child(div().absolute().left_0().right_0().top(relative(0.62)).h(px(1.0)).bg(gpui_kit::white().opacity(0.35)))
                                    .child(div().absolute().left(px(18.0)).top(px(12.0)).w(px(110.0)).h(px(74.0)).border_1().border_color(gpui_kit::white().opacity(0.9))),
                            ),
                        )
                        .child(div().h(px(12.0)).flex_none().px_1p5().flex().items_center().justify_between().child(bar(26.0, color::MUTED, 0.4)).child(bar(18.0, color::MUTED, 0.4))),
                )
                // The Develop panel: the histogram, then sliders.
                .child(
                    div()
                        .w(px(124.0))
                        .flex_none()
                        .rounded(px(4.0))
                        .bg(hsla(color::PANEL))
                        .p_1p5()
                        .flex()
                        .flex_col()
                        .gap_1p5()
                        .child(div().relative().h(px(34.0)).flex_none().rounded(px(3.0)).overflow_hidden().bg(hsla(color::INSET)).child(hill(0.05, 0.5, 0x3a7bff)).child(hill(0.25, 0.5, 0x34c759)).child(hill(0.45, 0.5, 0xff3b30)))
                        .child(bar(34.0, color::TEXT, 0.7))
                        .children([0.5, 0.62, 0.35, 0.7, 0.45].map(slider)),
                ),
        )
        .test_support()
}

/// Page 2: a starting point, then every tool and feature as a checkbox, by group.
fn features_page(settings: &AppSettings, cx: &mut Context<Workspace>) -> impl IntoElement {
    let matching = Focus::matching(settings);
    let starting_points = Focus::ALL.into_iter().enumerate().map(|(index, focus)| {
        choice_card(("onboarding-focus", index), None, focus.name(), focus.about(), matching == Some(focus)).on_click(cx.listener(move |this, _, _, cx| change(this, cx, |settings| focus.apply(settings)))).test_support()
    });
    let mut page = div().child(heading("Start from")).child(div().flex().gap_2().children(starting_points));
    for (group, features) in GROUPS.iter().filter(|(group, _)| *group != IRIS_GROUP) {
        let boxes = features.iter().map(|info| {
            let feature = info.feature;
            div().w(px((COLUMN_WIDTH - 26.0) / 3.0)).py_1().child(
                Checkbox::new(ElementId::Name(format!("onboarding-feature-{}", feature.id()).into()))
                    .label(info.name)
                    .checked(settings.feature_on(feature))
                    .tooltip(info.about)
                    .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                        let checked = *checked;
                        change(this, cx, |settings| settings.set_feature(feature, checked));
                    })),
            )
        });
        page = page.child(heading(group)).child(div().px_3().py_2().rounded(px(10.0)).bg(hsla(color::PANEL)).border_1().border_color(hsla(color::HAIRLINE)).flex().flex_wrap().children(boxes));
    }
    page.child(div().pt_4().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child("Open, Save, Export, the Hand, the Basic card, rotating and History are always there."))
}

/// Page 3: Iris on or off, and what she looks for.
fn iris_page(settings: &AppSettings, cx: &mut Context<Workspace>) -> impl IntoElement {
    let using = settings.iris_on;
    let face = div().h(px(18.0)).flex().items_center().child(super::iris::face("onboarding", 18.0, super::iris::Look::default())).into_any_element();
    let cards = div()
        .flex()
        .gap_3()
        .child(
            choice_card("onboarding-iris-on", Some(face), "Use Iris", "Suggestions for each photo with a fix in a click, and a chat that makes edits you ask for, works through a folder of photos, runs recipes you teach her and answers how-to questions.", using)
                .on_click(cx.listener(|this, _, _, cx| change(this, cx, |settings| settings.iris_on = true)))
                .test_support(),
        )
        .child(
            choice_card("onboarding-iris-off", Some(card_icon(IconName::EyeOff, !using)), "No Iris", "Hide her everywhere: no badge, no suggestions, no chat. Turn her on later in Settings ▸ Iris.", !using)
                .on_click(cx.listener(|this, _, _, cx| change(this, cx, |settings| settings.iris_on = false)))
                .test_support(),
        );
    let checks = Check::ALL.into_iter().enumerate().map(|(index, check)| {
        div()
            .py_1p5()
            .flex()
            .flex_col()
            .gap_0p5()
            .child(Checkbox::new(("onboarding-check", index)).label(check.name()).checked(settings.assistant.runs(check)).on_click(cx.listener(move |this, checked: &bool, _, cx| {
                let checked = *checked;
                change(this, cx, |settings| settings.assistant.set(check, checked));
            })))
            .child(div().pl_6().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(check.about()))
    });
    let badge = Checkbox::new("onboarding-iris-badge").label("Her badge on the photo, with a count of what she noticed").checked(settings.assistant_badge).on_click(cx.listener(|this, checked: &bool, _, cx| {
        let checked = *checked;
        change(this, cx, |settings| settings.assistant_badge = checked);
    }));
    div().child(cards).when(using, |page| {
        page.child(heading("What she looks for"))
            .children(checks)
            .child(heading("On the photo"))
            .child(badge)
            .child(
                div()
                    .mt_3()
                    .p_3()
                    .rounded(px(8.0))
                    .bg(hsla(color::PANEL))
                    .text_size(px(size::SMALL_TEXT))
                    .text_color(hsla(color::MUTED))
                    .child("Iris measures each photo on this Mac with plain rules: how bright it is, how much is clipped, how strong and how even its colors are. Each suggestion says what she measured. No AI model is used or downloaded, and nothing is uploaded."),
            )
    })
}

/// Page 4: a summary of what was picked, and where each choice lives in Settings.
fn done_page(settings: &AppSettings) -> impl IntoElement {
    let total = tool_features().count();
    let on = tool_features().filter(|feature| settings.feature_on(*feature)).count();
    let iris = if settings.iris_on {
        let checks = Check::ALL.into_iter().filter(|check| settings.assistant.runs(*check)).count();
        format!("On, {checks} of {} checks", Check::ALL.len())
    } else {
        "Off. Nothing is measured.".to_owned()
    };
    let row = |name: &'static str, value: String, where_: &'static str| {
        div()
            .py_2p5()
            .flex()
            .items_center()
            .gap_4()
            .border_b_1()
            .border_color(hsla(color::HAIRLINE))
            .child(div().w(px(160.0)).flex_none().text_color(hsla(color::MUTED)).child(name))
            .child(div().flex_1().min_w_0().child(value))
            .child(div().flex_none().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(where_))
    };
    let tick = div()
        .size(px(56.0))
        .mb_4()
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(hsla(color::ACCENT).opacity(0.18))
        .border_1()
        .border_color(hsla(color::ACCENT).opacity(0.5))
        .child(Icon::new(IconName::Check).size(px(26.0)).text_color(hsla(color::ACCENT)));
    div()
        .id("onboarding-summary")
        .pt_2()
        .child(tick)
        .child(row("Look", format!("{}, {}", settings.theme.name(), settings.accent.name()), "Settings ▸ Appearance"))
        .child(row("Tools & features", format!("{on} of {total} on{}", Focus::matching(settings).map(|focus| format!(" ({})", focus.name())).unwrap_or_default()), "Settings ▸ Tools & Features"))
        .child(row("Iris", iris, "Settings ▸ Iris"))
        .test_support()
}

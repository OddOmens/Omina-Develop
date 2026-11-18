//! The Settings dialog (⌘,), laid out as Studio's: sections in a menu on the left, each a page
//! of cards. General (start, the welcome setup and undo), Appearance (theme and accent), Viewer
//! (backdrop and crop grid), Tools & Features, Presets, Iris, Recipes, Keyboard Shortcuts and
//! About.
//!
//! Keyboard Shortcuts and Recipes take a key press: Change (or Record Key) holds the keyboard
//! until the next key, which becomes the shortcut (Esc cancels).

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::switch::Switch;
use gpui_kit::assets::IconName;
use gpui_kit::component::{Disableable as _, Icon, Sizable as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputState, Textarea, TextareaState};
use gpui_kit::{AnyElement, App, AppContext as _, Context, Entity, InteractiveElement, IntoElement, MouseButton, ParentElement, SharedString, StatefulInteractiveElement, Styled, TestSupportExt as _, WeakEntity, Window, div, prelude::FluentBuilder as _, px, rgb};

use crate::app_settings::{self, AppSettings, Backdrop, StartWith};
use crate::theme::{Accent, ThemeName, color, hsla, size};
use crate::workspace::Workspace;

/// How often unsaved work can be copied aside, in seconds (0 for never).
const RECOVERY_CHOICES: &[u32] = &[15, 30, 60, 120, 300, 0];

/// A recovery-copy interval as the menu shows it: "Every 30 seconds", "Never".
fn recovery_label(seconds: u32) -> String {
    match seconds {
        0 => "Never".to_owned(),
        s if s < 60 => format!("Every {s} seconds"),
        60 => "Every minute".to_owned(),
        s => format!("Every {} minutes", s / 60),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Section {
    General,
    Appearance,
    Viewer,
    Features,
    Presets,
    Iris,
    Recipes,
    Shortcuts,
    About,
}

impl Section {
    const ALL: [Section; 9] = [Self::General, Self::Appearance, Self::Viewer, Self::Features, Self::Presets, Self::Iris, Self::Recipes, Self::Shortcuts, Self::About];

    /// Whether the section is listed: Presets goes when its feature is turned off, and Recipes
    /// with Iris.
    fn shown(self, settings: &AppSettings) -> bool {
        match self {
            Self::Presets => settings.feature_on(crate::features::Feature::Presets),
            Self::Recipes => settings.iris_on,
            _ => true,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Appearance => "Appearance",
            Self::Viewer => "Viewer",
            Self::Features => "Tools & Features",
            Self::Presets => "Presets",
            Self::Iris => "Iris",
            Self::Recipes => "Recipes",
            Self::Shortcuts => "Keyboard Shortcuts",
            Self::About => "About",
        }
    }

    fn about(self) -> &'static str {
        match self {
            Self::General => "What Omina Develop does when it starts, how much it can undo, and keeping unsaved work safe.",
            Self::Appearance => "How Omina Develop looks. Omina Motion and Omina Edit use the same themes.",
            Self::Viewer => "How the photo is shown while you work on it.",
            Self::Features => "Turn off what you don't use. It is hidden everywhere: tools, panels, cards, menus and shortcuts.",
            Self::Presets => "The looks you have saved with New Preset.",
            Self::Iris => "What Iris, the assistant, looks for in your photos. Every rule can be switched off.",
            Self::Recipes => "Things you've taught Iris to do by name, and the ones she comes with: their steps, keys, buttons and the moments they run by themselves.",
            Self::Shortcuts => "The keys for tools and commands, starting from Lightroom's. Change any of them; a key another command had moves to the new one.",
            Self::About => "Version and license.",
        }
    }

    fn icon(self) -> IconName {
        match self {
            Self::General => IconName::Settings,
            Self::Appearance => IconName::Palette,
            Self::Viewer => IconName::Image,
            Self::Features => IconName::SlidersHorizontal,
            Self::Presets => IconName::Sparkles,
            Self::Iris => IconName::Eye,
            Self::Recipes => IconName::WandSparkles,
            Self::Shortcuts => IconName::Keyboard,
            Self::About => IconName::Info,
        }
    }
}

/// What is waiting for a key press.
#[derive(Clone, Debug, PartialEq)]
enum Recording {
    /// A command's new shortcut.
    Command(&'static str),
    /// A recipe's key, by the recipe's name.
    Recipe(String),
}

/// A taught recipe being edited: its name and steps (one per line).
struct Editor {
    name: String,
    name_input: Entity<InputState>,
    steps_input: Entity<TextareaState>,
}

struct Shown {
    section: Section,
    settings: AppSettings,
    recording: Option<Recording>,
    /// What the last change came to, shown under the page.
    note: Option<String>,
    editing: Option<Editor>,
    /// The recipe deleted last and its place, for Undo.
    deleted: Option<(::iris::recipes::Recipe, usize)>,
}

type SharedShown = Rc<RefCell<Shown>>;

/// Saves a settings change, shows it, and hands it to the workspace, which applies it (the
/// theme, shortcuts and menus, undo's length).
fn change(shown: &SharedShown, workspace: &WeakEntity<Workspace>, window: &mut Window, cx: &mut App, edit: impl Fn(&mut AppSettings)) {
    app_settings::update(&edit);
    edit(&mut shown.borrow_mut().settings);
    let settings = shown.borrow().settings.clone();
    workspace.update(cx, |this, cx| this.use_settings(settings, cx)).ok();
    window.refresh();
}

fn page(section: Section) -> gpui_kit::Div {
    div()
        .pb_3()
        .flex()
        .flex_col()
        .gap_1()
        .child(div().text_size(px(size::PAGE_TITLE_TEXT)).text_color(hsla(color::TEXT)).font_weight(gpui_kit::FontWeight::SEMIBOLD).child(section.name()))
        .child(div().text_size(px(size::TEXT)).text_color(hsla(color::MUTED)).child(section.about()))
}

fn group_title(text: &'static str) -> gpui_kit::Div {
    div().pt_3().pb_1p5().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).font_weight(gpui_kit::FontWeight::SEMIBOLD).child(text.to_uppercase())
}

fn card(rows: Vec<AnyElement>) -> gpui_kit::Div {
    let count = rows.len();
    div()
        .rounded(px(10.0))
        .border_1()
        .border_color(hsla(color::HAIRLINE))
        .bg(hsla(color::RAISED))
        .children(rows.into_iter().enumerate().map(move |(index, row)| div().when(index + 1 < count, |row| row.border_b_1().border_color(hsla(color::HAIRLINE))).child(row)))
}

fn setting(name: impl Into<SharedString>, about: impl Into<SharedString>, control: impl IntoElement) -> AnyElement {
    div()
        .min_h(px(54.0))
        .px_3p5()
        .py_2()
        .flex()
        .items_center()
        .gap_4()
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap_0p5()
                .child(div().text_size(px(size::LARGE_TEXT)).text_color(hsla(color::TEXT)).child(name.into()))
                .child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(about.into())),
        )
        .child(div().flex_none().flex().items_center().gap_1p5().child(control))
        .into_any_element()
}

fn switch(id: &'static str, on: bool, shown: &SharedShown, workspace: &WeakEntity<Workspace>, set: fn(&mut AppSettings, bool)) -> impl IntoElement {
    let (shown, workspace) = (shown.clone(), workspace.clone());
    Switch::new(id).checked(on).on_click(move |checked: &bool, window, cx| {
        let checked = *checked;
        change(&shown, &workspace, window, cx, |settings| set(settings, checked));
    })
}

fn menu<T: Clone + PartialEq + 'static>(id: &'static str, choices: Vec<(String, T)>, current: T, shown: &SharedShown, workspace: &WeakEntity<Workspace>, set: fn(&mut AppSettings, T)) -> impl IntoElement {
    let label = choices.iter().find(|(_, value)| *value == current).map_or_else(|| "Custom".to_owned(), |(label, _)| label.clone());
    let (shown, workspace) = (shown.clone(), workspace.clone());
    crate::panels::chrome::dropdown(id, label).dropdown_menu(move |menu, _, _| {
        choices.iter().fold(menu, |menu, (label, value)| {
            let (shown, workspace, value) = (shown.clone(), workspace.clone(), value.clone());
            let checked = value == current;
            menu.item(PopupMenuItem::new(label.clone()).checked(checked).on_click(move |_, window, cx| {
                let value = value.clone();
                change(&shown, &workspace, window, cx, |settings| set(settings, value.clone()));
            }))
        })
    })
}

fn general(shown: &SharedShown, workspace: &WeakEntity<Workspace>) -> gpui_kit::Div {
    let welcome = {
        let workspace = workspace.clone();
        Button::new("general-welcome").outline().small().label("Show Again").on_click(move |_, window, cx| {
            window.close_dialog(cx);
            workspace.update(cx, super::onboarding::show).ok();
        })
    };
    let now = shown.borrow().settings.clone();
    let starts = vec![("Start Screen".to_owned(), StartWith::StartScreen), ("Last Project".to_owned(), StartWith::LastProject)];
    let states: Vec<(String, usize)> = [20usize, 50, 100, 200, 500].into_iter().map(|count| (format!("{count} steps"), count)).collect();
    let recovery = RECOVERY_CHOICES.iter().map(|&seconds| (recovery_label(seconds), seconds)).collect();
    div()
        .child(page(Section::General))
        .child(group_title("Starting"))
        .child(card(vec![
            setting("On launch, show", "The start screen, or the project you had open last.", menu("settings-start", starts, now.start_with, shown, workspace, |settings, value| settings.start_with = value)),
            setting("Welcome setup", "Pick a theme, the tools and features you want, and whether to use Iris, step by step.", welcome),
        ]))
        .child(group_title("Undo"))
        .child(card(vec![setting(
            "History states",
            "How many steps Undo and the History panel keep. Each step is small: only the settings are kept, never the pixels.",
            menu("settings-history", states, now.history_states, shown, workspace, |settings, value| settings.history_states = value),
        )]))
        .child(group_title("Recovery"))
        .child(card(vec![setting(
            "Recovery copy",
            "How often unsaved photos are kept aside, so a crash loses little. Offered back on the next start.",
            menu("general-recovery", recovery, now.recovery_seconds, shown, workspace, |settings, seconds| settings.recovery_seconds = seconds),
        )]))
}

/// A theme drawn as a small picture of a window in its colors, with its name under it,
/// outlined in the accent when picked. Used here and in the welcome setup.
pub(super) fn theme_choice(theme: ThemeName, accent: Accent, selected: bool) -> gpui_kit::Stateful<gpui_kit::Div> {
    let (background, panel, text) = theme.preview();
    let accent = accent.on_theme(theme);
    div()
        .id(("theme", theme as usize))
        .flex_1()
        .flex()
        .flex_col()
        .gap_2()
        .cursor_pointer()
        .child(
            div()
                .h(px(84.0))
                .rounded(px(10.0))
                .border_2()
                .border_color(hsla(if selected { color::ACCENT } else { color::HAIRLINE }))
                .bg(rgb(background))
                .p_2()
                .flex()
                .gap_1p5()
                .child(div().w(px(18.0)).rounded(px(4.0)).bg(rgb(panel)))
                .child(
                    div()
                        .flex_1()
                        .rounded(px(4.0))
                        .bg(rgb(panel))
                        .p_1p5()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(div().h(px(5.0)).w(px(46.0)).rounded(px(2.0)).bg(rgb(text)))
                        .child(div().h(px(4.0)).w(px(30.0)).rounded(px(2.0)).bg(rgb(text)).opacity(0.5))
                        .child(div().mt_auto().h(px(8.0)).w(px(26.0)).rounded(px(4.0)).bg(rgb(accent))),
                ),
        )
        .child(div().text_size(px(size::TEXT)).text_color(hsla(if selected { color::TEXT } else { color::MUTED })).child(theme.name()))
}

/// An accent color as a round swatch, ringed when picked. Used here and in the welcome setup.
pub(super) fn accent_choice(accent: Accent, selected: bool) -> gpui_kit::Stateful<gpui_kit::Div> {
    div()
        .id(("accent", accent as usize))
        .size(px(28.0))
        .rounded_full()
        .border_2()
        .border_color(hsla(if selected { color::TEXT } else { color::RAISED }))
        .p(px(3.0))
        .cursor_pointer()
        .child(div().size_full().rounded_full().bg(rgb(accent.value())))
}

fn appearance(shown: &SharedShown, workspace: &WeakEntity<Workspace>) -> gpui_kit::Div {
    let now = shown.borrow().settings.clone();
    let themes = ThemeName::ALL.into_iter().map(|theme| {
        let (shown, workspace) = (shown.clone(), workspace.clone());
        theme_choice(theme, now.accent, now.theme == theme)
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(move |_, window, cx| change(&shown, &workspace, window, cx, |settings| settings.theme = theme))
            .test_support()
    });
    let accents = Accent::ALL.into_iter().map(|accent| {
        let (shown, workspace) = (shown.clone(), workspace.clone());
        accent_choice(accent, now.accent == accent)
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(move |_, window, cx| change(&shown, &workspace, window, cx, |settings| settings.accent = accent))
            .test_support()
    });
    div()
        .child(page(Section::Appearance))
        .child(group_title("Theme"))
        .child(div().flex().gap_3().children(themes))
        .child(group_title("Accent color"))
        .child(card(vec![setting(now.accent.name(), "Used for selections, active tools and highlights.", div().flex().gap_1().children(accents))]))
}

fn viewer_page(shown: &SharedShown, workspace: &WeakEntity<Workspace>) -> gpui_kit::Div {
    let now = shown.borrow().settings.clone();
    let choices: Vec<(String, Backdrop)> = Backdrop::ALL.into_iter().map(|choice| (choice.name().to_owned(), choice)).collect();
    div()
        .child(page(Section::Viewer))
        .child(card(vec![
            setting("Backdrop", "What shows around the photo. Middle gray judges tones most fairly.", menu("settings-backdrop", choices, now.backdrop, shown, workspace, |settings, value| settings.backdrop = value)),
            setting("Crop grid", "The rule of thirds over the crop while cropping.", switch("settings-crop-grid", now.crop_grid, shown, workspace, |settings, on| settings.crop_grid = on)),
        ]))
}

/// Tools & Features: a switch for each, by group, and a button to turn them all back on. Iris
/// has her own page, so she isn't listed here.
fn features_page(shown: &SharedShown, workspace: &WeakEntity<Workspace>) -> gpui_kit::Div {
    use crate::features::{Feature, GROUPS, IRIS_GROUP, tool_features};
    let now = shown.borrow().settings.clone();
    let any_off = tool_features().any(|feature| !now.feature_on(feature));
    let switch_for = |feature: Feature| {
        let (shown, workspace) = (shown.clone(), workspace.clone());
        Switch::new(gpui_kit::ElementId::Name(format!("feature-{}", feature.id()).into())).checked(now.feature_on(feature)).on_click(move |checked: &bool, window, cx| {
            let checked = *checked;
            change(&shown, &workspace, window, cx, |settings| settings.set_feature(feature, checked));
        })
    };
    let mut page_div = div().child(page(Section::Features));
    for (group, features) in GROUPS.iter().filter(|(group, _)| *group != IRIS_GROUP) {
        page_div = page_div.child(group_title(group)).child(card(features.iter().map(|info| setting(info.name, info.about, switch_for(info.feature))).collect()));
    }
    let (reset_shown, reset_workspace) = (shown.clone(), workspace.clone());
    page_div.child(
        div()
            .pt_3()
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            // The note wraps, so the button always stays in view beside it.
            .child(div().flex_1().min_w_0().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child("Open, Save, Export, the Hand, the Basic card, rotating and History are always on. Iris has her own page. Turning something off keeps what it already did to your photos."))
            .child(Button::new("features-all-on").flex_none().outline().small().label("Turn All On").disabled(!any_off).on_click(move |_, window, cx| {
                change(&reset_shown, &reset_workspace, window, cx, |settings| {
                    for feature in tool_features() {
                        settings.set_feature(feature, true);
                    }
                });
            })),
    )
}

fn presets_page(shown: &SharedShown, workspace: &WeakEntity<Workspace>) -> gpui_kit::Div {
    let now = shown.borrow().settings.clone();
    let clear = {
        let (shown, workspace) = (shown.clone(), workspace.clone());
        Button::new("settings-clear-presets").outline().small().label("Delete All").on_click(move |_, window, cx| {
            change(&shown, &workspace, window, cx, |settings| settings.user_presets.clear());
        })
    };
    div().child(page(Section::Presets)).child(card(vec![setting(
        format!("{} presets of your own", now.user_presets.len()),
        "Delete every preset you have made. The built-in ones always stay. Right-click a preset in the Presets panel to delete just that one.",
        clear,
    )]))
}

fn iris_page(shown: &SharedShown, workspace: &WeakEntity<Workspace>) -> gpui_kit::Div {
    use develop_core::assistant::Check;
    let now = shown.borrow().settings.clone();
    // Her name and blinking are shared with Studio and Canvas, so they're read from there.
    let shared = crate::panels::iris::load_shared(&now);
    let rules: Vec<AnyElement> = Check::ALL
        .into_iter()
        .enumerate()
        .map(|(index, check)| {
            let (shown, workspace) = (shown.clone(), workspace.clone());
            let control = Switch::new(("settings-iris-rule", index)).checked(now.assistant.runs(check)).disabled(!now.iris_on).on_click(move |on: &bool, window, cx| {
                let on = *on;
                change(&shown, &workspace, window, cx, |settings| settings.assistant.set(check, on));
            });
            setting(check.name(), check.about(), control)
        })
        .collect();
    let blinks = {
        let workspace = workspace.clone();
        Switch::new("settings-iris-blinks").checked(shared.blinks).disabled(!now.iris_on).on_click(move |on: &bool, window, cx| {
            let on = *on;
            workspace.update(cx, |this, cx| {
                crate::panels::iris::change_shared(this, |shared| shared.blinks = on);
                cx.notify();
            }).ok();
            window.refresh();
        })
    };
    let name = shared.shown_name();
    div()
        .child(page(Section::Iris))
        .child(card(vec![
            setting("Use Iris", "Iris looks over each photo for problems, fixes them in a click, and answers questions about Omina Develop, Omina Motion and Omina Edit. Rules and hand-written help, not AI. Turned off, she's hidden everywhere and stops measuring.", switch("settings-iris-on", now.iris_on, shown, workspace, |settings, on| settings.iris_on = on)),
            setting(format!("Called {name}"), "Her name is the same in every app. Type /name and a new name in her chat to change it everywhere.", div()),
            setting("Blinking", "Her face blinks and glances about now and then. Shared with Omina Motion and Omina Edit.", blinks),
            setting("Badge on the photo", "Her face in a corner of the photo, with a count of what she noticed. A click opens her suggestions.", {
                let (shown, workspace) = (shown.clone(), workspace.clone());
                Switch::new("settings-iris-badge").checked(now.assistant_badge).disabled(!now.iris_on).on_click(move |on: &bool, window, cx| {
                    let on = *on;
                    change(&shown, &workspace, window, cx, |settings| settings.assistant_badge = on);
                })
            }),
        ]))
        .child(group_title("What Iris looks for"))
        .child(card(rules))
}

/// A key as a small chip, or "None".
fn key_chip(key: Option<String>) -> AnyElement {
    match key {
        Some(key) => div().px_2().py_0p5().rounded(px(5.0)).border_1().border_color(hsla(color::HAIRLINE)).bg(hsla(color::INSET)).font_family(crate::theme::MONO_FONT).text_color(hsla(color::VALUE)).child(key).into_any_element(),
        None => div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child("None").into_any_element(),
    }
}

/// What the last change on the page came to, if anything.
fn note(shown: &SharedShown) -> Option<AnyElement> {
    let text = shown.borrow().note.clone()?;
    Some(div().id("settings-note").pt_3().text_color(hsla(color::MUTED)).child(text).test_support().into_any_element())
}

/// Every command that works now, with its key: Change waits for the next key pressed, Reset
/// puts its own key back, and the clear button leaves it without one. Those of features turned
/// off are left out.
fn shortcuts_page(shown: &SharedShown, workspace: &WeakEntity<Workspace>) -> gpui_kit::Div {
    use crate::keymap;
    let now = shown.borrow().settings.clone();
    let recording = shown.borrow().recording.clone();
    let keys: std::collections::BTreeMap<&str, Option<String>> = keymap::keys_in_use(&now.shortcuts).into_iter().collect();
    let reset_all = {
        let (shown, workspace) = (shown.clone(), workspace.clone());
        Button::new("shortcuts-reset-all").outline().small().label("Put Them All Back").disabled(now.shortcuts.custom.is_empty()).on_click(move |_, window, cx| {
            change(&shown, &workspace, window, cx, |settings| settings.shortcuts.custom.clear());
            shown.borrow_mut().note = Some("Every shortcut is Lightroom's again.".to_owned());
        })
    };
    let mut page_div = div().child(page(Section::Shortcuts)).child(card(vec![setting(format!("{} changed", now.shortcuts.custom.len()), "Commands given keys of their own. Recipes' keys are set under Recipes.", reset_all)]));
    let mut groups: Vec<&str> = Vec::new();
    for (_, _, group) in keymap::commands() {
        if !groups.contains(&group) {
            groups.push(group);
        }
    }
    for group in groups {
        let rows: Vec<AnyElement> = keymap::commands()
            .filter(|(id, _, in_group)| *in_group == group && now.command_on(id) && keys.contains_key(id))
            .map(|(id, name, _)| {
                let key = keys.get(id).cloned().flatten();
                let waiting = recording == Some(Recording::Command(id));
                let changed = now.shortcuts.custom.contains_key(id);
                let record = {
                    let shown = shown.clone();
                    Button::new(gpui_kit::ElementId::Name(format!("shortcut-change:{id}").into())).small().map(|button| if waiting { button.primary().label("Press a key…") } else { button.ghost().label("Change") }).on_click(move |_, window, _| {
                        let mut now = shown.borrow_mut();
                        now.recording = if waiting { None } else { Some(Recording::Command(id)) };
                        now.note = None;
                        drop(now);
                        window.refresh();
                    })
                };
                let reset = {
                    let (shown, workspace) = (shown.clone(), workspace.clone());
                    Button::new(gpui_kit::ElementId::Name(format!("shortcut-reset:{id}").into())).ghost().small().label("Reset").disabled(!changed).on_click(move |_, window, cx| {
                        let taken = keymap::reset(&mut shown.borrow().settings.shortcuts.clone(), id);
                        change(&shown, &workspace, window, cx, move |settings| {
                            keymap::reset(&mut settings.shortcuts, id);
                        });
                        shown.borrow_mut().note = Some(match taken {
                            Some(other) => format!("{name} has its own key again. {} had it, and has no key now.", keymap::name(other)),
                            None => format!("{name} has its own key again."),
                        });
                    })
                };
                let clear = {
                    let (shown, workspace) = (shown.clone(), workspace.clone());
                    Button::new(gpui_kit::ElementId::Name(format!("shortcut-clear:{id}").into())).ghost().small().icon(Icon::new(IconName::X)).tooltip("No shortcut").disabled(key.is_none()).on_click(move |_, window, cx| {
                        change(&shown, &workspace, window, cx, move |settings| {
                            let _ = keymap::assign(&mut settings.shortcuts, id, None);
                        });
                        shown.borrow_mut().note = Some(format!("{name} has no shortcut now."));
                    })
                };
                div()
                    .h(px(36.0))
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .child(div().flex_1().child(name))
                    .child(key_chip(key.map(|key| keymap::display(&key))))
                    .child(record)
                    .child(reset)
                    .child(clear)
                    .into_any_element()
            })
            .collect();
        if !rows.is_empty() {
            page_div = page_div.child(group_title(group)).child(card(rows));
        }
    }
    page_div.children(note(shown))
}

/// The next key pressed while a command or a recipe waits for one: it becomes the shortcut.
/// Esc cancels. Keys pressed now don't run their own commands.
fn capture_key(shown: &SharedShown, workspace: &WeakEntity<Workspace>, event: &gpui_kit::KeyDownEvent, window: &mut Window, cx: &mut App) {
    let Some(recording) = shown.borrow().recording.clone() else { return };
    cx.stop_propagation();
    let key = event.keystroke.unparse(gpui_kit::OperatingSystem::Mac);
    // A modifier pressed on its own isn't the key yet.
    if matches!(event.keystroke.key.as_str(), "shift" | "control" | "alt" | "platform" | "function" | "cmd" | "ctrl") {
        return;
    }
    shown.borrow_mut().recording = None;
    if key == "escape" {
        window.refresh();
        return;
    }
    let note = match recording {
        Recording::Command(id) => {
            let name = crate::keymap::name(id);
            let mut trial = shown.borrow().settings.shortcuts.clone();
            match crate::keymap::assign(&mut trial, id, Some(&key)) {
                Err(recipe) => format!("{} runs the recipe “{recipe}”, so {name} keeps its key. Change the recipe's key under Recipes first.", crate::keymap::display(&key)),
                Ok(taken) => {
                    let assigned = key.clone();
                    change(shown, workspace, window, cx, move |settings| {
                        let _ = crate::keymap::assign(&mut settings.shortcuts, id, Some(&assigned));
                    });
                    match taken {
                        Some(other) => format!("{} is now {name}. It was {}, which has no shortcut now.", crate::keymap::display(&key), crate::keymap::name(other)),
                        None => format!("{} is now {name}.", crate::keymap::display(&key)),
                    }
                }
            }
        }
        Recording::Recipe(name) => recipe_change(workspace, cx, crate::panels::iris::RecipeChange::Key { name, key: Some(key) }),
    };
    shown.borrow_mut().note = Some(note);
    window.refresh();
}

/// Makes a change to Iris's recipes and says what came of it.
fn recipe_change(workspace: &WeakEntity<Workspace>, cx: &mut App, change: crate::panels::iris::RecipeChange) -> String {
    match workspace.update(cx, |this, cx| {
        let said = crate::panels::iris::change_recipes(this, change, cx);
        cx.notify();
        said
    }) {
        Ok(Ok(said)) | Ok(Err(said)) => said,
        Err(_) => "The window has closed.".to_owned(),
    }
}

/// One recipe: its name, steps and what runs it, with the controls to change that. A taught
/// one can also be edited, moved and deleted.
fn recipe_card(recipe: &::iris::recipes::Recipe, taught: bool, place: usize, shown: &SharedShown, workspace: &WeakEntity<Workspace>) -> AnyElement {
    use ::iris::recipes::{Trigger, key_label};
    let name = recipe.name.clone();
    let id = |what: &str| gpui_kit::ElementId::Name(format!("recipe-{what}:{name}").into());
    let waiting = shown.borrow().recording == Some(Recording::Recipe(name.clone()));
    let takes_values = !recipe.slots().is_empty();
    let act = |change: crate::panels::iris::RecipeChange| {
        let (shown, workspace) = (shown.clone(), workspace.clone());
        move |window: &mut Window, cx: &mut App| {
            let said = recipe_change(&workspace, cx, change.clone());
            shown.borrow_mut().note = Some(said);
            window.refresh();
        }
    };
    let record = {
        let (shown, name) = (shown.clone(), name.clone());
        Button::new(id("key")).small().disabled(takes_values).map(|button| if waiting { button.primary().label("Press a key…") } else { button.ghost().label("Record Key") }).on_click(move |_, window, _| {
            let mut now = shown.borrow_mut();
            now.recording = if waiting { None } else { Some(Recording::Recipe(name.clone())) };
            now.note = None;
            drop(now);
            window.refresh();
        })
    };
    let clear_key = {
        let run = act(crate::panels::iris::RecipeChange::Key { name: name.clone(), key: None });
        Button::new(id("no-key")).ghost().small().icon(Icon::new(IconName::X)).tooltip("No key").disabled(recipe.key.is_none()).on_click(move |_, window, cx| run(window, cx))
    };
    let button = {
        let (on, name) = (recipe.button, name.clone());
        let (shown, workspace) = (shown.clone(), workspace.clone());
        Checkbox::new(id("button")).label("Button beside the chat").checked(on).disabled(takes_values).on_click(move |checked: &bool, window, cx| {
            let said = recipe_change(&workspace, cx, crate::panels::iris::RecipeChange::Button { name: name.clone(), on: *checked });
            shown.borrow_mut().note = Some(said);
            window.refresh();
        })
    };
    let moments = Trigger::ALL.into_iter().enumerate().map(|(index, trigger)| {
        let (on, name) = (recipe.when.contains(&trigger), name.clone());
        let (shown, workspace) = (shown.clone(), workspace.clone());
        let label = match trigger {
            Trigger::BeforeExport => "Before export",
            Trigger::OnOpen => "On open",
            Trigger::BeforeSave => "Before save",
            Trigger::AfterImport => "After import",
        };
        Checkbox::new(gpui_kit::ElementId::Name(format!("recipe-when-{index}:{name}").into())).label(label).checked(on).on_click(move |checked: &bool, window, cx| {
            let said = recipe_change(&workspace, cx, crate::panels::iris::RecipeChange::Trigger { name: name.clone(), trigger, on: *checked });
            shown.borrow_mut().note = Some(said);
            window.refresh();
        })
    });
    let mut actions: Vec<AnyElement> = Vec::new();
    if taught {
        let edit = {
            let (shown, recipe) = (shown.clone(), recipe.clone());
            Button::new(id("edit")).ghost().small().label("Edit").on_click(move |_, window, cx| {
                let (name, steps) = (recipe.name.clone(), recipe.steps.join("\n"));
                let name_input = cx.new(|cx| InputState::new(window, cx).default_value(name.clone()));
                let steps_input = cx.new(|cx| TextareaState::new(window, cx).rows(4).default_value(steps));
                shown.borrow_mut().editing = Some(Editor { name, name_input, steps_input });
                window.refresh();
            })
        };
        let up = {
            let run = act(crate::panels::iris::RecipeChange::Move { name: name.clone(), up: true });
            Button::new(id("up")).ghost().small().label("↑").tooltip("Move up").disabled(place == 0).on_click(move |_, window, cx| run(window, cx))
        };
        let down = {
            let run = act(crate::panels::iris::RecipeChange::Move { name: name.clone(), up: false });
            Button::new(id("down")).ghost().small().label("↓").tooltip("Move down").on_click(move |_, window, cx| run(window, cx))
        };
        let delete = {
            let (shown, workspace, recipe) = (shown.clone(), workspace.clone(), recipe.clone());
            Button::new(id("delete")).ghost().small().icon(Icon::new(IconName::Trash)).tooltip("Delete (Undo brings it back)").on_click(move |_, window, cx| {
                let said = recipe_change(&workspace, cx, crate::panels::iris::RecipeChange::Delete { name: recipe.name.clone() });
                let mut now = shown.borrow_mut();
                now.deleted = Some((recipe.clone(), place));
                now.note = Some(said);
                drop(now);
                window.refresh();
            })
        };
        actions.extend([edit.into_any_element(), up.into_any_element(), down.into_any_element(), delete.into_any_element()]);
    } else if !recipe.when.is_empty() || recipe.key.is_some() || recipe.button {
        // One Develop comes with goes back to how it came.
        let run = act(crate::panels::iris::RecipeChange::Delete { name: name.clone() });
        actions.push(Button::new(id("reset")).ghost().small().label("Reset").tooltip("No key, button or moments").on_click(move |_, window, cx| run(window, cx)).into_any_element());
    }
    let editing = shown.borrow().editing.as_ref().filter(|editor| editor.name == name).map(|editor| (editor.name_input.clone(), editor.steps_input.clone()));
    let body = match editing {
        Some((name_input, steps_input)) => {
            let save = {
                let (shown, workspace, old) = (shown.clone(), workspace.clone(), name.clone());
                let (name_input, steps_input) = (name_input.clone(), steps_input.clone());
                Button::new(id("save")).primary().small().label("Save").on_click(move |_, window, cx| {
                    let new_name = name_input.read(cx).value().to_string();
                    let steps: Vec<String> = steps_input.read(cx).value().lines().map(str::to_owned).collect();
                    let said = recipe_change(&workspace, cx, crate::panels::iris::RecipeChange::Edit { name: old.clone(), new_name, steps });
                    let mut now = shown.borrow_mut();
                    // A change that won't do keeps the editor open, to put right.
                    if said.starts_with("Saved") {
                        now.editing = None;
                    }
                    now.note = Some(said);
                    drop(now);
                    window.refresh();
                })
            };
            let cancel = {
                let shown = shown.clone();
                Button::new(id("cancel")).ghost().small().label("Cancel").on_click(move |_, window, _| {
                    shown.borrow_mut().editing = None;
                    window.refresh();
                })
            };
            div()
                .flex()
                .flex_col()
                .gap_1p5()
                .child(Input::new(&name_input).small())
                .child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child("Steps, one per line, each as you'd say it to Iris:"))
                .child(Textarea::new(&steps_input).h(px(96.0)))
                .child(div().flex().gap_1p5().child(save).child(cancel))
                .into_any_element()
        }
        None => div()
            .flex()
            .flex_col()
            .gap_0p5()
            .child(div().flex().items_center().gap_1p5().child(div().flex_1().font_weight(gpui_kit::FontWeight::MEDIUM).child(recipe.called())).children(actions))
            .child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(recipe.steps.join(" → ")))
            .into_any_element(),
    };
    div()
        .id(id("card"))
        .px_3()
        .py_2p5()
        .flex()
        .flex_col()
        .gap_1p5()
        .child(body)
        .child(div().flex().flex_wrap().items_center().gap_1p5().child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child("Key")).child(key_chip(recipe.key.as_deref().map(key_label))).child(record).child(clear_key).child(div().w(px(8.0))).child(button))
        .child(div().flex().flex_wrap().items_center().gap_2().child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child("Runs by itself")).children(moments))
        .test_support()
        .into_any_element()
}

/// Settings ▸ Recipes: the recipes taught to Iris and the ones Omina Develop comes with, each
/// with what runs it; and sharing them as a file.
fn recipes_page(shown: &SharedShown, workspace: &WeakEntity<Workspace>, cx: &App) -> gpui_kit::Div {
    let (taught, own) = workspace.upgrade().map(|workspace| crate::panels::iris::recipes_page_data(workspace.read(cx))).unwrap_or_default();
    let taught_cards: Vec<AnyElement> = if taught.is_empty() {
        vec![div().px_3().py_2p5().text_color(hsla(color::MUTED)).child("None yet. Teach Iris one in her chat: “teach tidy: auto, then sharpen”.").into_any_element()]
    } else {
        taught.iter().enumerate().map(|(place, recipe)| recipe_card(recipe, true, place, shown, workspace)).collect()
    };
    let own_cards: Vec<AnyElement> = own.iter().enumerate().map(|(place, recipe)| recipe_card(recipe, false, place, shown, workspace)).collect();
    let undo = shown.borrow().deleted.clone().map(|(recipe, at)| {
        let (shown, workspace) = (shown.clone(), workspace.clone());
        Button::new("recipes-undo-delete").outline().small().label(format!("Undo Delete “{}”", recipe.called())).on_click(move |_, window, cx| {
            let said = recipe_change(&workspace, cx, crate::panels::iris::RecipeChange::Restore { recipe: recipe.clone(), at });
            let mut now = shown.borrow_mut();
            now.deleted = None;
            now.note = Some(said);
            drop(now);
            window.refresh();
        })
    });
    let export = {
        let (shown, workspace) = (shown.clone(), workspace.clone());
        Button::new("recipes-export").outline().small().label("Export…").on_click(move |_, _, cx| {
            let Some(text) = workspace.upgrade().map(|workspace| crate::panels::iris::recipes_share_text(workspace.read(cx))) else { return };
            let folder = std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_default();
            let chosen = cx.prompt_for_new_path(&folder, Some("Omina Develop recipes.json"));
            let shown = shown.clone();
            cx.spawn(async move |cx| {
                let Ok(Ok(Some(path))) = chosen.await else { return };
                let said = match develop_core::image_io::write_atomically(&path, text.as_bytes()) {
                    Ok(()) => format!("Exported the recipes to {}.", path.display()),
                    Err(why) => format!("The recipes couldn't be exported: {why}"),
                };
                shown.borrow_mut().note = Some(said);
                cx.update(|cx| cx.refresh_windows());
            })
            .detach();
        })
    };
    let import = {
        let (shown, workspace) = (shown.clone(), workspace.clone());
        Button::new("recipes-import").outline().small().label("Import…").on_click(move |_, _, cx| {
            let chosen = cx.prompt_for_paths(gpui_kit::PathPromptOptions { files: true, directories: false, multiple: false, prompt: Some("Import".into()) });
            let (shown, workspace) = (shown.clone(), workspace.clone());
            cx.spawn(async move |cx| {
                let Ok(Ok(Some(paths))) = chosen.await else { return };
                let Some(path) = paths.into_iter().next() else { return };
                let said = match std::fs::read_to_string(&path) {
                    Err(error) => format!("{} can't be read: {error}", path.display()),
                    Ok(text) => cx.update(|cx| recipe_change(&workspace, cx, crate::panels::iris::RecipeChange::Import { text }))
                };
                shown.borrow_mut().note = Some(said);
                cx.update(|cx| cx.refresh_windows());
            })
            .detach();
        })
    };
    div()
        .child(page(Section::Recipes))
        .child(card(vec![setting("Share recipes", "Write your recipes to a file to keep or give to someone, or add the recipes from one.", div().flex().gap_1p5().child(export).child(import))]))
        .child(group_title("Your recipes"))
        .child(card(taught_cards))
        .children(undo.map(|undo| div().pt_2().child(undo)))
        .child(group_title("Comes with Omina Develop"))
        .child(card(own_cards))
        .children(note(shown))
}

fn about_page() -> gpui_kit::Div {
    div().child(page(Section::About)).child(card(vec![
        setting(format!("Omina Develop {}", env!("CARGO_PKG_VERSION")), "Lightroom-style photo editing, built in Rust beside Omina Motion and Omina Edit.", div()),
        setting("License", "Free software under the GNU General Public License, version 3 or later.", div()),
    ]))
}

fn section_menu(shown: &SharedShown) -> gpui_kit::Div {
    let current = shown.borrow().section;
    let listed = Section::ALL.into_iter().enumerate().filter(|(_, section)| section.shown(&shown.borrow().settings)).collect::<Vec<_>>();
    div().w(px(196.0)).flex_none().flex().flex_col().pr_3().border_r_1().border_color(hsla(color::HAIRLINE)).children(listed.into_iter().map(|(index, section)| {
        let shown = shown.clone();
        let selected = section == current;
        div()
            .id(("settings-section", index))
            .h(px(30.0))
            .px_2()
            .flex()
            .items_center()
            .gap_2()
            .rounded(px(6.0))
            .cursor_pointer()
            .text_size(px(size::LARGE_TEXT))
            .text_color(hsla(if selected { color::TEXT } else { color::MUTED }))
            .when(selected, |item| item.bg(hsla(color::ROW_SELECTED)))
            .hover(|style| style.bg(hsla(color::HOVER)).text_color(hsla(color::TEXT)))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(move |_, window, _| {
                let mut now = shown.borrow_mut();
                now.section = section;
                now.note = None;
                now.recording = None;
                drop(now);
                window.refresh();
            })
            .child(Icon::new(section.icon()).size(px(14.0)).text_color(hsla(if selected { color::ACCENT } else { color::MUTED })))
            .child(section.name())
            .test_support()
    }))
}

pub fn open(_workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let shown: SharedShown = Rc::new(RefCell::new(Shown { section: Section::General, settings: app_settings::load(), recording: None, note: None, editing: None, deleted: None }));
    let target = cx.weak_entity();
    // Holds the keyboard while a shortcut or a recipe's key is being recorded.
    let capture = cx.focus_handle();
    super::chrome::open_dialog(window, cx, move |dialog, window, cx| {
        // Waiting for a key: the keyboard comes here, so the key pressed is caught.
        if shown.borrow().recording.is_some() && !capture.is_focused(window) {
            window.focus(&capture, cx);
        }
        // A section whose feature was just turned off gives way to General.
        let section = Some(shown.borrow().section).filter(|section| section.shown(&shown.borrow().settings)).unwrap_or(Section::General);
        shown.borrow_mut().section = section;
        let content = match section {
            Section::General => general(&shown, &target),
            Section::Appearance => appearance(&shown, &target),
            Section::Viewer => viewer_page(&shown, &target),
            Section::Features => features_page(&shown, &target),
            Section::Presets => presets_page(&shown, &target),
            Section::Iris => iris_page(&shown, &target),
            Section::Recipes => recipes_page(&shown, &target, cx),
            Section::Shortcuts => shortcuts_page(&shown, &target),
            Section::About => about_page(),
        };
        dialog
            .title("Settings")
            .w(px(820.0))
            .child(div().h(px(500.0)).flex().gap_5().text_size(px(size::TEXT)).child(section_menu(&shown)).child({
                let (shown, target) = (shown.clone(), target.clone());
                div()
                    .id("settings-content")
                    .flex_1()
                    .min_w_0()
                    .overflow_y_scroll()
                    .pr_2()
                    .pb_2()
                    .track_focus(&capture)
                    .capture_key_down(move |event, window, cx| capture_key(&shown, &target, event, window, cx))
                    .child(content)
                    .test_support()
            }))
            .footer(div().w_full().flex().justify_end().child(Button::new("settings-done").primary().small().px_4().label("Done").on_click(|_, window, cx| window.close_dialog(cx))))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_intervals_read_as_people_say_them() {
        assert_eq!(recovery_label(30), "Every 30 seconds");
        assert_eq!(recovery_label(60), "Every minute");
        assert_eq!(recovery_label(300), "Every 5 minutes");
        assert_eq!(recovery_label(0), "Never");
    }
}

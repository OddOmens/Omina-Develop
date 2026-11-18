//! Iris, the assistant Develop shares with Studio and Canvas, opened from the rail. It has two
//! tabs. Suggestions lists what her rules notice in the photo (blown highlights, crushed
//! shadows, too much contrast, a color cast…; see `develop_core::assistant`), each with a button
//! that fixes it and a way to dismiss it. Chat is a conversation: a message typed at the foot of
//! the panel is answered from the photo itself or makes the edit it asks for ("a bit brighter",
//! "shadows +30"; see `develop_core::chat`); a how-to question ("how do I crop?") is answered
//! from the help every app shares (the `iris` crate's knowledge), with a button that does it
//! when it can be done here. Nothing here is AI: every answer comes from a plain rule, a phrase
//! she knows or help written by hand.
//!
//! This file draws the panel and keeps what Iris measured. What happens to a message once it
//! is sent is in `conversation`; doing one thing to several open photos is in `batch`, and to a
//! folder of photos in `folder`; sending a photo to the other apps and bringing theirs in is in
//! `handoff`.
//!
//! The conversation and what was typed are kept in Iris's shared folder between runs
//! (`iris::conversation`): what was said before is shown above a quiet "Earlier" line. The
//! message field suggests the commands and recipes a half-typed message could become (Tab
//! takes the first), and the up and down arrows go back through what was typed.
//!
//! Her name and whether she blinks are shared with the other apps (`iris::settings`), so a
//! rename in one is a rename in all. She can be turned off in Settings ▸ Iris, as Studio's
//! Features can; then her rail button, badge and measuring all stop.
//!
//! The rules read a small render of the finished photo, made off the UI thread whenever the
//! edit settles. With the badge on, Iris's face sits in a corner of the photo with a count of
//! what it noticed; a click opens its suggestions.

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use develop_core::assistant::{Area, Check, Measure, Options, Suggestion, neutral_color, suggestions};
use develop_core::{Edit, View};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Icon, Selectable as _, Sizable as _};
use gpui_kit::{
    AnyElement, AppContext as _, Context, ElementId, Entity, InteractiveElement, IntoElement, ParentElement, ScrollHandle, StatefulInteractiveElement, Styled, Subscription,
    TestSupportExt as _, Window, div, prelude::FluentBuilder as _, px,
};

use super::chrome::panel;
use crate::app_settings::AppSettings;
use crate::features::Feature;
use crate::preview::{Pyramid, source_id};
use crate::theme::{color, hsla, size};
use crate::workspace::{LeftPanel, PanelId, Workspace};

mod batch;
mod conversation;
mod folder;
mod handoff;
mod recipe_book;

pub use conversation::{run_recipe, run_trigger, send};
pub use recipe_book::{RecipeChange, change_recipes, recipes_page_data, share_text as recipes_share_text};
pub use handoff::receive as receive_handoffs;

/// The longest side of the render the rules measure, in pixels.
const MEASURE_SIDE: u32 = 512;
/// The most completions shown under the message field.
const MOST_COMPLETIONS: usize = 5;
/// Iris reads the shared settings again every this many blinks, to pick up a rename made in
/// another app.
const SHARED_EVERY_BLINKS: u32 = 4;

/// The folder Iris's shared settings and recipes live in: the one every app reads, or (in tests)
/// a folder inside the test thread's own settings folder, so tests never touch the real one or
/// each other's.
pub(crate) fn shared_folder() -> Option<PathBuf> {
    if cfg!(test) {
        return Some(crate::app_settings::folder().join("iris"));
    }
    ::iris::settings::default_folder()
}

/// The settings shared with Studio and Canvas. When no app has written them yet, they start
/// from the name Develop kept itself (unless that is the old name, Tween) and are saved for the
/// others.
pub fn load_shared(settings: &AppSettings) -> ::iris::settings::Shared {
    let folder = shared_folder();
    if let Some(shared) = folder.as_deref().and_then(::iris::settings::load) {
        return shared;
    }
    let mut shared = ::iris::settings::Shared::default();
    let older = ::iris::settings::tidy_name(&settings.assistant_name);
    if !older.is_empty() && older != "Tween" {
        shared.name = older;
    }
    if let Some(folder) = folder {
        save_shared(&folder, &shared);
    }
    shared
}

/// Writes the shared settings, saying so if they couldn't be written (the change then lasts
/// only until Develop quits).
fn save_shared(folder: &std::path::Path, shared: &::iris::settings::Shared) {
    if let Err(error) = ::iris::settings::save(folder, shared) {
        eprintln!("could not save Iris's shared settings: {error}");
    }
}

/// Changes the shared settings and saves them for the other apps.
pub fn change_shared(workspace: &mut Workspace, change: impl FnOnce(&mut ::iris::settings::Shared)) {
    change(&mut workspace.ui.iris.shared);
    if let Some(folder) = shared_folder() {
        save_shared(&folder, &workspace.ui.iris.shared);
    }
}

/// What Iris is called, in every app.
pub fn name(workspace: &Workspace) -> String {
    workspace.ui.iris.shared.shown_name()
}

/// Shows Iris's Chat tab (`true`) or her Suggestions.
pub fn choose_tab(workspace: &mut Workspace, chat: bool) {
    workspace.ui.iris.chat_chosen = Some(chat);
}

/// Whether Iris is turned on (Settings ▸ Iris).
pub fn is_on(workspace: &Workspace) -> bool {
    workspace.ui.settings.iris_on
}

/// One message in the conversation.
pub struct Message {
    /// Whether the person wrote it, rather than Iris.
    from_person: bool,
    text: String,
    /// Whether the message ends with a button that opens the Suggestions tab.
    opens_suggestions: bool,
    /// For a help answer about something Develop does: that command's id, offered as "Do It".
    command: Option<String>,
    /// Short replies offered under the message while it is the newest: a click sends one as
    /// if it had been typed.
    replies: Vec<String>,
    /// Whether it is the progress line of a folder batch still working, kept up to date.
    progress: bool,
}

impl Message {
    fn from_iris(text: impl Into<String>) -> Self {
        Self { from_person: false, text: text.into(), opens_suggestions: false, command: None, replies: Vec::new(), progress: false }
    }
}

/// What Iris keeps between frames.
#[derive(Default)]
pub struct IrisUi {
    /// The photo (its original), edit and rules the findings below are for.
    measured_for: Option<(usize, Edit, Options)>,
    measuring: bool,
    /// The latest measurements, and what the rules made of them.
    measure: Option<Measure>,
    found: Vec<Suggestion>,
    /// Suggestions dismissed, by photo and rule, until brought back.
    dismissed: HashSet<(usize, Check)>,
    messages: Vec<Message>,
    /// The field a message is typed into, made the first time the workspace is drawn.
    ask: Option<(Entity<InputState>, Subscription)>,
    /// The tab chosen: Chat (`true`) or Suggestions. Until one is chosen, Suggestions shows when
    /// there are some, else Chat.
    chat_chosen: Option<bool>,
    /// How many tips have been given, to give the next.
    tips: usize,
    /// A passing feeling (happy or sad) and when it ends.
    feeling: Option<(Mood, Instant)>,
    /// How many times the face has blinked; each new number plays one blink.
    blinks: u32,
    /// When something last happened with the photo or with Iris, for falling asleep.
    active_at: Option<Instant>,
    /// The photo's revision when Iris last looked, to notice work going on.
    seen_revision: u64,
    /// Whether the blinking timer is running.
    blinking: bool,
    settings_open: bool,
    chat_scroll: ScrollHandle,
    /// Iris's name and manner, shared with the other apps.
    pub shared: ::iris::settings::Shared,
    /// What the front door every app shares remembers between messages: the last message
    /// that changed something (for "again") and the recipes.
    memory: ::iris::front::Memory,
    /// The last slider change Iris made, which "more" and "less" build on.
    last_adjust: Option<conversation::LastAdjust>,
    /// The last time Iris tightened the crop, which "more" and "less" build on in its place.
    last_crop: Option<conversation::LastTighten>,
    /// A change to several photos that is waiting for a yes or no.
    awaiting: Option<batch::Plan>,
    /// The last change made to several photos, which "undo that" takes back on all of them.
    last_batch: Option<batch::Done>,
    /// A folder batch waiting for a yes or no, and one being worked through.
    awaiting_folder: Option<folder::Plan>,
    folder_run: Option<folder::Running>,
    /// Whether a recipe is running by itself at a moment (before an export, say), so what it
    /// does doesn't set off that moment's recipes again.
    in_trigger: bool,
    /// How many of the messages were brought back from the last run, drawn above "Earlier".
    restored: usize,
    /// What was typed before, for the up and down arrows, and the text an arrow last put in
    /// the field (so the arrows keep going back only while it is still there).
    recall: ::iris::complete::Recall,
    recalled: Option<String>,
    /// What the half-typed message in the field could become.
    completions: Vec<::iris::complete::Completion>,
}

impl IrisUi {
    /// Iris as she starts, with the settings shared with the other apps, the recipes she has
    /// been taught, and the conversation from last time.
    pub fn new(settings: &AppSettings) -> Self {
        let saved = shared_folder().map(|folder| ::iris::conversation::load(&folder, ::iris::App::Develop)).unwrap_or_default();
        let messages: Vec<Message> = saved.lines.into_iter().map(|line| Message { from_person: line.from_person, ..Message::from_iris(line.text) }).collect();
        let mut memory = conversation::remembered();
        memory.app_keys = crate::keymap::app_keys();
        Self { shared: load_shared(settings), memory, restored: messages.len(), messages, recall: ::iris::complete::Recall::new(saved.typed), ..Self::default() }
    }

    /// Notes the keys Develop's own commands use now, so no recipe is given one of them.
    pub fn use_app_keys(&mut self) {
        self.memory.app_keys = crate::keymap::app_keys();
    }

    /// The recipes that have keys of their own, for binding them.
    pub fn recipe_keys(&self) -> Vec<(String, String)> {
        self.memory.keyed()
    }
}

/// The findings for the photo showing, if they are for it (a moment after switching photos
/// they may still be another's).
fn current(workspace: &Workspace) -> Option<(&Measure, &[Suggestion])> {
    let photo = workspace.state.photo()?;
    let iris = &workspace.ui.iris;
    let (source, _, _) = iris.measured_for.as_ref()?;
    if *source != source_id(photo) {
        return None;
    }
    Some((iris.measure.as_ref()?, iris.found.as_slice()))
}

/// What Iris notices in the photo showing, less what was dismissed.
pub fn open_suggestions(workspace: &Workspace) -> Vec<Suggestion> {
    let Some(photo) = workspace.state.photo() else { return Vec::new() };
    let source = source_id(photo);
    current(workspace).map(|(_, found)| found.iter().filter(|suggestion| !workspace.ui.iris.dismissed.contains(&(source, suggestion.check))).cloned().collect()).unwrap_or_default()
}

/// Measures the finished photo with `edit` from `pyramid`, and the near-gray color of the photo
/// as it came (for the color-cast fix).
fn measure(pyramid: &Pyramid, edit: &Edit) -> (Measure, Option<[f32; 3]>) {
    let level = pyramid.level_at_least(MEASURE_SIDE);
    let source = &pyramid.levels[level];
    let scale = pyramid.scale(level);
    let finished = edit.render(source, pyramid.size, scale, View::Result);
    let before = Edit { crop: edit.crop.clone(), ..Edit::default() }.render(source, pyramid.size, scale, View::Before);
    (Measure::of(&finished), neutral_color(&before))
}

/// Makes the message field the first time, and has the rules look at the photo again when its
/// edit has settled since they last did. Called from the workspace's render.
pub fn keep_up(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    if workspace.ui.iris.ask.is_none() {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Ask about the photo, or type /help"));
        let sent = cx.subscribe_in(&input, window, |this, input, event: &InputEvent, window, cx| match event {
            InputEvent::PressEnter { .. } => {
                let message = input.read(cx).value().to_string();
                input.update(cx, |input, cx| input.set_value("", window, cx));
                this.ui.iris.completions.clear();
                send(this, &message, window, cx);
            }
            // What the typing so far could become.
            InputEvent::Change => {
                let typed = input.read(cx).value().to_string();
                let known = this.ui.iris.memory.known();
                this.ui.iris.completions = ::iris::complete::suggestions(&typed, develop_core::chat::COMMANDS, &known, MOST_COMPLETIONS);
                cx.notify();
            }
            _ => {}
        });
        workspace.ui.iris.ask = Some((input, sent));
    }
    if !is_on(workspace) {
        return;
    }
    start_blinking(workspace, window, cx);
    // Work on the photo keeps Iris awake.
    if workspace.ui.iris.active_at.is_none() || workspace.ui.iris.seen_revision != workspace.state.revision {
        workspace.ui.iris.seen_revision = workspace.state.revision;
        workspace.ui.iris.active_at = Some(Instant::now());
    }
    let Some(photo) = workspace.state.photo() else { return };
    let busy = workspace.ui.slider_pressed || workspace.state.interaction.is_some() || cx.has_active_drag();
    let wanted = (source_id(photo), photo.edit.clone(), workspace.ui.settings.assistant.clone());
    let iris = &workspace.ui.iris;
    if busy || iris.measuring || iris.measured_for.as_ref() == Some(&wanted) {
        return;
    }
    let Some(pyramid) = workspace.ui.preview.pyramid_of(photo) else { return };
    workspace.ui.iris.measuring = true;
    cx.spawn_in(window, async move |this, cx| {
        let (source, edit, options) = wanted;
        let job_edit = edit.clone();
        let job_options = options.clone();
        let (measured, found) = cx
            .background_spawn(async move {
                let (measured, neutral) = measure(&pyramid, &job_edit);
                let found = suggestions(&measured, neutral, &job_edit, &job_options);
                (measured, found)
            })
            .await;
        this.update(cx, |this, cx| {
            let iris = &mut this.ui.iris;
            iris.measuring = false;
            iris.measure = Some(measured);
            iris.found = found;
            iris.measured_for = Some((source, edit, options));
            cx.notify();
        })
        .ok();
    })
    .detach();
}

/// Makes one suggestion's fix.
fn fix(workspace: &mut Workspace, suggestion: &Suggestion) {
    if let Some(fix) = &suggestion.fix {
        workspace.state.apply_fix(fix);
        workspace.ui.hover_edit = None;
        feel(workspace, Mood::Happy);
    }
}

fn show_newest(window: &mut Window, cx: &mut Context<Workspace>) {
    cx.on_next_frame(window, |this, _, cx| {
        this.ui.iris.chat_scroll.scroll_to_bottom();
        cx.notify();
    });
    cx.notify();
}

/// How Iris feels, shown by its eyes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mood {
    #[default]
    Awake,
    /// Nothing has happened for a while: eyes shut.
    Asleep,
    /// Measuring the photo: eyes glance up.
    Thinking,
    /// Just fixed something: eyes curve into a smile.
    Happy,
    /// Didn't understand a message: eyes droop.
    Sad,
}

/// How long nothing must change before Iris falls asleep.
const SLEEP_AFTER: Duration = Duration::from_secs(120);
/// How long it stays happy or sad after what caused it.
const FEELING_LASTS: Duration = Duration::from_secs(4);
/// How long the face waits between blinks, used in turn so it doesn't look mechanical.
const BLINK_PAUSES: [Duration; 5] = [Duration::from_millis(3200), Duration::from_millis(5100), Duration::from_millis(2400), Duration::from_millis(6300), Duration::from_millis(4000)];
/// How long one blink takes, from open to closed and back.
const BLINK_LENGTH: Duration = Duration::from_millis(240);
/// Which way the eyes glance after each blink, taken in turn: mostly straight ahead.
const GLANCES: [(f32, f32); 8] = [(0.0, 0.0), (-1.0, 0.0), (0.0, 0.0), (1.0, 0.0), (0.0, 0.0), (0.6, -0.8), (0.0, 0.0), (-0.6, 0.6)];

/// Makes Iris happy or sad for a few seconds, and wakes it.
fn feel(workspace: &mut Workspace, mood: Mood) {
    workspace.ui.iris.feeling = Some((mood, Instant::now() + FEELING_LASTS));
    workspace.ui.iris.active_at = Some(Instant::now());
}

/// Iris's mood now: what it is doing comes first, then how it feels, then sleep.
pub fn mood(workspace: &Workspace) -> Mood {
    let iris = &workspace.ui.iris;
    let now = Instant::now();
    if iris.measuring && current(workspace).is_none() {
        Mood::Thinking
    } else if let Some((feeling, _)) = iris.feeling.filter(|(_, until)| now < *until) {
        feeling
    } else if iris.active_at.is_some_and(|at| now.duration_since(at) >= SLEEP_AFTER) {
        Mood::Asleep
    } else {
        Mood::Awake
    }
}

/// How the face is drawn: its mood, the blink to play, and where the eyes look (each from -1
/// to 1: across, then down).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Look {
    pub mood: Mood,
    pub blink: Option<u32>,
    pub glance: (f32, f32),
}

/// How the face should be drawn now.
pub fn current_look(workspace: &Workspace) -> Look {
    let blinks = workspace.ui.iris.blinks;
    // With blinking switched off (in any app) she holds still.
    let awake = blinks > 0 && workspace.ui.iris.shared.blinks;
    Look { mood: mood(workspace), blink: awake.then_some(blinks), glance: if awake { GLANCES[blinks as usize % GLANCES.len()] } else { (0.0, 0.0) } }
}

/// Starts the timer that blinks the face now and then and notices when nothing has happened
/// for a while (Iris falls asleep). Runs as long as the window does.
fn start_blinking(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    if workspace.ui.iris.blinking || cfg!(test) {
        return;
    }
    workspace.ui.iris.blinking = true;
    cx.spawn_in(window, async move |this, cx| {
        let mut turn = 0usize;
        loop {
            cx.background_executor().timer(BLINK_PAUSES[turn % BLINK_PAUSES.len()]).await;
            turn += 1;
            let alive = this.update(cx, |this, cx| {
                let iris = &mut this.ui.iris;
                iris.blinks = iris.blinks.wrapping_add(1);
                // A rename made in Studio or Canvas shows here within a few blinks.
                if iris.blinks.is_multiple_of(SHARED_EVERY_BLINKS)
                    && let Some(shared) = shared_folder().as_deref().and_then(::iris::settings::load)
                {
                    iris.shared = shared;
                }
                cx.notify();
            });
            if alive.is_err() {
                break;
            }
        }
    })
    .detach();
}

/// Iris's face: a circle in the accent color with two eyes, so its suggestions read as coming
/// from a helper rather than as error messages. The eyes show its mood, turn the way the look
/// says, close and open once each time the blink count changes, and narrow into a smile under
/// the pointer. `place` names where the face is drawn, so each place animates on its own.
pub fn face(place: &'static str, side: f32, look: Look) -> impl IntoElement {
    use gpui_kit::{Animation, AnimationExt as _};
    const GROUP: &str = "iris-face";
    let (open_width, open_height, inset) = (side * 0.16, side * 0.32, side * 0.27);
    let (width, height, across, down) = match look.mood {
        Mood::Awake => (open_width, open_height, look.glance.0 * side * 0.06, look.glance.1 * side * 0.05),
        Mood::Asleep => (side * 0.2, side * 0.05, 0.0, side * 0.05),
        Mood::Thinking => (open_width, open_height, side * 0.06, -side * 0.08),
        Mood::Happy => (open_width * 1.15, open_height * 0.5, 0.0, -side * 0.02),
        Mood::Sad => (open_width * 1.1, open_height * 0.55, 0.0, side * 0.07),
    };
    let top = (side - height) / 2.0 + down;
    let eye = |index: u32| {
        let left = if index == 0 { inset } else { side - inset - open_width } + (open_width - width) / 2.0 + across;
        let smiling = open_height * 0.5;
        let shape = div().absolute().w(px(width)).h(px(height)).top(px(top)).left(px(left)).bg(gpui_kit::white());
        // A smile is flat underneath and a sad eye is flat on top; the rest are rounded all round.
        let shape = match look.mood {
            Mood::Happy => shape.rounded_t(px(width)),
            Mood::Sad => shape.rounded_b(px(width)),
            _ => shape.rounded_full().group_hover(GROUP, move |style| style.h(px(smiling)).top(px((side - smiling) / 2.0))),
        };
        match (look.mood, look.blink) {
            (Mood::Asleep, _) | (_, None) => shape.into_any_element(),
            // The eye squeezes shut around its middle and opens again.
            (_, Some(blink)) => shape
                .with_animation(ElementId::Name(format!("iris-eye:{place}:{index}:{blink}").into()), Animation::new(BLINK_LENGTH), move |eye, progress| {
                    let shut = height * (1.0 - 0.88 * (progress * std::f32::consts::PI).sin());
                    eye.h(px(shut)).top(px(top + (height - shut) / 2.0))
                })
                .into_any_element(),
        }
    };
    div().group(GROUP).relative().size(px(side)).flex_none().rounded_full().bg(hsla(color::ACCENT)).child(eye(0)).child(eye(1))
}

/// The color that marks an area's suggestions.
fn area_color(area: Area) -> u32 {
    match area {
        Area::Light => 0xf5b942,
        Area::Contrast => 0x9b8cff,
        Area::Color => 0x4cc2ff,
        Area::Detail => 0x5fd38d,
    }
}

/// The small colored label saying which area a suggestion is about.
fn area_tag(area: Area) -> impl IntoElement {
    let tint = gpui_kit::rgb(area_color(area));
    div().flex_none().px_1p5().rounded_full().bg(gpui_kit::Hsla::from(tint).opacity(0.18)).text_size(px(size::CAPTION_TEXT)).text_color(tint).child(area.name())
}

/// What Iris says above its list.
fn greeting(workspace: &Workspace, count: usize) -> String {
    if workspace.state.photo().is_none() {
        return "Open a photo and I'll look it over.".to_owned();
    }
    if current(workspace).is_none() {
        return "Looking at the photo…".to_owned();
    }
    match count {
        0 => "Looking good. I have nothing to suggest.".to_owned(),
        1 => "I noticed one thing in this photo.".to_owned(),
        _ => format!("I noticed {count} things in this photo."),
    }
}

/// One suggestion: what was noticed, why it matters, what was measured, and its buttons.
fn card(index: usize, suggestion: Suggestion, workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement {
    let (for_fix, check) = (suggestion.clone(), suggestion.check);
    // Resting the pointer on a fix shows it on the photo before it's made.
    let fix_button = suggestion.fix.as_ref().map(|found| {
        let previewed = found.clone();
        div()
            .id(("iris-fix-preview", index))
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                this.ui.hover_edit = if *hovered { this.state.photo().map(|photo| (source_id(photo), previewed.apply_to_edit(&photo.edit))) } else { None };
                cx.notify();
            }))
            .child(Button::new(("iris-fix", index)).primary().small().label("Fix It").tooltip("Rest the pointer here to preview it on the photo; click to make it").on_click(cx.listener(move |this, _, _, cx| {
                fix(this, &for_fix);
                cx.notify();
            })))
    });
    // For clipping, a button that paints where the detail is lost.
    // It goes with the Clipping Warning, when that is turned off.
    let clipping = (matches!(check, Check::BlownHighlights | Check::CrushedShadows) && workspace.ui.settings.feature_on(Feature::Clipping)).then(|| {
        let showing = workspace.ui.show_clipping;
        Button::new(("iris-show", index)).outline().small().label(if showing { "Hide Clipping" } else { "Show Clipping" }).selected(showing).tooltip(crate::keymap::with_key("Paint clipped highlights red and shadows blue", "clipping")).on_click(cx.listener(|this, _, _, cx| {
            this.ui.show_clipping = !this.ui.show_clipping;
            cx.notify();
        }))
    });
    let dismiss = Button::new(("iris-dismiss", index)).ghost().small().label("Dismiss").tooltip("Hide this suggestion for this photo").on_click(cx.listener(move |this, _, _, cx| {
        if let Some(photo) = this.state.photo() {
            let source = source_id(photo);
            this.ui.iris.dismissed.insert((source, check));
        }
        cx.notify();
    }));
    div()
        .id(("iris-card", index))
        .mx_2()
        .p_2p5()
        .flex()
        .flex_col()
        .gap_1p5()
        .rounded(px(8.0))
        .bg(hsla(color::RAISED))
        .border_1()
        .border_color(hsla(color::HAIRLINE))
        .child(div().flex().child(area_tag(suggestion.check.area())))
        .child(div().font_weight(gpui_kit::FontWeight::MEDIUM).child(suggestion.title))
        .child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(suggestion.detail))
        // What was measured, so the suggestion can be judged rather than taken on trust.
        .child(div().text_size(px(size::CAPTION_TEXT)).text_color(hsla(color::MUTED)).opacity(0.75).child(suggestion.evidence))
        // And what the fix would set, in full (the button stays short).
        .children(suggestion.fix.as_ref().map(|found| div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::TEXT)).child(format!("Fix: {}", found.summary()))))
        .child(div().pt_1().flex().flex_wrap().items_center().gap_1p5().children(fix_button).children(clipping).child(div().flex_1()).child(dismiss))
        .test_support()
}

/// One message: the person's on the right, Iris's on the left beside its face. `newest` is
/// whether it is the last in the conversation, which is when its short replies are offered:
/// "Undo that" under an older message would undo something else by now.
fn bubble(index: usize, message: &Message, newest: bool, settings: &AppSettings, cx: &mut Context<Workspace>) -> AnyElement {
    let lines = message.text.lines().map(|line| div().child(line.to_owned())).collect::<Vec<_>>();
    if message.from_person {
        let said = div().max_w(gpui_kit::relative(0.85)).px_2p5().py_1p5().rounded(px(10.0)).bg(hsla(color::ACCENT).opacity(0.18)).children(lines);
        return div().id(("iris-message", index)).px_2().flex().justify_end().child(said).test_support().into_any_element();
    }
    let open = message.opens_suggestions.then(|| {
        Button::new(("iris-message-suggestions", index)).outline().small().label("Open Suggestions").on_click(cx.listener(|this, _, _, cx| {
            this.ui.iris.chat_chosen = Some(false);
            cx.notify();
        }))
    });
    // A help answer about something Develop does offers to do it.
    // Not for a command whose feature is turned off: it would do nothing.
    let do_it = message.command.clone().filter(|id| settings.command_on(id)).map(|id| {
        let tip = crate::keymap::key_for(&id).map_or_else(|| "Do it now".to_owned(), |key| format!("Do it now ({key})"));
        Button::new(("iris-message-do", index)).outline().small().label("Do It").tooltip(tip).on_click(cx.listener(move |_, _, window, cx| {
            if let Some(action) = crate::commands::action_for(&id) {
                window.dispatch_action(action, cx);
            }
            cx.notify();
        }))
    });
    let buttons = (open.is_some() || do_it.is_some()).then(|| div().pt_1().flex().gap_1p5().children(do_it).children(open));
    // The short replies: each sends its own words.
    let replies = (newest && !message.replies.is_empty()).then(|| {
        let buttons = message.replies.iter().enumerate().map(|(place, reply)| {
            let sent = reply.clone();
            Button::new(ElementId::Name(format!("iris-reply-{index}-{place}").into())).outline().small().label(reply.clone()).on_click(cx.listener(move |this, _, window, cx| {
                send(this, &sent, window, cx);
            }))
        });
        div().pt_1().flex().flex_wrap().gap_1p5().children(buttons.collect::<Vec<_>>())
    });
    let said = div()
        .flex_1()
        .min_w_0()
        .px_2p5()
        .py_1p5()
        .flex()
        .flex_col()
        .gap_0p5()
        .rounded(px(10.0))
        .bg(hsla(color::RAISED))
        .border_1()
        .border_color(hsla(color::HAIRLINE))
        .children(lines)
        .children(buttons)
        .children(replies);
    div().id(("iris-message", index)).px_2().flex().items_start().gap_1p5().child(face("message", 20.0, Look::default())).child(said).test_support().into_any_element()
}

/// The conversation: a welcome until something has been said, then every message in order.
fn conversation(workspace: &Workspace, cx: &mut Context<Workspace>) -> Vec<AnyElement> {
    let messages = &workspace.ui.iris.messages;
    if messages.is_empty() {
        let mut welcome = Message::from_iris(format!(
            "Hi! I'm {}. Ask me \"what's wrong?\" or \"is it overexposed?\", tell me what to change (\"a bit brighter\", \"shadows +30, highlights -20\", \"make it moody\", \"crop to 16:9\"), or ask how to do something (\"how do I darken a sky?\").\nType /help for everything I understand.",
            name(workspace)
        ));
        if workspace.state.photo().is_some() {
            welcome.replies = vec!["What's wrong?".to_owned(), "Is this ready?".to_owned(), "What can you do?".to_owned()];
        }
        return vec![bubble(0, &welcome, true, &workspace.ui.settings, cx)];
    }
    let last = messages.len() - 1;
    let restored = workspace.ui.iris.restored.min(messages.len());
    let mut drawn: Vec<AnyElement> = Vec::new();
    for (index, message) in messages.iter().enumerate() {
        drawn.push(bubble(index, message, index == last, &workspace.ui.settings, cx));
        // What was said before this run ends with a quiet line, so it reads as history.
        if index + 1 == restored {
            drawn.push(earlier().into_any_element());
        }
    }
    drawn
}

/// The quiet line between the conversation brought back from the last run and this one's.
fn earlier() -> impl IntoElement {
    let rule = || div().flex_1().h(px(1.0)).bg(hsla(color::HAIRLINE));
    div().id("iris-earlier").px_3().flex().items_center().gap_2().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(rule()).child("Earlier").child(rule()).test_support()
}

/// The recipes with a button of their own, in a strip at the top of the chat. A click runs one
/// as its name would.
fn recipe_buttons(workspace: &Workspace, cx: &mut Context<Workspace>) -> Option<impl IntoElement> {
    let names = workspace.ui.iris.memory.buttons();
    if names.is_empty() {
        return None;
    }
    let buttons = names.into_iter().enumerate().map(|(index, recipe)| {
        let label = workspace.ui.iris.memory.recipe(&recipe).map_or_else(|| recipe.clone(), |known| known.called());
        Button::new(("iris-recipe-button", index)).outline().small().label(label).tooltip("Run this recipe").on_click(cx.listener(move |this, _, window, cx| {
            run_recipe(this, &recipe, window, cx);
        }))
    });
    Some(div().px_2().flex().flex_wrap().gap_1p5().children(buttons.collect::<Vec<_>>()))
}

/// What the half-typed message could become, each a click away; Tab takes the first.
fn completions(workspace: &Workspace, cx: &mut Context<Workspace>) -> Option<impl IntoElement> {
    let found = &workspace.ui.iris.completions;
    if found.is_empty() {
        return None;
    }
    let rows = found.iter().enumerate().map(|(index, completion)| {
        let text = completion.text.clone();
        div()
            .id(("iris-completion", index))
            .px_2()
            .py_0p5()
            .flex()
            .gap_2()
            .rounded(px(5.0))
            .cursor_pointer()
            .hover(|style| style.bg(hsla(color::HOVER)))
            .on_click(cx.listener(move |this, _, window, cx| complete_with(this, &text, window, cx)))
            .child(div().flex_none().text_color(hsla(color::TEXT)).child(completion.text.trim_end().to_owned()))
            .child(div().min_w_0().truncate().text_color(hsla(color::MUTED)).child(completion.about.clone()))
            .test_support()
    });
    Some(div().mx_2().p_1().flex().flex_col().rounded(px(8.0)).bg(hsla(color::RAISED)).border_1().border_color(hsla(color::HAIRLINE)).text_size(px(size::SMALL_TEXT)).children(rows.collect::<Vec<_>>()))
}

/// Puts `text` in the message field, the cursor after it.
fn fill_field(workspace: &mut Workspace, text: &str, window: &mut Window, cx: &mut Context<Workspace>) {
    if let Some((input, _)) = &workspace.ui.iris.ask {
        let text = text.to_owned();
        input.update(cx, |input, cx| input.set_value(text, window, cx));
    }
}

/// Takes a completion into the field.
fn complete_with(workspace: &mut Workspace, text: &str, window: &mut Window, cx: &mut Context<Workspace>) {
    fill_field(workspace, text, window, cx);
    workspace.ui.iris.completions.clear();
    cx.notify();
}

/// Tab and the up and down arrows in the message field, caught before the field's own
/// handling of them. Tab takes the first completion. The arrows go back through what was
/// typed, but only while the field is empty or still holds what an arrow put there, so they
/// keep moving the cursor in a message being written. Anything else is left to the field.
fn field_key(workspace: &mut Workspace, key: &str, window: &mut Window, cx: &mut Context<Workspace>) {
    let Some(typed) = workspace.ui.iris.ask.as_ref().map(|(input, _)| input.read(cx).value().to_string()) else { return };
    let recalling = typed.trim().is_empty() || workspace.ui.iris.recalled.as_deref() == Some(typed.as_str());
    let next = match key {
        "tab" => match workspace.ui.iris.completions.first() {
            Some(first) => {
                let text = first.text.clone();
                complete_with(workspace, &text, window, cx);
                cx.stop_propagation();
                return;
            }
            None => return,
        },
        "up" if recalling => workspace.ui.iris.recall.older(&typed),
        "down" if recalling => workspace.ui.iris.recall.newer(),
        _ => return,
    };
    if let Some(text) = next {
        fill_field(workspace, &text, window, cx);
        workspace.ui.iris.recalled = Some(text);
    }
    workspace.ui.iris.completions.clear();
    cx.stop_propagation();
    cx.notify();
}

/// The rules, each with its switch, under the gear.
fn rules(workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement {
    let options = workspace.ui.settings.assistant.clone();
    let badge = workspace.ui.settings.assistant_badge;
    div()
        .mx_2()
        .p_2p5()
        .flex()
        .flex_col()
        .gap_1p5()
        .rounded(px(8.0))
        .bg(hsla(color::INSET))
        .border_1()
        .border_color(hsla(color::HAIRLINE))
        .child(div().font_weight(gpui_kit::FontWeight::MEDIUM).child("What I look for"))
        .children(Check::ALL.into_iter().enumerate().map(|(index, check)| {
            Checkbox::new(("iris-rule", index)).label(check.name()).checked(options.runs(check)).tooltip(check.about()).on_click(cx.listener(move |this, on: &bool, _, cx| {
                this.ui.settings.assistant.set(check, *on);
                let saved = this.ui.settings.assistant.clone();
                crate::app_settings::update(move |settings| settings.assistant = saved.clone());
                cx.notify();
            }))
        }))
        .child(div().pt_1().child(Checkbox::new("iris-badge").label("Show my face on the photo with a count").checked(badge).on_click(cx.listener(|this, on: &bool, _, cx| {
            this.ui.settings.assistant_badge = *on;
            let on = *on;
            crate::app_settings::update(move |settings| settings.assistant_badge = on);
            cx.notify();
        }))))
}

pub fn render(workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let found = open_suggestions(workspace);
    let count = found.len();
    let settings_open = workspace.ui.iris.settings_open;
    let gear = Button::new("iris-settings").ghost().small().icon(Icon::new(IconName::Settings)).selected(settings_open).tooltip("Choose what Iris looks for").on_click(cx.listener(|this, _, _, cx| {
        this.ui.iris.settings_open = !this.ui.iris.settings_open;
        cx.notify();
    }));
    let header = div()
        .px_2p5()
        .pt_2()
        .flex()
        .items_center()
        .gap_2p5()
        // A click on the face gives the next tip, in the conversation.
        .child(div().id("iris-face").cursor_pointer().child(face("panel", 32.0, current_look(workspace))).on_click(cx.listener(|this, _, window, cx| {
            this.ui.iris.active_at = Some(Instant::now());
            this.ui.iris.blinks = this.ui.iris.blinks.wrapping_add(1);
            conversation::tip(this, window, cx);
            this.ui.iris.chat_chosen = Some(true);
        })).test_support())
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(div().font_weight(gpui_kit::FontWeight::SEMIBOLD).child(name(workspace)))
                .child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(greeting(workspace, count))),
        )
        .child(gear);
    let dismissed = workspace.state.photo().map_or(0, |photo| {
        let source = source_id(photo);
        workspace.ui.iris.dismissed.iter().filter(|(of, _)| *of == source).count()
    });
    let restore = (dismissed > 0).then(|| {
        div().px_2().child(Button::new("iris-restore").ghost().small().label(format!("Bring back {dismissed} dismissed")).on_click(cx.listener(|this, _, _, cx| {
            if let Some(photo) = this.state.photo() {
                let source = source_id(photo);
                this.ui.iris.dismissed.retain(|(of, _)| *of != source);
            }
            cx.notify();
        })))
    });
    let fixable = found.iter().filter(|suggestion| suggestion.fix.is_some()).count();
    let fix_all_button = (fixable > 1).then(|| {
        div().px_2().flex().child(Button::new("iris-fix-all").outline().small().label(format!("Fix All {fixable}")).tooltip("Makes each fix in turn, checking the photo again after each. One Undo takes them all back.").on_click(cx.listener(|this, _, window, cx| {
            conversation::fix_everything(this, window, cx);
        })))
    });
    let cards = found.into_iter().enumerate().map(|(index, suggestion)| card(index, suggestion, workspace, cx).into_any_element()).collect::<Vec<_>>();
    // The two tabs. Until one is chosen, Suggestions shows when there is something to suggest.
    let chatting = workspace.ui.iris.chat_chosen.unwrap_or(count == 0);
    let tab = |id: &'static str, label: String, chat: bool, cx: &mut Context<Workspace>| {
        Button::new(id).ghost().small().label(label).selected(chatting == chat).on_click(cx.listener(move |this, _, _, cx| {
            this.ui.iris.chat_chosen = Some(chat);
            cx.notify();
        }))
    };
    let tabs = div().px_2().flex().gap_1().child(tab("iris-tab-chat", "Chat".to_owned(), true, cx)).child(tab("iris-tab-suggestions", if count == 0 { "Suggestions".to_owned() } else { format!("Suggestions {count}") }, false, cx));
    let list = div().id("iris-list").flex_1().min_h_0().overflow_y_scroll().flex().flex_col().gap_2().pb_3().when(settings_open, |list| list.child(rules(workspace, cx)));
    let list = if chatting {
        list.track_scroll(&workspace.ui.iris.chat_scroll).children(conversation(workspace, cx))
    } else {
        let empty = (cards.is_empty() && workspace.state.photo().is_some() && current(workspace).is_some()).then(|| div().px_3().text_color(hsla(color::MUTED)).child("Nothing to fix. Nothing is badly clipped, too dark, too bright, too harsh or off-color as far as my rules can tell."));
        list.children(fix_all_button).children(cards).children(empty).children(restore)
    };
    // The message field stays at the foot of the panel under both tabs; Return sends.
    let field = workspace.ui.iris.ask.as_ref().map(|(input, _)| {
        div()
            .flex_none()
            .mx_2()
            .mb_2()
            .h(px(32.0))
            .px_2()
            .flex()
            .items_center()
            .rounded(px(8.0))
            .bg(hsla(color::INSET))
            .border_1()
            .border_color(hsla(color::HAIRLINE))
            .capture_action(cx.listener(|this, _: &gpui_kit::component::input::IndentInline, window, cx| field_key(this, "tab", window, cx)))
            .capture_action(cx.listener(|this, _: &gpui_kit::component::input::MoveUp, window, cx| field_key(this, "up", window, cx)))
            .capture_action(cx.listener(|this, _: &gpui_kit::component::input::MoveDown, window, cx| field_key(this, "down", window, cx)))
            .child(div().flex_1().min_w_0().child(Input::new(input).appearance(false).small()))
    });
    let buttons = if chatting { recipe_buttons(workspace, cx).map(IntoElement::into_any_element) } else { None };
    let completed = completions(workspace, cx).map(IntoElement::into_any_element);
    let body = div().size_full().flex().flex_col().gap_2().child(header).child(tabs).children(buttons).child(list).children(completed).children(field);
    panel(PanelId::Side, &[], None, body, workspace, cx)
}

/// Iris in a corner of the photo: its face with a count while it has something to suggest. A
/// click opens its suggestions beside the rail.
pub fn badge(workspace: &Workspace, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    if !is_on(workspace) || !workspace.ui.settings.assistant_badge {
        return None;
    }
    // With her panel already open beside the rail, the badge would only repeat it.
    let panel_open = workspace.ui.side_panel() == LeftPanel::Iris && workspace.ui.showing_panels.contains(&PanelId::Side) && !workspace.ui.hidden_panels.contains(&PanelId::Side);
    if panel_open {
        return None;
    }
    let count = open_suggestions(workspace).len();
    if count == 0 {
        return None;
    }
    let iris_name = name(workspace);
    let label = if count == 1 { format!("{iris_name} noticed 1 thing") } else { format!("{iris_name} noticed {count} things") };
    Some(
        div()
            .id("iris-badge")
            .absolute()
            .right(px(12.0))
            .bottom(px(12.0))
            .flex()
            .items_center()
            .gap_1p5()
            .pl_1()
            .pr_2p5()
            .py_1()
            .rounded_full()
            .bg(hsla(color::POPOVER).opacity(0.92))
            .border_1()
            .border_color(hsla(color::HAIRLINE))
            .shadow_md()
            .cursor_pointer()
            .hover(|style| style.border_color(hsla(color::ACCENT)))
            .on_mouse_down(gpui_kit::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(|this, _, _, cx| {
                this.show_side(LeftPanel::Iris);
                this.ui.iris.chat_chosen = Some(false);
                cx.notify();
            }))
            .child(face("badge", 22.0, current_look(workspace)))
            .child(div().text_size(px(size::SMALL_TEXT)).child(label))
            .test_support()
            .into_any_element(),
    )
}

/// What the headless UI tests read of the conversation.
#[cfg(test)]
pub mod tests_support {
    use crate::workspace::Workspace;

    /// What Iris has said since the last message from the person, bubble by bubble.
    pub fn answer(workspace: &Workspace) -> Vec<String> {
        let messages = &workspace.ui.iris.messages;
        let from = messages.iter().rposition(|message| message.from_person).map_or(0, |index| index + 1);
        messages[from..].iter().map(|message| message.text.clone()).collect()
    }

    /// The short replies offered under the newest message.
    pub fn replies(workspace: &Workspace) -> Vec<String> {
        workspace.ui.iris.messages.last().map(|message| message.replies.clone()).unwrap_or_default()
    }

    /// The recipes Iris has been taught, by name.
    pub fn taught(workspace: &Workspace) -> Vec<String> {
        workspace.ui.iris.memory.taught.iter().map(|recipe| recipe.name.clone()).collect()
    }

    /// The recipes Develop comes with.
    pub fn built_in_recipes() -> Vec<::iris::recipes::Recipe> {
        super::conversation::built_in_recipes()
    }

    /// The folder Iris's recipes are kept in during this test.
    pub fn folder() -> std::path::PathBuf {
        super::shared_folder().expect("tests have a folder of their own")
    }

    /// The text in the message field.
    pub fn field(workspace: &Workspace, cx: &gpui_kit::App) -> String {
        workspace.ui.iris.ask.as_ref().map(|(input, _)| input.read(cx).value().to_string()).unwrap_or_default()
    }

    /// The message field, to type into.
    pub fn field_state(workspace: &Workspace) -> Option<gpui_kit::Entity<gpui_kit::component::input::InputState>> {
        workspace.ui.iris.ask.as_ref().map(|(input, _)| input.clone())
    }

    /// The completions offered under the field, as their text.
    pub fn completions(workspace: &Workspace) -> Vec<String> {
        workspace.ui.iris.completions.iter().map(|completion| completion.text.clone()).collect()
    }

    /// The key a recipe has, if any.
    pub fn recipe_key(workspace: &Workspace, name: &str) -> Option<String> {
        workspace.ui.iris.memory.recipe(name).and_then(|recipe| recipe.key)
    }

    /// How many messages were brought back from the last run, and how many there are.
    pub fn restored(workspace: &Workspace) -> (usize, usize) {
        (workspace.ui.iris.restored, workspace.ui.iris.messages.len())
    }

    /// Iris as she would start now, from what she keeps between runs.
    pub fn restart(workspace: &mut Workspace) {
        let field = workspace.ui.iris.ask.take();
        workspace.ui.iris = super::IrisUi::new(&workspace.ui.settings);
        // The field itself is the window's, so the same one carries on.
        workspace.ui.iris.ask = field;
    }
}

#[cfg(test)]
mod tests {
    use super::conversation::{asks_how, help_answer};
    use super::*;

    #[test]
    fn how_to_questions_are_answered_from_the_shared_help_with_develops_keys() {
        assert!(asks_how("How do I crop a photo?") && asks_how("where is the white balance") && !asks_how("crop it square"));
        let answer = help_answer("how do I crop and straighten a photo").expect("Develop's crop topic answers it");
        assert!(answer.text.starts_with("Crop and straighten a photo"), "{}", answer.text);
        // Develop's own shortcut is filled in, and the answer offers to do it here.
        assert!(answer.text.contains(&crate::keymap::key_for("crop-tool").unwrap()), "{}", answer.text);
        assert_eq!(answer.command.as_deref(), Some("crop-tool"));
        // A question about another app is answered as being about that app, with no button.
        let elsewhere = help_answer("how do I animate a canvas design").expect("Canvas's topic answers it");
        assert!(elsewhere.text.contains("In Omina Edit"), "{}", elsewhere.text);
        assert_eq!(elsewhere.command, None);
        assert!(help_answer("bake a cake").is_none());
    }

    #[test]
    fn every_develop_topic_names_real_develop_commands_and_keys() {
        for topic in ::iris::knowledge::topics().iter().filter(|topic| topic.is_for(::iris::App::Develop)) {
            if let Some(command) = &topic.command {
                assert!(crate::commands::action_for(command).is_some(), "{}: {command} is not a Develop command", topic.title);
            }
            let mut rest = topic.body.as_str();
            while let Some(start) = rest.find("{key:") {
                let end = rest[start..].find('}').expect("closed");
                let id = &rest[start + 5..start + end];
                assert!(crate::keymap::key_for(id).is_some(), "{}: {id} has no key in Develop", topic.title);
                rest = &rest[start + end..];
            }
        }
        assert!(::iris::knowledge::tips(::iris::App::Develop).count() >= 5);
    }

    #[test]
    fn the_shared_name_starts_from_develops_own_unless_it_was_tween() {
        let folder = shared_folder().unwrap();
        let _ = std::fs::remove_dir_all(&folder);
        let settings = AppSettings { assistant_name: "Tween".into(), ..AppSettings::default() };
        assert_eq!(load_shared(&settings).shown_name(), "Iris");
        // Saved for the other apps, so the next load reads it back.
        assert_eq!(::iris::settings::load(&folder).unwrap().shown_name(), "Iris");
        let _ = std::fs::remove_dir_all(&folder);
        let settings = AppSettings { assistant_name: "Pixel".into(), ..AppSettings::default() };
        assert_eq!(load_shared(&settings).shown_name(), "Pixel");
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn measuring_finds_problems_in_an_overexposed_photo_and_fix_all_settles_them() {
        let mut bright = develop_core::Bitmap::new(240, 160);
        for x in 0..240 {
            for y in 0..160 {
                let v = (200 + (x + y) / 8).min(255) as u8;
                bright.set_pixel(x, y, [v, v, v, 255]);
            }
        }
        let source = develop_core::Source::new("Bright.jpg", Vec::new(), bright);
        let photo = develop_core::Photo::new(source);
        let pyramid = crate::preview::tests_support::pyramid(&photo);
        let (measured, neutral) = measure(&pyramid, &photo.edit);
        let found = suggestions(&measured, neutral, &photo.edit, &Options::default());
        assert!(found.iter().any(|suggestion| suggestion.check == Check::Overexposed), "{found:?}");
        // Each fix, made and measured again, leaves the photo no longer overexposed.
        let mut edit = photo.edit.clone();
        for suggestion in &found {
            if let Some(fix) = &suggestion.fix {
                edit.develop = fix.apply_to(&edit.develop);
            }
        }
        let (after, _) = measure(&pyramid, &edit);
        assert!(after.median < measured.median, "{} → {}", measured.median, after.median);
    }
}

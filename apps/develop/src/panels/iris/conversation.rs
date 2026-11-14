//! Iris's side of the conversation: what happens to a message once it is sent.
//!
//! Every message comes through the front door Iris has in all three apps (`iris::front`),
//! which splits it into steps, runs recipes, answers sums and resolves "again". Requests of
//! different kinds joined by "and" are first put apart as steps (`iris::script::joined`).
//! Each step is then read here, the first reader that understands it winning:
//!
//! 1. Iris's own readers about the photo (`develop_core::chat`): edits, looks, the crop,
//!    exporting, the other open photos, questions;
//! 2. one of Develop's menu commands asked for by name ("zoom in", "open settings");
//! 3. those two again with mistyped words put right, saying how the message was read;
//! 4. the help every app shares, as typed and then with its words put right.
//!
//! A step nothing understands is said to be not followed, with the nearest help topics to
//! pick from. Nothing is claimed that wasn't done: each change is checked against the photo
//! before the reply says it was made.

use std::collections::HashSet;
use std::time::Instant;

use develop_core::assistant::{Check, Fix, suggestions};
use develop_core::chat::{self, Action, ExportAsk, NOT_FOLLOWED, Reply, Scene, SizeAsk, Where};
use develop_core::crop::ASPECTS;
use develop_core::develop::{Develop, DevelopSetting};
use develop_core::image_io::ExportFormat;
use develop_core::presets::{Preset, PresetParts};
use gpui_kit::{Context, Window};
use ::iris::calc::Units;
use ::iris::followup::Followup;
use ::iris::front::{self, Turn};
use ::iris::recipes::{self, Recipe, Trigger};

use super::{Message, Mood, batch, current, feel, folder, handoff, measure, name, open_suggestions, show_newest};
use crate::app_settings::{ExportChoices, ExportSize};
use crate::commands::{NAMED, Named};
use crate::features::{Feature, GROUPS};
use crate::preview::source_id;
use crate::state::Tool;
use crate::workspace::{LeftPanel, Workspace};

/// The most messages kept; the oldest are dropped past this.
const MOST_MESSAGES: usize = 200;
/// The most fixes Fix All makes in a row, measuring again after each.
const MOST_FIXES: usize = 6;
/// How many shared help answers a how-to question lists (the best in full, the rest by name).
const MOST_ANSWERS: usize = 3;
/// How many help topics are offered when a message isn't followed.
const MOST_NEAR: usize = 3;
/// What a message starts with to ask for a help topic by name ("Help: Fix a color cast").
const HELP_PREFIX: &str = "help:";

/// What came of one step of a message.
#[derive(Clone, Debug, Default)]
pub struct Outcome {
    /// Whether a photo was changed.
    pub changed: bool,
    /// Whether the step asked for something that couldn't be done, or wasn't understood.
    pub failed: bool,
    /// Whether what the step started is still going (a dialog is open, a question was asked),
    /// so the steps after it can't simply carry on.
    pub waits: bool,
    /// Whether the change was made to several photos at once (see `batch`).
    pub batch: bool,
    /// Short replies worth offering under the answer.
    pub replies: Vec<String>,
}

impl Outcome {
    /// A step that changed the photo, with the replies that usually follow one.
    pub(super) fn changed(replies: &[&str]) -> Self {
        Self { changed: true, replies: replies.iter().map(|reply| (*reply).to_owned()).collect(), ..Self::default() }
    }

    pub(super) fn failed() -> Self {
        Self { failed: true, ..Self::default() }
    }

    pub(super) fn waiting() -> Self {
        Self { waits: true, ..Self::default() }
    }
}

/// The last slider change Iris made, which "more" and "less" build on.
#[derive(Clone, Debug)]
pub struct LastAdjust {
    /// The open photo it was made to.
    session: u64,
    /// The Develop settings before and after it.
    before: Develop,
    after: Develop,
    /// The sliders it moved.
    settings: Vec<DevelopSetting>,
    /// What it was called.
    label: String,
}

/// The last time Iris tightened or loosened the crop, which "more" and "less" build on.
#[derive(Clone, Debug)]
pub struct LastTighten {
    /// The open photo it was made to.
    session: u64,
    /// The share cropped off each way (negative for cropped back out).
    share: f32,
    /// The crop as it left it.
    after: develop_core::crop::Crop,
}

/// Forgets the change "more" and "less" would build on: the last thing done wasn't one of those.
pub(super) fn forget_tunable(workspace: &mut Workspace) {
    workspace.ui.iris.last_adjust = None;
    workspace.ui.iris.last_crop = None;
}

/// The recipes Develop comes with: each is only messages Iris already understands, said in a
/// row. They can't be replaced or forgotten.
pub fn built_in_recipes() -> Vec<Recipe> {
    let recipe = |name: &str, steps: &[&str]| Recipe::new(name, steps.iter().copied());
    vec![
        recipe("portrait finish", &["softer skin tones", "texture -10", "add a subtle vignette", "sharpen a bit"]),
        recipe("landscape finish", &["highlights -30, shadows +25", "dehaze a bit", "make the sky bluer", "greener grass", "sharpen"]),
        recipe("mono finish", &["dramatic black and white", "add a little grain"]),
        recipe("fresh start", &["reset", "auto"]),
        recipe("web export", &["export as jpeg at 80% quality, 2048 pixels long edge"]),
        recipe("tidy frame", &["straighten it", "crop a bit off the edges"]),
        // A recipe that takes a value: "proof Warm and Soft".
        recipe("proof {name}", &["save a snapshot called {name}", "export as jpeg at 70% quality, 1200 pixels long edge"]),
    ]
}

/// What Iris remembers between messages, with the recipes taught so far read from her shared
/// folder.
pub fn remembered() -> front::Memory {
    let taught = super::shared_folder().map(|folder| recipes::load(&folder, ::iris::App::Develop)).unwrap_or_default();
    front::Memory::new(taught, built_in_recipes())
}

/// Saves the taught recipes for next time, saying so if they couldn't be written (they then
/// last only until Develop quits), and binds their keys anew.
pub(super) fn save_recipes(workspace: &mut Workspace, cx: &mut gpui_kit::App) {
    if let Some(folder) = super::shared_folder()
        && let Err(error) = recipes::save(&folder, ::iris::App::Develop, &workspace.ui.iris.memory.taught)
    {
        eprintln!("could not save Iris's recipes: {error}");
    }
    bind_recipe_keys(workspace, cx);
}

/// Binds every recipe's key as a real shortcut (and lets go of keys that were taken away), and
/// tells Iris which keys Develop's own commands use, so no recipe is given one of them.
pub fn bind_recipe_keys(workspace: &mut Workspace, cx: &mut gpui_kit::App) {
    crate::keymap::use_recipe_keys(workspace.ui.iris.memory.keyed());
    crate::workspace::apply_keys(&workspace.ui.settings, cx);
    workspace.ui.iris.memory.app_keys = crate::keymap::app_keys();
}

/// Keeps the conversation and what was typed in Iris's shared folder, for the next run.
pub(super) fn keep_conversation(workspace: &Workspace) {
    let Some(folder) = super::shared_folder() else { return };
    let iris = &workspace.ui.iris;
    let lines = iris.messages.iter().filter(|message| !message.text.trim().is_empty()).map(|message| ::iris::conversation::Line { from_person: message.from_person, text: message.text.clone() }).collect();
    let saved = ::iris::conversation::Saved { lines, typed: iris.recall.entries().to_vec() };
    if let Err(error) = ::iris::conversation::save(&folder, ::iris::App::Develop, &saved) {
        eprintln!("could not keep Iris's conversation: {error}");
    }
}

/// Iris's own commands, which a recipe can't be named after.
pub(super) fn reserved() -> Vec<&'static str> {
    chat::COMMANDS.iter().map(|(command, _)| *command).collect()
}

/// Adds words from Iris to the conversation.
fn say(workspace: &mut Workspace, text: impl Into<String>) {
    workspace.ui.iris.messages.push(Message::from_iris(text));
}

/// Whether a message asks how to do something, so the shared help is looked in first ("how do
/// I crop?" is about Develop, not a request to crop).
pub(super) fn asks_how(message: &str) -> bool {
    let lower = message.trim().to_lowercase();
    ["how do", "how can", "how to", "how would", "how should", "where is", "where are", "where do", "where can", "what does", "what is a", "what is the", "can i", "is there a way", "explain", "help me", "teach me"]
        .iter()
        .any(|start| lower.starts_with(start))
}

/// Iris's answer from the help every app shares: the best topic in full (with its command, when
/// it is something Develop does), and the names of others that fit. `None` when nothing fits.
pub(super) fn help_answer(message: &str) -> Option<Message> {
    use ::iris::knowledge::{find, topics};
    let found = find(message, MOST_ANSWERS);
    let (&best, others) = found.split_first()?;
    let key_for = |id: &str| crate::keymap::key_for(id);
    let answer = topics()[best].answer_in(::iris::App::Develop, &key_for);
    let mut text = format!("{}\n{}", answer.title, answer.body);
    let also: Vec<String> = others.iter().map(|index| topics()[*index].title.clone()).collect();
    if !also.is_empty() {
        text.push_str(&format!("\nYou can also ask about: {}.", also.join(", ")));
    }
    // Only commands Develop really has get a button.
    let command = answer.command.filter(|id| crate::commands::action_for(id).is_some());
    Some(Message { command, ..Message::from_iris(text) })
}

/// The next of Develop's tips from the shared help, with its command.
fn next_tip(workspace: &mut Workspace) -> Message {
    let tips: Vec<&::iris::knowledge::Topic> = ::iris::knowledge::tips(::iris::App::Develop).collect();
    if tips.is_empty() {
        return Message::from_iris("I have no tips for Omina Develop yet.");
    }
    let topic = tips[workspace.ui.iris.tips % tips.len()];
    workspace.ui.iris.tips += 1;
    let key_for = |id: &str| crate::keymap::key_for(id);
    let answer = topic.answer_in(::iris::App::Develop, &key_for);
    let command = answer.command.filter(|id| crate::commands::action_for(id).is_some());
    Message { command, ..Message::from_iris(format!("Tip: {}\n{}", answer.title, answer.body)) }
}

/// Gives the next tip in the conversation (a click on Iris's face).
pub(super) fn tip(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let tip = next_tip(workspace);
    workspace.ui.iris.messages.push(tip);
    show_newest(window, cx);
}

/// Makes Iris's fixes one after another, measuring the photo again after each (a fix for one
/// thing can settle another), until nothing is left to fix. They are all one undo step.
/// Returns the fixes made.
fn fix_all(workspace: &mut Workspace) -> Vec<String> {
    let mut made: Vec<String> = Vec::new();
    let mut tried: HashSet<Check> = HashSet::new();
    for _ in 0..MOST_FIXES {
        let Some(photo) = workspace.state.photo() else { break };
        let Some(pyramid) = workspace.ui.preview.pyramid_of(photo) else { break };
        let source = source_id(photo);
        let (measured, neutral) = measure(&pyramid, &photo.edit);
        let found = suggestions(&measured, neutral, &photo.edit, &workspace.ui.settings.assistant);
        let next = found.into_iter().find(|suggestion| suggestion.fix.is_some() && !tried.contains(&suggestion.check) && !workspace.ui.iris.dismissed.contains(&(source, suggestion.check)));
        let Some(suggestion) = next else { break };
        tried.insert(suggestion.check);
        if let Some(fix) = &suggestion.fix {
            workspace.state.apply_fix(fix);
            made.push(format!("{} ({})", suggestion.title, fix.label));
        }
    }
    if !made.is_empty() {
        workspace.state.label = Some("Iris: Fix All".to_owned());
        feel(workspace, Mood::Happy);
    }
    made
}

/// Makes every fix and says which were made (the Fix All button, "fix it" and /fix).
pub(super) fn fix_everything(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) -> Outcome {
    let made = fix_all(workspace);
    let outcome = if made.is_empty() {
        say(workspace, "There's nothing for me to fix: nothing I measure is out of the ordinary. (Anything you dismissed stays as it is.)");
        Outcome::default()
    } else {
        forget_tunable(workspace);
        say(workspace, format!("Done. I fixed:\n• {}\nOne Undo takes them all back.", made.join("\n• ")));
        Outcome::changed(&["Undo that", "Is this ready?"])
    };
    show_newest(window, cx);
    outcome
}

/// Takes a message the person typed: adds it to the conversation with Iris's reply, and does
/// whatever the reply calls for (an edit, fixes, clearing the conversation, renaming).
pub fn send(workspace: &mut Workspace, message: &str, window: &mut Window, cx: &mut Context<Workspace>) {
    let message = message.trim();
    if message.is_empty() {
        return;
    }
    workspace.ui.iris.messages.push(Message { from_person: true, ..Message::from_iris(message) });
    workspace.ui.iris.chat_chosen = Some(true);
    workspace.ui.iris.active_at = Some(Instant::now());
    workspace.ui.iris.recall.push(message);
    workspace.ui.iris.recalled = None;
    workspace.ui.iris.completions.clear();
    // Whatever was changed before this message becomes a history step of its own, so this
    // message's changes are undone apart from it.
    workspace.state.record_history(false);
    let replies = answer(workspace, message, window, cx);
    // The replies go under the last thing Iris said, unless it brought its own.
    if let Some(last) = workspace.ui.iris.messages.last_mut().filter(|last| !last.from_person && last.replies.is_empty()) {
        last.replies = replies;
    }
    trim_and_keep(workspace);
    show_newest(window, cx);
}

/// Drops the oldest messages past the most kept, and keeps the conversation for next time.
fn trim_and_keep(workspace: &mut Workspace) {
    let iris = &mut workspace.ui.iris;
    if iris.messages.len() > MOST_MESSAGES {
        let excess = iris.messages.len() - MOST_MESSAGES;
        iris.messages.drain(..excess);
        iris.restored = iris.restored.saturating_sub(excess);
    }
    keep_conversation(workspace);
}

/// Runs the recipe called `name` as its key or its button does: as if its name had been typed,
/// with the result in the chat. Its changes are one Undo.
pub fn run_recipe(workspace: &mut Workspace, name: &str, window: &mut Window, cx: &mut Context<Workspace>) {
    let Some(recipe) = workspace.ui.iris.memory.recipe(name) else { return };
    workspace.ui.iris.messages.push(Message { from_person: true, ..Message::from_iris(recipe.called()) });
    workspace.ui.iris.active_at = Some(Instant::now());
    workspace.state.record_history(false);
    let reading = front::run(name, &mut workspace.ui.iris.memory, &Units::default(), &reserved());
    if reading.recipes_changed {
        save_recipes(workspace, cx);
    }
    let outcome = take_turns(workspace, &reading.turns, window, cx);
    if outcome.changed {
        workspace.state.label = Some(format!("Recipe: {}", recipe.called()));
    }
    if let Some(last) = workspace.ui.iris.messages.last_mut().filter(|last| !last.from_person && last.replies.is_empty()) {
        last.replies = outcome.replies;
    }
    trim_and_keep(workspace);
    show_newest(window, cx);
}

/// Runs the recipes set to run by themselves at `trigger` on the photo showing, each posting
/// what it did in the chat and each one Undo of its own. Nothing happens with no photo open,
/// with Iris turned off, or while a recipe run this way is itself exporting or saving. A
/// recipe that fails part way says so; what set it off goes ahead regardless.
pub fn run_trigger(workspace: &mut Workspace, trigger: Trigger, window: &mut Window, cx: &mut Context<Workspace>) {
    if workspace.state.photo().is_none() || !super::is_on(workspace) || workspace.ui.iris.in_trigger {
        return;
    }
    let names = workspace.ui.iris.memory.triggered(trigger);
    if names.is_empty() {
        return;
    }
    workspace.ui.iris.in_trigger = true;
    for name in names {
        let called = workspace.ui.iris.memory.recipe(&name).map_or_else(|| name.clone(), |recipe| recipe.called());
        say(workspace, format!("Running “{called}” by itself, {}:", trigger.phrase()));
        workspace.state.record_history(false);
        let reading = front::run(&name, &mut workspace.ui.iris.memory, &Units::default(), &reserved());
        let outcome = take_turns(workspace, &reading.turns, window, cx);
        if outcome.changed {
            workspace.state.label = Some(format!("Recipe: {called}"));
            workspace.state.record_history(false);
        }
        if outcome.failed && trigger == Trigger::BeforeExport {
            say(workspace, "The export goes ahead anyway.");
        }
    }
    workspace.ui.iris.in_trigger = false;
    trim_and_keep(workspace);
    show_newest(window, cx);
}

/// Answers one message, step by step. Returns the short replies worth offering after it.
fn answer(workspace: &mut Workspace, message: &str, window: &mut Window, cx: &mut Context<Workspace>) -> Vec<String> {
    // A folder batch at work stops when asked.
    if folder::asks_to_stop(message) && folder::stop(workspace) {
        say(workspace, "Stopping after the photo I'm working on. What's done so far stays done.");
        return Vec::new();
    }
    if let Some(plan) = workspace.ui.iris.awaiting_folder.take() {
        match batch::answer_to(message) {
            Some(true) => return folder::run(workspace, plan, window, cx).replies,
            Some(false) => {
                say(workspace, "Left as they are.");
                return Vec::new();
            }
            None => {}
        }
    }
    // A change to several photos that was waiting for a yes or no gets its answer. Anything
    // else lets the question drop and is read as usual.
    if let Some(plan) = workspace.ui.iris.awaiting.take() {
        match batch::answer_to(message) {
            Some(true) => return batch::carry_out(workspace, plan).replies,
            Some(false) => {
                say(workspace, "Left as they are.");
                return Vec::new();
            }
            None => {}
        }
    }
    if holds_stray_braces(&workspace.ui.iris.memory, message) {
        say(workspace, "Curly brackets mark where a value goes when a recipe is taught (“teach proof {name}: …”). A value given to a recipe can't hold them, so say it without.");
        return Vec::new();
    }
    let message = with_joined_requests_apart(workspace, message);
    let reading = front::read(&message, &mut workspace.ui.iris.memory, &Units::default(), &reserved());
    if reading.recipes_changed {
        save_recipes(workspace, cx);
    }
    take_turns(workspace, &reading.turns, window, cx).replies
}

/// Whether `message` holds curly brackets anywhere but in teaching a recipe. A recipe run with
/// a value holding its own "{name}" would never finish filling it in, so such a message is
/// refused before it reaches the front door.
pub(super) fn holds_stray_braces(memory: &front::Memory, message: &str) -> bool {
    let teaching = matches!(recipes::read(message, &memory.known()), Some(recipes::Said::Teach(_) | recipes::Said::TeachLast(_) | recipes::Said::Problem(_)));
    message.contains(['{', '}']) && !teaching
}

/// Which reader would take a piece of a message, judged without doing anything: two pieces of
/// one kind are a list for that reader; pieces of different kinds are separate requests.
#[derive(PartialEq)]
enum Kind {
    /// A slider edit, which the edit reader takes any number of at once.
    Edit,
    /// Something else Iris does to the photo or the app, by what it is.
    Act(std::mem::Discriminant<Action>),
    /// A menu command, by its id.
    Command(&'static str),
    /// A recipe, by its place among the known ones.
    Recipe(usize),
    /// A reply that refers back ("undo that", "more").
    Followup,
}

/// The kind of request `text` is, or `None` when nothing would act on it (a question, a help
/// topic, or words nothing understands).
fn kind_of(workspace: &Workspace, text: &str) -> Option<Kind> {
    if export_preset_said(text).is_some() {
        return Some(Kind::Command("export-preset"));
    }
    if let Some(recipes::Said::Run { index, .. }) = recipes::read(text, &workspace.ui.iris.memory.known()) {
        return Some(Kind::Recipe(index));
    }
    if ::iris::followup::read(text).is_some() {
        return Some(Kind::Followup);
    }
    match read(workspace, text) {
        Reply::Do(_, Action::Adjust(_)) => Some(Kind::Edit),
        Reply::Do(_, action) => Some(Kind::Act(std::mem::discriminant(&action))),
        Reply::Say(_) | Reply::Partly { .. } => None,
        Reply::Unknown => {
            use ::iris::commands::{Command, Found, find};
            let offered = offered_commands(workspace);
            let listed: Vec<Command> = offered.iter().map(|command| Command { name: command.name, keywords: command.keywords }).collect();
            match find(text, &listed) {
                Found::One(index) => Some(Kind::Command(offered[index].id)),
                Found::None | Found::Several(_) => None,
            }
        }
    }
}

/// `message` with requests of different kinds that an "and" joins put apart as steps ("warmer
/// and crop to square" becomes "warmer then crop to square"), so each goes to the reader that
/// understands it. A list one reader takes whole ("shadows +30 and a bit warmer"), a message
/// about recipes, and a command are left as typed.
fn with_joined_requests_apart(workspace: &Workspace, message: &str) -> String {
    // A message about recipes, or one running a recipe whole (whose value may hold an "and"
    // of its own: "proof Warm and Soft"), is the front door's to read as it is.
    if message.starts_with('/') || recipes::read(message, &workspace.ui.iris.memory.known()).is_some() {
        return message.to_owned();
    }
    let steps = ::iris::script::steps(message);
    let mut apart: Vec<String> = Vec::new();
    for step in &steps {
        // A folder batch lists what to do to each photo itself ("… and export").
        if matches!(read(workspace, step), Reply::Do(_, Action::Folder(_))) {
            apart.push(step.clone());
            continue;
        }
        // A name being given may hold an "and" of its own, which quotes keep whole.
        apart.extend(::iris::script::joined(&chat::quote_name(step), |piece| kind_of(workspace, piece)));
    }
    if apart.len() == steps.len() { message.to_owned() } else { apart.join(" then ") }
}

/// Carries out a message's turns in order. A turn that fails, or leaves something waiting for
/// an answer, ends the message there, and the turns not reached are named.
fn take_turns(workspace: &mut Workspace, turns: &[Turn], window: &mut Window, cx: &mut Context<Workspace>) -> Outcome {
    let mut whole = Outcome::default();
    for (index, turn) in turns.iter().enumerate() {
        let outcome = match turn {
            Turn::Say(text) => {
                say(workspace, text.clone());
                Outcome::default()
            }
            Turn::Ask(text) => ask(workspace, text, window, cx),
            Turn::Followup(followup) => follow_up(workspace, followup, window, cx),
        };
        if outcome.changed {
            workspace.ui.iris.memory.changed();
            // A change to one photo ends the chance to take a change to many back as one.
            if !outcome.batch {
                workspace.ui.iris.last_batch = None;
            }
        }
        whole.changed |= outcome.changed;
        whole.failed |= outcome.failed;
        whole.waits |= outcome.waits;
        whole.replies = outcome.replies;
        let left: Vec<String> = turns[index + 1..].iter().filter_map(turn_words).collect();
        if (outcome.failed || outcome.waits) && !left.is_empty() {
            let why = if outcome.failed { "That step didn't work, so I stopped there." } else { "I stopped there until that's settled." };
            say(workspace, format!("{why} Not run:\n• {}", left.join("\n• ")));
            break;
        }
    }
    whole
}

/// A turn as the words that asked for it, for naming the turns that were not run.
fn turn_words(turn: &Turn) -> Option<String> {
    match turn {
        Turn::Ask(text) => Some(text.clone()),
        Turn::Followup(Followup::More) => Some("more".to_owned()),
        Turn::Followup(Followup::Less) => Some("less".to_owned()),
        Turn::Followup(Followup::Undo) => Some("undo".to_owned()),
        Turn::Followup(Followup::Redo) => Some("redo".to_owned()),
        Turn::Followup(Followup::SameFor(name)) => Some(format!("same for {name}")),
        Turn::Followup(Followup::Again) | Turn::Say(_) => None,
    }
}

/// Iris's reading of `message` about `photo`, which is (or stands in for) open photo `index`.
/// Only the photo showing has measurements and suggestions; `found` is its suggestions.
pub(super) fn reply_about_photo(workspace: &Workspace, photo: &develop_core::Photo, index: usize, message: &str, found: &[develop_core::assistant::Suggestion]) -> Reply {
    let showing = index == workspace.state.active;
    let iris_name = name(workspace);
    let presets: Vec<String> = crate::panels::presets::all(workspace).into_iter().map(|preset| preset.name).collect();
    let names: Vec<String> = workspace.state.sessions.iter().map(|session| session.photo.name.clone()).collect();
    let scene = Scene {
        photo,
        measure: if showing { current(workspace).map(|(measured, _)| measured) } else { None },
        suggestions: if showing { found } else { &[] },
        name: &iris_name,
        photo_names: &names,
        active: index,
        presets: &presets,
        unsaved: workspace.state.is_unsaved(index),
    };
    chat::reply(message, &scene)
}

/// Iris's own reading of a message about the photo showing. Without a photo only the commands
/// about Iris herself make sense.
fn read(workspace: &Workspace, message: &str) -> Reply {
    let found = open_suggestions(workspace);
    match workspace.state.photo().map(|photo| reply_about_photo(workspace, photo, workspace.state.active, message, &found)) {
        Some(reply) => reply,
        None if message.starts_with("/clear") => Reply::Do(String::new(), Action::Clear),
        None if message.starts_with("/name") => Reply::Do(String::new(), Action::Rename(message.trim_start_matches("/name").trim().to_owned())),
        None => Reply::Unknown,
    }
}

/// The help topic a message asks for by name ("Help: Fix a color cast"), if it does.
fn help_topic(text: &str) -> Option<&str> {
    let start = text.get(..HELP_PREFIX.len())?;
    start.eq_ignore_ascii_case(HELP_PREFIX).then(|| text[HELP_PREFIX.len()..].trim())
}

/// Answers one request, trying each reader in turn.
fn ask(workspace: &mut Workspace, text: &str, window: &mut Window, cx: &mut Context<Workspace>) -> Outcome {
    // A help topic asked for by name, as the replies under "I didn't follow that" do.
    if let Some(topic) = help_topic(text) {
        match help_answer(topic) {
            Some(answer) => workspace.ui.iris.messages.push(answer),
            None => say(workspace, format!("I have no help on “{topic}”. Ask in your own words, like “how do I crop?”.")),
        }
        return Outcome::default();
    }
    // "How do I…?" is answered from the shared help before anything is read as a request.
    if asks_how(text)
        && let Some(answer) = help_answer(text)
    {
        workspace.ui.iris.messages.push(answer);
        return Outcome::default();
    }
    if let Some(outcome) = acted_on(workspace, text, window, cx) {
        return outcome;
    }
    // Read once more with mistyped words put right, saying how the message was read. This
    // comes before the help, which finds an answer for most sentences: "make it brigther"
    // should be made brighter, not explained.
    let fixed = ::iris::spelling::correct(text, &vocabulary(workspace));
    let first = workspace.ui.iris.messages.len();
    let with_note = |workspace: &mut Workspace, fixed: &::iris::spelling::Correction| {
        if let Some(reply) = workspace.ui.iris.messages.get_mut(first) {
            reply.text = format!("{}\n{}", fixed.note(), reply.text);
        }
    };
    if let Some(fixed) = &fixed
        && let Some(outcome) = acted_on(workspace, &fixed.message, window, cx)
    {
        with_note(workspace, fixed);
        return outcome;
    }
    // A list followed only in part changes nothing, and the help shouldn't pick one word out
    // of it either.
    let partly = matches!(read(workspace, text), Reply::Partly { .. });
    if !partly && explained(workspace, text) {
        return Outcome::default();
    }
    if !partly
        && let Some(fixed) = &fixed
        && explained(workspace, &fixed.message)
    {
        with_note(workspace, fixed);
        return Outcome::default();
    }
    not_followed(workspace, text)
}

/// Every word Iris listens for in Develop, for putting a mistyped message right: the shared
/// readers' words, her own, and the names of the menu commands.
fn vocabulary(workspace: &Workspace) -> Vec<String> {
    let commands = NAMED.iter().flat_map(|command| ::iris::spelling::words(command.name).chain(::iris::spelling::words(command.keywords)).collect::<Vec<_>>());
    let recipes = workspace.ui.iris.memory.known().into_iter().flat_map(|recipe| ::iris::spelling::words(&recipe.name).collect::<Vec<_>>());
    let mut words: Vec<String> = front::vocabulary().into_iter().chain(chat::vocabulary()).chain(commands).chain(recipes).collect();
    words.sort_unstable();
    words.dedup();
    words
}

/// Does what a request asks for: Iris's own reading of it about the photo, or else the menu
/// command it names. `None` when it is neither.
fn acted_on(workspace: &mut Workspace, text: &str, window: &mut Window, cx: &mut Context<Workspace>) -> Option<Outcome> {
    if let Some(said) = export_preset_said(text) {
        return Some(export_preset(workspace, said, window, cx));
    }
    match read(workspace, text) {
        Reply::Unknown => {}
        // A list followed only in part changes nothing; it may still read whole once its
        // mistyped words are put right.
        Reply::Partly { .. } => return None,
        reply => return Some(deliver(workspace, reply, text, window, cx)),
    }
    // Without a photo, an unknown command has nothing to act on.
    if workspace.state.photo().is_none() && text.starts_with('/') {
        return None;
    }
    named_command(workspace, text, window, cx)
}

/// Words a request to do something starts with.
const REQUEST_VERBS: &[&str] = &[
    "make", "add", "remove", "set", "put", "give", "turn", "do", "change", "fix", "get", "apply", "use", "delete", "move", "create", "increase", "decrease", "more", "less", "blur", "erase", "replace", "clone", "paint", "draw", "please", "can", "could",
    "would", "let", "lets", "now", "just",
];

/// Whether a message asks for something to be done, rather than asking a question or naming a
/// topic.
fn asks_for_something(text: &str) -> bool {
    let first = text.split(|character: char| !character.is_alphanumeric()).find(|word| !word.is_empty()).map(str::to_lowercase);
    !text.trim_end().ends_with('?') && first.is_some_and(|first| REQUEST_VERBS.contains(&first.as_str()))
}

/// Answers a message nothing could act on from the help every app shares, or by saying which
/// app does what it asks. Returns whether it was answered.
fn explained(workspace: &mut Workspace, text: &str) -> bool {
    // Without a photo, a request is answered by asking for one; how-to questions were
    // answered from the help before this.
    if workspace.state.photo().is_none() {
        return false;
    }
    // Layers and timing belong to the other two apps. Asked for here, saying so is the answer;
    // asked about, the help says how the other app does it.
    let mentions_layers = text.to_lowercase().split(|character: char| !character.is_alphanumeric()).any(|word| matches!(word, "layer" | "layers" | "keyframe" | "keyframes" | "timeline"));
    if mentions_layers && !::iris::intent::asks(text) {
        say(
            workspace,
            "Omina Develop edits one photo at a time and has no layers or timeline: layers are Omina Edit's, and animation is Omina Motion's. Here I can change the photo's light and color, crop it, and adjust one part of it with a mask (ask me “how do I adjust just one part of a photo?”).",
        );
        return true;
    }
    // A request Iris can't carry out isn't answered with an article that shares a word with
    // it ("make it purple" is not about making motion smoother): the nearest topics are
    // offered under "I didn't follow that" instead. A question or a bare topic is answered.
    if !asks_for_something(text)
        && let Some(answer) = help_answer(text)
    {
        workspace.ui.iris.messages.push(answer);
        return true;
    }
    false
}

/// What Iris says when nothing understood a message: which part of a list wasn't followed, or
/// that she didn't follow it at all, with the nearest help topics to pick from.
fn not_followed(workspace: &mut Workspace, text: &str) -> Outcome {
    if workspace.state.photo().is_none() {
        let iris_name = name(workspace);
        say(workspace, format!("Open a photo and I'll look it over. I'm {iris_name}: I spot things like blown highlights, crushed shadows or a color cast, and fix them when you ask."));
        return Outcome::failed();
    }
    feel(workspace, Mood::Sad);
    if let Reply::Partly { followed, missed } = read(workspace, text) {
        let quoted = |parts: &[String]| parts.iter().map(|part| format!("“{part}”")).collect::<Vec<_>>().join(" and ");
        let message = Message {
            replies: vec![followed.join(", ")],
            ..Message::from_iris(format!("I followed {} but not {}, so I haven't changed anything. Say it without that part, or put it another way.", quoted(&followed), quoted(&missed)))
        };
        workspace.ui.iris.messages.push(message);
        return Outcome::failed();
    }
    let near: Vec<String> = ::iris::knowledge::closest(text, MOST_NEAR).into_iter().map(|index| format!("Help: {}", ::iris::knowledge::topics()[index].title)).collect();
    let replies = if near.is_empty() { vec!["What can you do?".to_owned(), "What's wrong?".to_owned()] } else { near };
    workspace.ui.iris.messages.push(Message { replies, ..Message::from_iris(NOT_FOLLOWED) });
    Outcome::failed()
}

/// The commands that can be asked for by name now: those whose feature is on.
fn offered_commands(workspace: &Workspace) -> Vec<&'static Named> {
    let settings = &workspace.ui.settings;
    NAMED
        .iter()
        .filter(|command| settings.command_on(command.id))
        .filter(|command| side_panel(command.id).is_none_or(|panel| settings.panel_on(panel)))
        .collect()
}

/// The side panel a "show …" command opens, for the commands that are panels.
fn side_panel(id: &str) -> Option<LeftPanel> {
    match id {
        "show-presets" => Some(LeftPanel::Presets),
        "show-snapshots" => Some(LeftPanel::Snapshots),
        "show-history" => Some(LeftPanel::History),
        _ => None,
    }
}

/// A command's name without the "…" its menu shows.
fn plain_name(command: &Named) -> &'static str {
    command.name.trim_end_matches('…')
}

/// Runs the menu command a message names. When several fit equally, they are offered instead.
fn named_command(workspace: &mut Workspace, text: &str, window: &mut Window, cx: &mut Context<Workspace>) -> Option<Outcome> {
    use ::iris::commands::{Command, Found, find};
    let offered = offered_commands(workspace);
    let listed: Vec<Command> = offered.iter().map(|command| Command { name: command.name, keywords: command.keywords }).collect();
    match find(text, &listed) {
        Found::None => None,
        Found::One(index) => Some(run_command(workspace, offered[index], text, window, cx)),
        Found::Several(indexes) => {
            let names: Vec<String> = indexes.iter().map(|index| plain_name(offered[*index]).to_owned()).collect();
            workspace.ui.iris.messages.push(Message { replies: names.clone(), ..Message::from_iris(format!("Which do you mean: {}?", names.join(", "))) });
            Some(Outcome::waiting())
        }
    }
}

/// Runs one menu command and says so, with its key if it has one.
fn run_command(workspace: &mut Workspace, command: &Named, text: &str, window: &mut Window, cx: &mut Context<Workspace>) -> Outcome {
    if command.needs_photo && workspace.state.photo().is_none() {
        say(workspace, format!("{} needs a photo. Open one first (⌘O).", plain_name(command)));
        return Outcome::failed();
    }
    let key = crate::keymap::key_for(command.id).map(|key| format!(" ({key})")).unwrap_or_default();
    if let Some(panel) = side_panel(command.id) {
        workspace.show_side(panel);
        say(workspace, format!("{} is showing beside the rail.", plain_name(command)));
        return Outcome::default();
    }
    // The clipping warning is a switch, so the words say which way to turn it.
    if command.id == "clipping" {
        let words: Vec<String> = text.to_lowercase().split_whitespace().map(str::to_owned).collect();
        let said = |list: &[&str]| words.iter().any(|word| list.contains(&word.as_str()));
        workspace.ui.show_clipping = if said(&["hide", "off", "stop"]) { false } else { said(&["show", "on"]) || !workspace.ui.show_clipping };
        let how = if workspace.ui.show_clipping { "on: parts clipped to white are painted red, and to black blue" } else { "off" };
        say(workspace, format!("The clipping warning is {how}{key}."));
        return Outcome::default();
    }
    let Some(action) = crate::commands::action_for(command.id) else { return Outcome::failed() };
    window.dispatch_action(action, cx);
    say(workspace, if command.opens { format!("Opening {}{key}.", plain_name(command)) } else { format!("{}{key}: done.", plain_name(command)) });
    if command.opens { Outcome::waiting() } else { Outcome::default() }
}

/// The feature a slider belongs to, when it sits on a card that can be turned off.
fn feature_of(setting: DevelopSetting) -> Option<Feature> {
    use DevelopSetting::*;
    match setting {
        MixerHue(_) | MixerSaturation(_) | MixerLuminance(_) => Some(Feature::ColorMixer),
        Grade(..) | GradeBlending | GradeBalance => Some(Feature::ColorGrading),
        SharpenAmount | SharpenRadius | NoiseLuminance | NoiseColor => Some(Feature::Detail),
        Vignette | VignetteMidpoint | VignetteFeather | Grain | GrainSize => Some(Feature::Effects),
        _ => None,
    }
}

/// A feature's name as Settings lists it.
fn feature_name(feature: Feature) -> &'static str {
    GROUPS.iter().flat_map(|(_, features)| features.iter()).find(|info| info.feature == feature).map_or("That feature", |info| info.name)
}

/// Iris's answer to a request for a feature that is turned off.
fn turned_off(feature: Feature) -> String {
    format!("{} is turned off in Settings ▸ Tools & Features. Turn it on there and ask me again.", feature_name(feature))
}

/// `fix` without the sliders whose card is turned off, and the features that were in the way.
pub(super) fn allowed_part(settings: &crate::app_settings::AppSettings, fix: &Fix) -> (Fix, Vec<Feature>) {
    let on = |setting: &DevelopSetting| feature_of(*setting).is_none_or(|feature| settings.feature_on(feature));
    let mut off: Vec<Feature> = fix.changes.iter().filter(|(setting, _)| !on(setting)).filter_map(|(setting, _)| feature_of(*setting)).collect();
    off.sort_unstable();
    off.dedup();
    let changes = fix.changes.iter().filter(|(setting, _)| on(setting)).copied().collect();
    (Fix { changes, ..fix.clone() }, off)
}

/// Makes a slider change and says what it set. The parts whose card is turned off are left
/// out and named; a change that would leave everything where it is says so instead.
fn adjust(workspace: &mut Workspace, text: String, fix: &Fix) -> Outcome {
    let Some(session) = workspace.state.session() else { return Outcome::failed() };
    let (id, before) = (session.id, session.photo.edit.develop.clone());
    let (allowed, off) = allowed_part(&workspace.ui.settings, fix);
    if allowed.changes.is_empty() && allowed.mask.is_none() {
        // Nothing to set: either its card is turned off, or the reply itself says why not.
        return match off.first() {
            Some(feature) => {
                say(workspace, turned_off(*feature));
                Outcome::failed()
            }
            None => {
                say(workspace, text);
                Outcome::default()
            }
        };
    }
    let after = allowed.apply_to(&before);
    if after == before && allowed.mask.is_none() {
        say(workspace, format!("That's where it already is ({}), so nothing changed.", chat::listed_changes(&allowed)));
        return Outcome::default();
    }
    workspace.state.apply_fix(&allowed);
    workspace.ui.hover_edit = None;
    let settings = allowed.changes.iter().map(|(setting, _)| *setting).collect();
    workspace.ui.iris.last_crop = None;
    workspace.ui.iris.last_adjust = Some(LastAdjust { session: id, before, after, settings, label: allowed.label.clone() });
    let text = if off.is_empty() {
        text
    } else {
        let names: Vec<&str> = off.iter().map(|feature| feature_name(*feature)).collect();
        format!("{}: {}. I left out the parts that belong to {}, which {} turned off in Settings ▸ Tools & Features.", allowed.label, chat::listed_changes(&allowed), names.join(" and "), if names.len() == 1 { "is" } else { "are" })
    };
    say(workspace, text);
    Outcome::changed(&["Undo that", "More", "Less"])
}

/// Whether the photo showing is the same as `before` (so what was asked for changed nothing).
fn unchanged(workspace: &Workspace, before: &Option<develop_core::Photo>) -> bool {
    workspace.state.photo() == before.as_ref()
}

/// Adds Iris's reply to the conversation and does what it calls for. `asked` is the request
/// the reply answers.
fn deliver(workspace: &mut Workspace, reply: Reply, asked: &str, window: &mut Window, cx: &mut Context<Workspace>) -> Outcome {
    let before = workspace.state.photo().cloned();
    let on = |workspace: &Workspace, feature: Feature| workspace.ui.settings.feature_on(feature);
    // A request for something whose feature is turned off is answered with where to turn it on.
    let needs = match &reply {
        Reply::Do(_, Action::ApplyPreset(_) | Action::SavePreset(_)) => Some(Feature::Presets),
        Reply::Do(_, Action::Compare(_)) => Some(Feature::BeforeAfter),
        Reply::Do(_, Action::Snapshot(_) | Action::RestoreSnapshot(_)) => Some(Feature::Snapshots),
        Reply::Do(_, Action::CopySettings | Action::PasteSettings { .. } | Action::Sync(_)) => Some(Feature::CopyPaste),
        Reply::Do(_, Action::Crop { .. } | Action::TurnCrop(_) | Action::ResetCrop | Action::Straighten(_) | Action::OpenCrop | Action::TightenCrop(_) | Action::Level) => Some(Feature::Crop),
        _ => None,
    };
    if let Some(feature) = needs.filter(|feature| !on(workspace, *feature)) {
        say(workspace, turned_off(feature));
        return Outcome::failed();
    }
    match reply {
        Reply::Say(text) => {
            // Not knowing a command makes Iris a little sad, for a moment.
            let unknown = text.starts_with("I don't know");
            if unknown {
                feel(workspace, Mood::Sad);
            }
            say(workspace, text);
            if unknown { Outcome::failed() } else { Outcome::default() }
        }
        // `acted_on` keeps these for the readers after Iris's own.
        Reply::Unknown | Reply::Partly { .. } => Outcome::failed(),
        Reply::Do(_, Action::Clear) => {
            workspace.ui.iris.messages.clear();
            workspace.ui.iris.restored = 0;
            if let Some(folder) = super::shared_folder()
                && let Err(error) = ::iris::conversation::clear(&folder, ::iris::App::Develop)
            {
                eprintln!("could not clear Iris's saved conversation: {error}");
            }
            Outcome::default()
        }
        Reply::Do(_, Action::Folder(job)) => folder::propose(workspace, job),
        Reply::Do(_, Action::SendTo(to)) => handoff::send_photo(workspace, to, window, cx),
        Reply::Do(text, Action::ShowSuggestions) => {
            workspace.ui.iris.messages.push(Message { opens_suggestions: true, replies: vec!["Fix it".to_owned()], ..Message::from_iris(text) });
            Outcome::default()
        }
        Reply::Do(text, Action::Offer(replies)) => {
            workspace.ui.iris.messages.push(Message { replies, ..Message::from_iris(text) });
            Outcome::default()
        }
        Reply::Do(text, Action::Help) => {
            say(workspace, format!("{text}\n\n{}", front::HELP));
            Outcome::default()
        }
        Reply::Do(_, Action::FixAll) => fix_everything(workspace, window, cx),
        Reply::Do(_, Action::Improve) => improve(workspace),
        Reply::Do(text, Action::OpenSettings) => {
            say(workspace, text);
            crate::panels::settings::open(workspace, window, cx);
            Outcome::waiting()
        }
        Reply::Do(_, Action::Rename(new_name)) => {
            let new_name = ::iris::settings::tidy_name(&new_name);
            let text = if new_name.is_empty() { "That name is empty, so I'll keep mine.".to_owned() } else { format!("{new_name} it is, in Omina Motion and Omina Edit too.") };
            if !new_name.is_empty() {
                super::change_shared(workspace, |shared| shared.name = new_name);
            }
            say(workspace, text);
            Outcome::default()
        }
        Reply::Do(_, Action::Tip) => {
            let tip = next_tip(workspace);
            workspace.ui.iris.messages.push(tip);
            Outcome::default()
        }
        Reply::Do(text, Action::Adjust(fix)) => adjust(workspace, text, &fix),
        Reply::Do(_, Action::Auto) => {
            workspace.auto();
            let text = format!("Auto set the light from the photo: {}.", light_set(workspace, &before));
            changed_or(workspace, &before, text, "Auto leaves the light where it is: it's already set as Auto would set it.", &["Undo that"])
        }
        Reply::Do(text, Action::Reset) => {
            workspace.state.reset_develop();
            changed_or(workspace, &before, text, "Every Develop setting is at zero already.", &["Undo that"])
        }
        Reply::Do(text, Action::ResetAll) => {
            workspace.state.reset_all();
            changed_or(workspace, &before, text, "The photo is already just as it came.", &["Undo that"])
        }
        Reply::Do(text, Action::ApplyPreset(preset_name)) => match crate::panels::presets::all(workspace).into_iter().find(|preset| preset.name == preset_name) {
            Some(preset) => {
                workspace.state.apply_preset(&preset);
                changed_or(workspace, &before, text, &format!("{preset_name} is what the photo already has."), &["Undo that"])
            }
            None => {
                say(workspace, format!("I can't find a preset called {preset_name}."));
                Outcome::failed()
            }
        },
        Reply::Do(_, Action::Undo) => undo(workspace),
        Reply::Do(_, Action::Redo) => redo(workspace),
        Reply::Do(text, Action::Rotate { clockwise, turns }) => {
            for _ in 0..turns {
                workspace.state.rotate(clockwise);
            }
            changed_or(workspace, &before, text, "The photo is turned as it was.", &["Undo that", "Again"])
        }
        Reply::Do(text, Action::Flip { horizontal }) => {
            workspace.state.flip(horizontal);
            changed_or(workspace, &before, text, "The photo is as it was.", &["Undo that"])
        }
        Reply::Do(text, Action::Compare(how)) => {
            workspace.state.compare = match how {
                None => crate::state::Compare::Off,
                Some(false) => crate::state::Compare::Before,
                Some(true) => crate::state::Compare::Split,
            };
            workspace.ui.iris.messages.push(Message { replies: if how.is_some() { vec!["Show after".to_owned()] } else { Vec::new() }, ..Message::from_iris(text) });
            Outcome::default()
        }
        Reply::Do(text, Action::Snapshot(snapshot_name)) => {
            workspace.state.add_snapshot(snapshot_name);
            say(workspace, text);
            Outcome::changed(&["Undo that"])
        }
        Reply::Do(text, Action::RestoreSnapshot(index)) => {
            workspace.state.apply_snapshot(index);
            changed_or(workspace, &before, text, "The edit is already exactly as that snapshot has it.", &["Undo that"])
        }
        Reply::Do(_, Action::SavePreset(preset_name)) => save_preset(workspace, preset_name, window, cx),
        Reply::Do(_, Action::CopySettings) => {
            workspace.state.copy_settings();
            let others = workspace.state.sessions.len() > 1;
            workspace.ui.iris.messages.push(Message {
                replies: if others { vec!["Sync to the others".to_owned()] } else { Vec::new() },
                ..Message::from_iris("Copied this photo's Develop settings and masks. Say “paste settings” on another photo to give it the same look; its crop stays its own unless you say “paste settings with the crop”.")
            });
            Outcome::default()
        }
        Reply::Do(_, Action::PasteSettings { look, crop }) => {
            if workspace.state.copied.is_none() {
                say(workspace, "There are no copied settings yet. Say “copy these settings” on the photo whose look you want first.");
                return Outcome::failed();
            }
            if crop && !on(workspace, Feature::Crop) {
                say(workspace, turned_off(Feature::Crop));
                return Outcome::failed();
            }
            if look {
                workspace.state.paste_settings();
            }
            let turned = if crop { workspace.state.paste_crop() } else { None };
            let text = match (look, crop) {
                (true, false) => "Pasted the copied settings and masks onto this photo; its crop stays its own.".to_owned(),
                (_, true) => {
                    let what = if look { "Pasted the copied settings, masks and crop" } else { "Pasted the copied crop" };
                    let how = if turned == Some(true) { " This photo stands the other way up, so the crop was turned a quarter to match." } else { "" };
                    let (width, height) = workspace.state.photo().map_or((0, 0), |photo| photo.edit.output_size(photo.source.size()));
                    format!("{what}. The crop was carried over in proportion to this photo's size and is now {width} × {height} pixels.{how}")
                }
                (false, false) => String::new(),
            };
            changed_or(workspace, &before, text, "This photo already has exactly what was copied.", &["Undo that"])
        }
        Reply::Do(_, Action::Crop { aspect, upright }) => {
            workspace.state.shape_crop(aspect, upright);
            let text = crop_words(workspace, &format!("Cropped to {}", ASPECTS[aspect.min(ASPECTS.len() - 1)].0));
            changed_or(workspace, &before, text, "The crop already has that shape.", &["Undo that", "Swap the crop"])
        }
        Reply::Do(_, Action::TurnCrop(upright)) => {
            workspace.state.turn_crop(upright);
            let text = crop_words(workspace, "Turned the crop");
            changed_or(workspace, &before, text, "The crop already lies that way.", &["Undo that"])
        }
        Reply::Do(text, Action::ResetCrop) => {
            workspace.state.reset_crop();
            changed_or(workspace, &before, text, "The photo isn't cropped or straightened.", &["Undo that"])
        }
        Reply::Do(_, Action::Straighten(degrees)) => {
            workspace.state.set_angle(degrees);
            let angle = workspace.state.edit_settings().map_or(0.0, |edit| edit.crop.angle);
            let text = format!("Straighten angle {angle:+.2}° (a positive angle turns it clockwise).");
            changed_or(workspace, &before, text, &format!("The straighten angle is already {angle:+.2}°."), &["Undo that"])
        }
        Reply::Do(text, Action::OpenCrop) => {
            workspace.state.set_tool(Tool::Crop);
            say(workspace, text);
            Outcome::default()
        }
        Reply::Do(_, Action::TightenCrop(share)) => tighten(workspace, share, &before),
        Reply::Do(_, Action::Level) => level(workspace),
        Reply::Do(_, Action::Go(place)) => go(workspace, place),
        Reply::Do(_, Action::Sync(parts)) => batch::propose(workspace, batch::What::Sync(parts), asked),
        Reply::Do(_, Action::EachPhoto(each)) => batch::propose(workspace, batch::What::Each(each), asked),
        Reply::Do(_, Action::Export(asked)) => export(workspace, asked, window, cx),
    }
}

/// Says `done` if the photo showing differs from `before`, with `replies` to offer; otherwise
/// says `same`, so nothing is claimed that didn't happen.
fn changed_or(workspace: &mut Workspace, before: &Option<develop_core::Photo>, done: String, same: &str, replies: &[&str]) -> Outcome {
    if unchanged(workspace, before) {
        say(workspace, same);
        return Outcome::default();
    }
    // "More" and "less" are about a slider change, which this wasn't.
    forget_tunable(workspace);
    say(workspace, done);
    Outcome::changed(replies)
}

/// `lead` with the crop's shape and size as it is now: "Cropped to 4 × 5, upright: 800 × 1000
/// pixels."
fn crop_words(workspace: &Workspace, lead: &str) -> String {
    let Some(photo) = workspace.state.photo() else { return format!("{lead}.") };
    let (width, height) = photo.edit.output_size(photo.source.size());
    let lies = match width.cmp(&height) {
        std::cmp::Ordering::Less => ", upright",
        std::cmp::Ordering::Greater => ", on its side",
        std::cmp::Ordering::Equal => "",
    };
    format!("{lead}{lies}: {width} × {height} pixels. Press R to move the crop on the photo.")
}

/// Shows another open photo and says which. Returns whether a different photo is showing now.
fn show(workspace: &mut Workspace, place: Where) -> bool {
    let count = workspace.state.sessions.len();
    let was = workspace.state.active;
    match place {
        Where::Next => workspace.state.activate_next(true),
        Where::Previous => workspace.state.activate_next(false),
        Where::Index(index) => workspace.state.activate(index),
    }
    if workspace.state.active == was {
        say(workspace, "That is the photo showing.");
        return false;
    }
    crate::panels::viewer::set_zoom(workspace, None);
    let shown = workspace.state.project_name();
    say(workspace, format!("Showing {shown} (photo {} of {count}).", workspace.state.active + 1));
    true
}

/// Shows another open photo, as a step of a message.
fn go(workspace: &mut Workspace, place: Where) -> Outcome {
    let moved = show(workspace, place);
    Outcome { replies: if moved { vec!["Next photo".to_owned(), "What's wrong?".to_owned()] } else { Vec::new() }, ..Outcome::default() }
}

/// Saves the photo's settings as a preset of your own called `preset_name`, or opens New
/// Preset to ask for a name.
fn save_preset(workspace: &mut Workspace, preset_name: Option<String>, window: &mut Window, cx: &mut Context<Workspace>) -> Outcome {
    let Some(develop) = workspace.state.edit_settings().map(|edit| edit.develop.clone()) else { return Outcome::failed() };
    let Some(preset_name) = preset_name else {
        crate::panels::dialogs::new_preset(workspace, window, cx);
        say(workspace, "New Preset is open: give it a name and tick the groups of settings it should set.");
        return Outcome::waiting();
    };
    // A built-in preset of that name would be the one applied by name, hiding this one.
    if develop_core::presets::builtin().iter().any(|preset| preset.name.eq_ignore_ascii_case(&preset_name)) {
        say(workspace, format!("{preset_name} is one of the presets Omina Develop comes with, so pick another name."));
        return Outcome::failed();
    }
    let replaced = crate::panels::dialogs::add_user_preset(workspace, Preset::new(preset_name.clone(), crate::panels::dialogs::USER_GROUP, develop, PresetParts::default()));
    say(
        workspace,
        format!(
            "{} the preset “{preset_name}” from this photo's settings. It sets everything but the white balance, as New Preset starts with, and is under User Presets. Say “try {preset_name}” on any photo.",
            if replaced { "Replaced" } else { "Saved" }
        ),
    );
    Outcome::default()
}

/// The export choices a request comes to for a photo whose finished size is `width` ×
/// `height`: what Export last used, with what the request names in its place. Export sizes by
/// the longer side, so a width or height asked for is turned into that for this photo.
pub(super) fn export_choices(last: ExportChoices, asked: &ExportAsk, format: Option<ExportFormat>, (width, height): (u32, u32)) -> ExportChoices {
    let long_edge = |side: u32, along: u32, other: u32| if along >= other { side } else { (side as f32 * other as f32 / along.max(1) as f32).round() as u32 };
    let size = match asked.size {
        None => last.size,
        Some(SizeAsk::Full) => ExportSize::Full,
        Some(SizeAsk::Long(edge)) => ExportSize::LongEdge(edge),
        Some(SizeAsk::Wide(wide)) => ExportSize::LongEdge(long_edge(wide, width, height)),
        Some(SizeAsk::Tall(tall)) => ExportSize::LongEdge(long_edge(tall, height, width)),
    };
    ExportChoices { format: format.unwrap_or(last.format), quality: asked.quality.unwrap_or(last.quality), size }
}

/// Export choices in words: "JPEG, quality 80".
fn choices_in_words(choices: ExportChoices) -> String {
    if choices.format.has_quality() { format!("{}, quality {}", choices.format.name(), choices.quality) } else { choices.format.name().to_owned() }
}

/// Exports as asked. A request that says enough (a format, "now", every photo) writes the
/// files straight into the folder exports go to; a plain "export", or one with nowhere known
/// to put the files, opens the Export sheet with the choices made.
fn export(workspace: &mut Workspace, asked: ExportAsk, window: &mut Window, cx: &mut Context<Workspace>) -> Outcome {
    let Some(photo) = workspace.state.photo() else { return Outcome::failed() };
    let last = workspace.ui.settings.export;
    let size = photo.edit.output_size(photo.source.size());
    let photo_name = photo.name.clone();
    let folder = crate::panels::export::folder_without_asking(workspace);
    let (true, Some(folder)) = (asked.is_direct(), folder.clone()) else {
        // The sheet, for the photo showing.
        let choices = export_choices(last, &asked, asked.formats.first().copied(), size);
        let (out_width, out_height) = crate::panels::export::exported_size(size.0, size.1, choices.size);
        let mut text = format!("Export is open for {photo_name} with {}, {out_width} × {out_height} pixels. Check it over and press Export… to pick where it goes.", choices_in_words(choices));
        if asked.is_direct() {
            text.push_str(" I don't know where your exports go yet (this photo isn't saved as a project, and nothing has been exported before). Once one export is saved, I'll write the next ones to the same folder without asking.");
        } else {
            text.push_str(" To skip this sheet, name a format or say “export now”.");
        }
        if asked.quality.is_some() && !choices.format.has_quality() {
            text.push_str(&format!(" ({} has no quality setting, so I left that out.)", choices.format.name()));
        }
        if matches!(choices.size, ExportSize::LongEdge(_)) && (out_width, out_height) == size && asked.size.is_some() {
            text.push_str(" (The photo is no bigger than that, and Export never enlarges, so it stays at its full size.)");
        }
        crate::panels::export::open_with(workspace, choices, window, cx);
        say(workspace, text);
        return Outcome::waiting();
    };
    // Recipes set to run before an export run first, on the photo showing; what they changed
    // is in what is written.
    run_trigger(workspace, Trigger::BeforeExport, window, cx);
    let size = workspace.state.photo().map_or(size, |photo| photo.edit.output_size(photo.source.size()));
    // Straight to files: each photo asked for, in each format, with its own edit and size.
    let formats: Vec<Option<ExportFormat>> = if asked.formats.is_empty() { vec![None] } else { asked.formats.iter().copied().map(Some).collect() };
    let sessions: Vec<usize> = if asked.all { (0..workspace.state.sessions.len()).collect() } else { vec![workspace.state.active] };
    let mut jobs: Vec<crate::panels::export::Job> = Vec::new();
    for index in &sessions {
        let photo = &workspace.state.sessions[*index].photo;
        for format in &formats {
            let choices = export_choices(last, &asked, *format, photo.edit.output_size(photo.source.size()));
            jobs.push(crate::panels::export::Job { name: photo.name.clone(), source: photo.source.clone(), edit: photo.edit.clone(), choices });
        }
    }
    // These choices are what Export starts with next time, as after using the sheet.
    let remembered = export_choices(last, &asked, asked.formats.first().copied(), size);
    workspace.ui.settings.export = remembered;
    crate::app_settings::update(|settings| settings.export = remembered);
    let kinds: Vec<String> = formats.iter().map(|format| choices_in_words(export_choices(last, &asked, *format, size))).collect();
    let what = if sessions.len() == 1 { photo_name } else { format!("all {} open photos", sessions.len()) };
    let sized = match remembered.size {
        ExportSize::Full => "at full size".to_owned(),
        ExportSize::LongEdge(_) if sessions.len() == 1 => {
            let (out_width, out_height) = crate::panels::export::exported_size(size.0, size.1, remembered.size);
            format!("at {out_width} × {out_height} pixels")
        }
        ExportSize::LongEdge(edge) => format!("each sized to fit {edge} pixels on its longer side (as asked, for its own shape)"),
    };
    say(workspace, format!("Exporting {what} as {} {sized} to {}. I'll say here when {} written.", kinds.join(" and "), folder.display(), if jobs.len() == 1 { "it's" } else { "they're" }));
    crate::panels::export::write_all(workspace, jobs, folder, window, cx, |workspace, written, window, cx| {
        say(workspace, export_report(written));
        show_newest(window, cx);
    });
    Outcome::default()
}

/// What an export wrote, in words: each file's full path, the folder for several, and every
/// photo that couldn't be written with why.
fn export_report(written: &crate::panels::export::Written) -> String {
    let failed: Vec<String> = written.failed.iter().map(|(photo, why)| format!("• {photo}: {why}")).collect();
    let mut text = match written.files.as_slice() {
        [] => "Nothing was exported.".to_owned(),
        [file] => format!("Exported: {}", file.display()),
        files => {
            let names: Vec<String> = files.iter().map(|file| format!("• {}", file.file_name().map_or_else(String::new, |name| name.to_string_lossy().into_owned()))).collect();
            format!("Exported {} files to {}:\n{}", files.len(), written.folder.display(), names.join("\n"))
        }
    };
    if !failed.is_empty() {
        text.push_str(&format!("\nCouldn't export {}:\n{}", if failed.len() == 1 { "this one".to_owned() } else { format!("these {}", failed.len()) }, failed.join("\n")));
    }
    text
}

/// The six light sliders Auto sets, as they are now where they differ from `before`.
fn light_set(workspace: &Workspace, before: &Option<develop_core::Photo>) -> String {
    use DevelopSetting::*;
    let (Some(now), Some(before)) = (workspace.state.edit_settings(), before) else { return String::new() };
    let moved: Vec<String> = [Exposure, Contrast, Highlights, Shadows, Whites, Blacks]
        .into_iter()
        .filter(|setting| (setting.get(&now.develop) - setting.get(&before.edit.develop)).abs() > 1e-3)
        .map(|setting| chat::valued(setting, setting.get(&now.develop)))
        .collect();
    moved.join(", ")
}

/// "Make it look good": Auto for the light, unless it has been set by hand, then every fix the
/// rules suggest, each measured after the one before. Says each thing done, and is one Undo.
fn improve(workspace: &mut Workspace) -> Outcome {
    use DevelopSetting::*;
    let before = workspace.state.photo().cloned();
    let Some(develop) = before.as_ref().map(|photo| photo.edit.develop.clone()) else { return Outcome::failed() };
    let mut done: Vec<String> = Vec::new();
    let light_untouched = [Exposure, Contrast, Highlights, Shadows, Whites, Blacks].into_iter().all(|setting| setting.get(&develop).abs() < 1e-3);
    if light_untouched {
        workspace.auto();
        let set = light_set(workspace, &before);
        if !set.is_empty() {
            done.push(format!("Auto for the light: {set}"));
        }
    }
    done.extend(fix_all(workspace));
    if unchanged(workspace, &before) {
        let why = if light_untouched { "Auto would leave the light where it is, and" } else { "I've left the light as you set it, and" };
        workspace.ui.iris.messages.push(Message {
            replies: vec!["Make it pop".to_owned(), "Make it moody".to_owned(), "Is this ready?".to_owned()],
            ..Message::from_iris(format!("It already looks right by my rules: {why} I have nothing to fix. Tell me the way to take it, like “make it pop” or “a bit warmer”."))
        });
        return Outcome::default();
    }
    workspace.state.label = Some("Iris: Make It Look Good".to_owned());
    forget_tunable(workspace);
    feel(workspace, Mood::Happy);
    let kept = if light_untouched { "" } else { "\nI left the light as you set it; say “fix the light” for Auto." };
    say(workspace, format!("Here's what I did:\n• {}{kept}\nOne Undo takes it all back.", done.join("\n• ")));
    Outcome::changed(&["Undo that", "Is this ready?"])
}

/// Crops in (or back out) by a share of the crop's size and says what that came to.
fn tighten(workspace: &mut Workspace, share: f32, before: &Option<develop_core::Photo>) -> Outcome {
    workspace.state.tighten_crop(share);
    let percent = (share.abs() * 100.0).round();
    let way = if share >= 0.0 { format!("Cropped in by {percent:.0}% each way") } else { format!("Cropped back out by {percent:.0}% each way") };
    let text = crop_words(workspace, &format!("{way}, same shape and middle"));
    let same = if share >= 0.0 { "The crop is as tight as it goes." } else { "The crop already takes in as much of the photo as its shape allows." };
    let outcome = changed_or(workspace, before, text, same, &["Undo that", "More", "Less"]);
    if outcome.changed
        && let Some(session) = workspace.state.session()
    {
        workspace.ui.iris.last_crop = Some(LastTighten { session: session.id, share, after: session.photo.edit.crop.clone() });
    }
    outcome
}

/// Straightens the photo by the tilt measured in it, or says that none could be measured and
/// opens the tool to do it by eye.
fn level(workspace: &mut Workspace) -> Outcome {
    use crate::commands::Leveled;
    let before = workspace.state.photo().cloned();
    match workspace.level() {
        None => Outcome::failed(),
        Some(Leveled::To(angle)) => {
            let text = format!("Leveled: straighten angle {angle:+.2}°, measured from the clearest level or upright line in the photo. If it picked the wrong line, say “undo that” or give the angle yourself (“straighten 1.5 degrees”).");
            changed_or(workspace, &before, text, "The photo already looks level.", &["Undo that"])
        }
        Some(Leveled::Already) => {
            say(workspace, "It already looks level: the clearest line in the photo is within a twentieth of a degree of where the straighten angle has it.");
            Outcome::default()
        }
        Some(Leveled::Unclear) => {
            workspace.state.set_tool(Tool::Crop);
            say(
                workspace,
                "I couldn't find a horizon or upright edge clear enough to level by, and I won't guess. Crop & Straighten is open: drag outside the crop box until it's level (hold Shift for whole degrees), or give me the angle, like “straighten 1.5 degrees”.",
            );
            Outcome::default()
        }
    }
}

/// What a message says about export presets ("export for Instagram", "save these export
/// settings as Print"), if anything.
fn export_preset_said(text: &str) -> Option<::iris::export_presets::Said> {
    let known: Vec<String> = crate::panels::export::all_presets().into_iter().map(|preset| preset.name).collect();
    ::iris::export_presets::read(text, &known)
}

/// Exports with a preset, keeps the export choices as one, forgets one, or lists them.
fn export_preset(workspace: &mut Workspace, said: ::iris::export_presets::Said, window: &mut Window, cx: &mut Context<Workspace>) -> Outcome {
    use ::iris::export_presets::Said;
    use crate::panels::export as sheet;
    match said {
        Said::Problem(why) => {
            say(workspace, why);
            Outcome::failed()
        }
        Said::List => {
            let lines: Vec<String> = sheet::all_presets().iter().map(|preset| format!("• {}: {}", preset.name, sheet::preset_choices(preset).map_or_else(|| "can't be read".to_owned(), |choices| choices.described()))).collect();
            say(workspace, format!("Export presets:\n{}\nSay “export for Web”, or “save these export settings as Print” to keep the ones Export last used.", lines.join("\n")));
            Outcome::default()
        }
        Said::Save(preset_name) => match sheet::save_preset(&preset_name, sheet::PresetChoices::from_choices(workspace.ui.settings.export)) {
            Ok(choices) => {
                say(workspace, format!("Saved the export preset “{preset_name}”: {}, the choices Export last used. Say “export for {preset_name}” to use it.", choices.described()));
                Outcome::default()
            }
            Err(why) => {
                say(workspace, why);
                Outcome::failed()
            }
        },
        Said::Forget(preset_name) => {
            say(workspace, sheet::forget_preset(&preset_name));
            Outcome::default()
        }
        Said::Use(preset_name) => {
            let Some(choices) = sheet::all_presets().iter().find(|preset| preset.name == preset_name).and_then(sheet::preset_choices) else {
                say(workspace, format!("The export preset “{preset_name}” can't be read."));
                return Outcome::failed();
            };
            let mut note = String::new();
            if let Some(why) = choices.note() {
                note = format!(" ({why})");
            }
            say(workspace, format!("Exporting with the “{preset_name}” preset: {}{note}.", choices.described()));
            export(workspace, choices.ask(), window, cx)
        }
    }
}

/// Undoes the last change and says which it was. A change Iris just made to several photos is
/// taken back on all of them.
fn undo(workspace: &mut Workspace) -> Outcome {
    if let Some(done) = workspace.ui.iris.last_batch.take() {
        return batch::undo(workspace, done);
    }
    workspace.state.record_history(false);
    let (step, _) = workspace.state.undo_and_redo_names();
    if !workspace.state.undo() {
        say(workspace, "There's nothing to undo.");
        return Outcome::failed();
    }
    forget_tunable(workspace);
    say(workspace, format!("Undone: {}.", step.unwrap_or_else(|| "the last change".to_owned())));
    Outcome::changed(&["Redo"])
}

/// Puts back what was just undone and says which change it was.
fn redo(workspace: &mut Workspace) -> Outcome {
    workspace.state.record_history(false);
    let (_, step) = workspace.state.undo_and_redo_names();
    if !workspace.state.redo() {
        say(workspace, "There's nothing to redo.");
        return Outcome::failed();
    }
    say(workspace, format!("Redone: {}.", step.unwrap_or_else(|| "the last change".to_owned())));
    Outcome::changed(&["Undo that"])
}

/// What a short reply that refers back ("more", "too much", "undo that", "same for the next
/// one") means here.
fn follow_up(workspace: &mut Workspace, followup: &Followup, window: &mut Window, cx: &mut Context<Workspace>) -> Outcome {
    match followup {
        Followup::Undo => undo(workspace),
        Followup::Redo => redo(workspace),
        Followup::More => tune(workspace, true),
        Followup::Less => tune(workspace, false),
        Followup::SameFor(place) => same_for(workspace, place, window, cx),
        // The front door turns "again" into the turns of the message it repeats.
        Followup::Again => Outcome::default(),
    }
}

/// "More" makes the last slider change again from where the photo is now; "less" takes half
/// of it back.
fn tune(workspace: &mut Workspace, more: bool) -> Outcome {
    if let Some(last) = workspace.ui.iris.last_crop.clone() {
        return tune_crop(workspace, &last, more);
    }
    let (Some(last), Some(session)) = (workspace.ui.iris.last_adjust.clone(), workspace.state.session()) else {
        say(workspace, "“More” and “less” follow a slider change or a tighter crop, like “a bit warmer”, “make it moody” or “crop it tighter”. There isn't one to build on right now.");
        return Outcome::failed();
    };
    let (id, now) = (session.id, session.photo.edit.develop.clone());
    // How far each slider moved last time.
    let steps: Vec<(DevelopSetting, f32)> = last.settings.iter().map(|setting| (*setting, setting.get(&last.after) - setting.get(&last.before))).filter(|(_, step)| step.abs() > 1e-4).collect();
    let rounded = |setting: DevelopSetting, value: f32| {
        let info = setting.slider();
        let places = 10f32.powi(info.decimals as i32);
        ((value * places).round() / places).clamp(info.min, info.max)
    };
    if more {
        let changes = steps.iter().map(|(setting, step)| (*setting, rounded(*setting, setting.get(&now) + step))).collect();
        let fix = Fix::settings(format!("More: {}", last.label), changes);
        let outcome = adjust(workspace, format!("More: {}.", chat::listed_changes(&fix)), &fix);
        // The change keeps its own name, so "more" again isn't "More: More: …".
        if outcome.changed
            && let Some(made) = &mut workspace.ui.iris.last_adjust
        {
            made.label = last.label;
        }
        return outcome;
    }
    if last.session != id {
        say(workspace, "The change I'd soften was made to another photo. Say what to ease on this one, like “less contrast”.");
        return Outcome::failed();
    }
    let changes: Vec<(DevelopSetting, f32)> = steps.iter().map(|(setting, step)| (*setting, rounded(*setting, setting.get(&now) - step * 0.5))).collect();
    let moves_from = |from: &Develop| changes.iter().any(|(setting, value)| (value - setting.get(from)).abs() > 1e-4);
    if !moves_from(&now) {
        say(workspace, "That change is too small to halve. Say “undo that” to take it back altogether.");
        return Outcome::failed();
    }
    let fix = Fix::settings(format!("Less: {}", last.label), changes.clone());
    // With nothing changed since, the change is undone and made again at half strength, so
    // History keeps one step for it rather than two. An Undo that takes back anything else is
    // put back, and the softer change is made on top instead.
    workspace.state.record_history(false);
    if now == last.after && workspace.state.undo() {
        let undone = workspace.state.edit_settings().is_some_and(|edit| edit.develop == last.before);
        if !undone {
            workspace.state.redo();
        } else if !moves_from(&last.before) {
            // Half of it rounds to nothing, so undoing it is all there is to do.
            forget_tunable(workspace);
            say(workspace, format!("Less: {} is taken back altogether (half of it would round to nothing).", last.label));
            return Outcome::changed(&["Redo"]);
        }
    }
    let outcome = adjust(workspace, format!("Less: {} (half of that last change taken back).", chat::listed_changes(&fix)), &fix);
    // The next "less" halves what is left of the change, whichever way this one was made.
    if outcome.changed
        && let Some(made) = &mut workspace.ui.iris.last_adjust
    {
        let mut start = made.after.clone();
        for (setting, step) in &steps {
            setting.set(&mut start, setting.get(&made.after) - step * 0.5);
        }
        made.before = start;
        made.label = last.label;
    }
    outcome
}

/// "More" after a tighter crop crops in as far again; "less" takes half of it back.
fn tune_crop(workspace: &mut Workspace, last: &LastTighten, more: bool) -> Outcome {
    let before = workspace.state.photo().cloned();
    let Some((id, crop)) = workspace.state.session().map(|session| (session.id, session.photo.edit.crop.clone())) else { return Outcome::failed() };
    if more {
        return tighten(workspace, last.share, &before);
    }
    if id != last.session {
        say(workspace, "The crop I'd ease was another photo's. Say “crop it looser” for this one.");
        return Outcome::failed();
    }
    // Half of it back: undone and made again at half where nothing has changed since, as
    // with a slider change. An Undo that takes back anything else is put back.
    workspace.state.record_history(false);
    let half = last.share * 0.5;
    if crop == last.after && workspace.state.undo() {
        let was_that_crop = workspace.state.photo().cloned().is_some_and(|mut photo| {
            crate::state::tighten_crop(&mut photo, last.share);
            photo.edit.crop == last.after
        });
        if was_that_crop {
            let undone = workspace.state.photo().cloned();
            return Outcome { changed: true, ..tighten(workspace, half, &undone) };
        }
        workspace.state.redo();
    }
    // From a crop at (1 - share) of the size it was, back out to (1 - half) of it.
    let outcome = tighten(workspace, 1.0 - (1.0 - half) / (1.0 - last.share), &before);
    // What is left of the change is the half, which the next "less" halves again.
    if let Some(made) = &mut workspace.ui.iris.last_crop {
        made.share = half;
    }
    outcome
}

/// "Same for the next one", "same for all the others", "same for Harbor": the last request
/// made again on another open photo, or on all of them.
fn same_for(workspace: &mut Workspace, place: &str, window: &mut Window, cx: &mut Context<Workspace>) -> Outcome {
    let Some(last) = workspace.ui.iris.memory.last_change.clone() else {
        say(workspace, "There's nothing to repeat yet: I haven't changed anything.");
        return Outcome::failed();
    };
    let count = workspace.state.sessions.len();
    if count < 2 {
        say(workspace, "This is the only photo open, so there is no other to do it to.");
        return Outcome::failed();
    }
    let lower = place.to_lowercase();
    let words: Vec<&str> = lower.split(|character: char| !character.is_alphanumeric()).filter(|word| !word.is_empty()).collect();
    let said = |list: &[&str]| words.iter().any(|word| list.contains(word));
    if said(&["all", "every", "each", "others", "other", "rest", "everything"]) {
        return batch::propose(workspace, batch::What::Repeat(last), place);
    }
    let by_name = workspace.state.sessions.iter().position(|session| session.photo.name.to_lowercase() == lower).or_else(|| workspace.state.sessions.iter().position(|session| lower.contains(&session.photo.name.to_lowercase())));
    let target = if said(&["next"]) {
        Where::Next
    } else if said(&["previous", "prev"]) {
        Where::Previous
    } else if let Some(index) = by_name {
        Where::Index(index)
    } else {
        say(workspace, format!("I can do the same for the next photo, for all the others, or for an open photo by its name. “{place}” isn't one of those."));
        return Outcome::failed();
    };
    if !show(workspace, target) {
        return Outcome::failed();
    }
    // The last request is read afresh for the photo now showing, so "a bit brighter" is a bit
    // brighter than this photo was.
    let turns = front::read(&last, &mut workspace.ui.iris.memory, &Units::default(), &reserved()).turns;
    take_turns(workspace, &turns, window, cx)
}

#[cfg(test)]
mod tests {
    use ::iris::commands::{Command, Found, find};

    use super::*;

    /// The command a message names among all of Develop's, by its id.
    fn named(message: &str) -> Vec<&'static str> {
        let listed: Vec<Command> = NAMED.iter().map(|command| Command { name: command.name, keywords: command.keywords }).collect();
        match find(message, &listed) {
            Found::None => Vec::new(),
            Found::One(index) => vec![NAMED[index].id],
            Found::Several(indexes) => indexes.into_iter().map(|index| NAMED[index].id).collect(),
        }
    }

    #[test]
    fn menu_commands_are_found_by_name_and_every_one_can_be_run() {
        for (message, id) in [
            ("open", "open"),
            ("open a photo", "open"),
            ("save", "save"),
            ("save as", "save-as"),
            ("close this photo", "close"),
            ("zoom in", "zoom-in"),
            ("zoom out", "zoom-out"),
            ("fit on screen", "fit"),
            ("zoom to 100%", "actual-size"),
            ("show clipping", "clipping"),
            ("hide clipping", "clipping"),
            ("white balance selector", "white-balance-tool"),
            ("eyedropper", "white-balance-tool"),
            ("radial mask", "radial-mask-tool"),
            ("hand tool", "view-tool"),
            ("show history", "show-history"),
            ("open presets", "show-presets"),
            ("start screen", "start-screen"),
            ("full screen", "full-screen"),
            ("reset the workspace", "reset-workspace"),
            ("about", "about"),
        ] {
            assert_eq!(named(message), vec![id], "{message}");
        }
        // Commands that fit equally are offered, not guessed between.
        assert_eq!(named("zoom"), vec!["zoom-in", "zoom-out"]);
        assert_eq!(named("mask"), vec!["linear-mask-tool", "radial-mask-tool"]);
        // Questions and other sentences find nothing.
        for message in ["how do I zoom in?", "bake a cake", "zoom in on the left eye of the bird please now", "quit", "hide the app", "delete mask"] {
            assert!(named(message).is_empty(), "{message}");
        }
        // Each is either a panel to show or a command Develop really has.
        for command in NAMED {
            assert!(side_panel(command.id).is_some() || crate::commands::action_for(command.id).is_some(), "{} can't be run", command.id);
        }
    }

    #[test]
    fn requests_are_told_from_questions_and_topics() {
        for request in ["make it purple", "Blur the background", "please remove the person", "add text"] {
            assert!(asks_for_something(request), "{request}");
        }
        for other in ["vignette", "how do I blur the background?", "can I add text?", "tone curve", "what is clarity", ""] {
            assert!(!asks_for_something(other), "{other}");
        }
    }


    #[test]
    fn sliders_belong_to_the_cards_that_can_be_turned_off() {
        assert_eq!(feature_of(DevelopSetting::MixerSaturation(5)), Some(Feature::ColorMixer));
        assert_eq!(feature_of(DevelopSetting::Grade(0, 1)), Some(Feature::ColorGrading));
        assert_eq!(feature_of(DevelopSetting::Vignette), Some(Feature::Effects));
        assert_eq!(feature_of(DevelopSetting::NoiseLuminance), Some(Feature::Detail));
        assert_eq!(feature_of(DevelopSetting::Exposure), None, "the Basic card is always on");
        assert_eq!(feature_name(Feature::Effects), "Effects");
    }
}

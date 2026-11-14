//! Keyboard shortcuts: Lightroom's keys for the tools and commands, and the person's own
//! changes on top (Settings ▸ Keyboard Shortcuts). Tool buttons, menus and Iris's answers read
//! keys from here, so they always name the key that works now.
//!
//! The keys in use are kept for the thread the window runs on (`use_shortcuts`), so the many
//! places that show a key don't each need the settings handed to them. The recipes Iris has
//! been given keys for are kept beside them (`use_recipe_keys`): those keys are in use too, so
//! a command can't be given one.

use std::cell::RefCell;
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// A command's id and its key as GPUI writes it (macOS symbols are shown).
pub const SHORTCUTS: &[(&str, &str)] = &[
    ("view-tool", "h"),
    ("crop-tool", "r"),
    ("white-balance-tool", "w"),
    ("linear-mask-tool", "m"),
    ("radial-mask-tool", "shift-m"),
    ("before", "\\"),
    ("split", "y"),
    ("auto", "cmd-u"),
    ("reset-all", "cmd-shift-r"),
    ("undo", "cmd-z"),
    ("redo", "cmd-shift-z"),
    ("open", "cmd-o"),
    ("import", "cmd-shift-i"),
    ("save", "cmd-s"),
    ("save-as", "cmd-shift-s"),
    ("close", "cmd-w"),
    ("export", "cmd-shift-e"),
    ("copy-settings", "cmd-shift-c"),
    ("paste-settings", "cmd-shift-v"),
    ("new-snapshot", "cmd-n"),
    ("new-preset", "cmd-shift-n"),
    ("rotate-left", "cmd-["),
    ("rotate-right", "cmd-]"),
    ("delete-mask", "backspace"),
    ("zoom-in", "cmd-="),
    ("zoom-out", "cmd--"),
    ("fit", "cmd-0"),
    ("actual-size", "cmd-1"),
    ("toggle-zoom", "z"),
    ("clipping", "j"),
    ("settings", "cmd-,"),
    ("next-photo", "ctrl-tab"),
    ("previous-photo", "ctrl-shift-tab"),
];

/// Each command's name as Settings lists it, and the heading it is under.
const NAMES: &[(&str, &str, &str)] = &[
    ("view-tool", "Hand", "Tools"),
    ("crop-tool", "Crop & Straighten", "Tools"),
    ("white-balance-tool", "White Balance Selector", "Tools"),
    ("linear-mask-tool", "Linear Gradient Mask", "Tools"),
    ("radial-mask-tool", "Radial Gradient Mask", "Tools"),
    ("delete-mask", "Delete Mask", "Tools"),
    ("auto", "Auto", "Settings"),
    ("reset-all", "Reset All", "Settings"),
    ("copy-settings", "Copy Settings", "Settings"),
    ("paste-settings", "Paste Settings", "Settings"),
    ("new-snapshot", "New Snapshot", "Settings"),
    ("new-preset", "New Preset", "Settings"),
    ("undo", "Undo", "Edit"),
    ("redo", "Redo", "Edit"),
    ("open", "Open", "File"),
    ("import", "Import (Open)", "File"),
    ("save", "Save", "File"),
    ("save-as", "Save As", "File"),
    ("close", "Close Photo", "File"),
    ("export", "Export", "File"),
    ("rotate-left", "Rotate Left", "Photo"),
    ("rotate-right", "Rotate Right", "Photo"),
    ("before", "Before", "View"),
    ("split", "Before / After Split", "View"),
    ("clipping", "Show Clipping", "View"),
    ("toggle-zoom", "Zoom to 100% and Back", "View"),
    ("zoom-in", "Zoom In", "View"),
    ("zoom-out", "Zoom Out", "View"),
    ("fit", "Fit on Screen", "View"),
    ("actual-size", "100%", "View"),
    ("next-photo", "Next Photo", "Window"),
    ("previous-photo", "Previous Photo", "Window"),
    ("settings", "Settings", "Window"),
];

/// The person's own changes to the shortcuts, saved in settings.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Shortcuts {
    /// By command id: the key it has instead of its own, or `None` for no key at all.
    pub custom: BTreeMap<String, Option<String>>,
}

thread_local! {
    /// The shortcuts in use on this thread (the window's).
    static IN_USE: RefCell<Shortcuts> = RefCell::new(Shortcuts::default());
    /// The recipes with keys of their own, as each key and the recipe's name.
    static RECIPE_KEYS: RefCell<Vec<(String, String)>> = const { RefCell::new(Vec::new()) };
}

/// A command's name as Settings lists it ("Crop & Straighten"), or its id for one not listed.
pub fn name(id: &str) -> &str {
    NAMES.iter().find(|(known, _, _)| *known == id).map_or(id, |(_, name, _)| name)
}

/// Each command that can have a shortcut, with its name and heading, in the order Settings
/// lists them.
pub fn commands() -> impl Iterator<Item = (&'static str, &'static str, &'static str)> {
    NAMES.iter().copied()
}

/// A key written with its modifiers in one fixed order (⌃ ⌥ ⇧ ⌘, as `iris::recipes::
/// normalize_key` writes them), so the same key typed or recorded two ways compares equal.
pub fn canonical(key: &str) -> String {
    let key = key.trim().to_lowercase();
    // A key that is itself "-" leaves an empty last part.
    let (modifiers, last) = match key.strip_suffix("--") {
        Some(rest) => (rest, "-".to_owned()),
        None => match key.rsplit_once('-') {
            Some((rest, last)) if !last.is_empty() => (rest, last.to_owned()),
            _ => ("", key.clone()),
        },
    };
    let has = |name: &str| modifiers.split('-').any(|part| part == name || (name == "cmd" && part == "super"));
    let mut parts: Vec<String> = ["ctrl", "alt", "shift", "cmd"].into_iter().filter(|name| has(name)).map(str::to_owned).collect();
    parts.push(last);
    parts.join("-")
}

/// Whether `key` could be a keystroke at all. One GPUI can't read is still left unbound (see
/// `workspace::apply_keys`), so a damaged settings file can't stop the app.
fn readable(key: &str) -> bool {
    !key.trim().is_empty() && !key.contains(char::is_control)
}

/// Every command's key now: its own unless changed. `None` for a command left without one.
pub fn keys_in_use(shortcuts: &Shortcuts) -> Vec<(&'static str, Option<String>)> {
    SHORTCUTS
        .iter()
        .map(|(id, key)| match shortcuts.custom.get(*id) {
            Some(custom) => (*id, custom.clone().filter(|key| readable(key))),
            None => (*id, Some((*key).to_owned())),
        })
        .collect()
}

/// The command other than `except` that `key` belongs to now, if any.
pub fn used_by(shortcuts: &Shortcuts, key: &str, except: &str) -> Option<&'static str> {
    let key = canonical(key);
    keys_in_use(shortcuts).into_iter().find(|(id, used)| *id != except && used.as_deref().is_some_and(|used| canonical(used) == key)).map(|(id, _)| id)
}

/// The recipe that runs on `key`, if any.
pub fn recipe_on(key: &str) -> Option<String> {
    let key = canonical(key);
    RECIPE_KEYS.with(|keys| keys.borrow().iter().find(|(used, _)| canonical(used) == key).map(|(_, name)| name.clone()))
}

/// Gives command `id` the key `key`, or none with `None`. Another command that had the key
/// loses it, and is returned. A key a recipe runs on is refused (`Err` with the recipe's name):
/// the recipe's key is changed where recipes are.
pub fn assign(shortcuts: &mut Shortcuts, id: &str, key: Option<&str>) -> Result<Option<&'static str>, String> {
    if let Some(recipe) = key.and_then(recipe_on) {
        return Err(recipe);
    }
    let taken = key.and_then(|key| used_by(shortcuts, key, id));
    if let Some(other) = taken {
        shortcuts.custom.insert(other.to_owned(), None);
    }
    shortcuts.custom.insert(id.to_owned(), key.map(str::to_owned));
    // A command back on its own key isn't a change.
    let own: BTreeMap<&str, &str> = SHORTCUTS.iter().copied().collect();
    shortcuts.custom.retain(|id, key| own.get(id.as_str()).map(|own| canonical(own)) != key.as_deref().map(canonical));
    Ok(taken)
}

/// Puts command `id` back on its own key; a command that had taken that key loses it.
pub fn reset(shortcuts: &mut Shortcuts, id: &str) -> Option<&'static str> {
    let own = SHORTCUTS.iter().find(|(known, _)| *known == id).map(|(_, key)| *key)?;
    let taken = used_by(shortcuts, own, id);
    if let Some(other) = taken {
        shortcuts.custom.insert(other.to_owned(), None);
    }
    shortcuts.custom.remove(id);
    taken
}

/// Uses `shortcuts` on this thread from now on.
pub fn use_shortcuts(shortcuts: &Shortcuts) {
    IN_USE.with(|in_use| *in_use.borrow_mut() = shortcuts.clone());
}

/// Uses `keyed` (each key and the recipe it runs) as the recipe keys from now on.
pub fn use_recipe_keys(keyed: Vec<(String, String)>) {
    RECIPE_KEYS.with(|keys| *keys.borrow_mut() = keyed);
}

/// The recipe keys in use, each with the recipe it runs.
pub fn recipe_keys() -> Vec<(String, String)> {
    RECIPE_KEYS.with(|keys| keys.borrow().clone())
}

/// Every key the app's own commands use now, written as recipes' keys are, so Iris never
/// gives a recipe one of them.
pub fn app_keys() -> Vec<String> {
    let shortcuts = IN_USE.with(|in_use| in_use.borrow().clone());
    let fixed = ["cmd-q", "cmd-h", "cmd-alt-h", "cmd-m", "ctrl-cmd-f", "cmd-c", "cmd-v", "cmd-x", "cmd-a"].map(str::to_owned);
    keys_in_use(&shortcuts).into_iter().filter_map(|(_, key)| key).chain(fixed).map(|key| canonical(&key)).collect()
}

/// The key for a command now, written the way macOS menus show it (⇧⌘S), if it has one.
pub fn key_for(command: &str) -> Option<String> {
    binding(command).map(|key| display(&key))
}

/// The key for a command now, as GPUI writes it, if it has one.
pub fn binding(command: &str) -> Option<String> {
    let shortcuts = IN_USE.with(|in_use| in_use.borrow().clone());
    keys_in_use(&shortcuts).into_iter().find(|(id, _)| *id == command).and_then(|(_, key)| key)
}

/// A name with its key in brackets ("Crop (R)"), for tooltips.
pub fn with_key(name: &str, command: &str) -> String {
    match key_for(command) {
        Some(key) => format!("{name} ({key})"),
        None => name.to_owned(),
    }
}

/// A GPUI key like "cmd-shift-z" written as macOS shows it: ⇧⌘Z.
pub fn display(key: &str) -> String {
    let mut modifiers = String::new();
    let mut parts: Vec<&str> = key.split('-').collect();
    // A key that is itself "-" leaves an empty last part.
    let last = if key.ends_with("--") {
        parts.truncate(parts.len().saturating_sub(2));
        "-".to_owned()
    } else {
        parts.pop().unwrap_or_default().to_owned()
    };
    // macOS lists modifiers in this order.
    for (name, symbol) in [("ctrl", "⌃"), ("alt", "⌥"), ("shift", "⇧"), ("cmd", "⌘")] {
        if parts.contains(&name) {
            modifiers.push_str(symbol);
        }
    }
    let key = match last.as_str() {
        "backspace" => "⌫".to_owned(),
        "delete" => "⌦".to_owned(),
        "enter" => "↩".to_owned(),
        "escape" => "⎋".to_owned(),
        "space" => "Space".to_owned(),
        other => other.to_uppercase(),
    };
    format!("{modifiers}{key}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_read_as_macos_writes_them() {
        assert_eq!(display("cmd-shift-z"), "⇧⌘Z");
        assert_eq!(display("cmd--"), "⌘-");
        assert_eq!(display("\\"), "\\");
        assert_eq!(with_key("Crop", "crop-tool"), "Crop (R)");
        assert_eq!(key_for("nothing"), None);
        assert_eq!(binding("nothing"), None);
    }

    #[test]
    fn keys_are_compared_in_one_order() {
        assert_eq!(canonical("cmd-shift-e"), "shift-cmd-e");
        assert_eq!(canonical("ctrl-alt-cmd-shift-k"), "ctrl-alt-shift-cmd-k");
        assert_eq!(canonical("cmd--"), "cmd--");
        assert_eq!(canonical("j"), "j");
        assert_eq!(canonical("shift-tab"), "shift-tab");
    }

    #[test]
    fn a_new_key_moves_from_the_command_that_had_it_and_a_recipe_keeps_its_own() {
        let mut shortcuts = Shortcuts::default();
        assert_eq!(assign(&mut shortcuts, "crop-tool", Some("c")), Ok(None));
        assert_eq!(assign(&mut shortcuts, "auto", Some("r")), Ok(None), "R is free now that Crop has C");
        // Taking a key another command uses leaves that one without a key.
        assert_eq!(assign(&mut shortcuts, "split", Some("j")), Ok(Some("clipping")));
        let keys: BTreeMap<&str, Option<String>> = keys_in_use(&shortcuts).into_iter().collect();
        assert_eq!((keys["crop-tool"].as_deref(), keys["split"].as_deref(), keys["clipping"].as_deref()), (Some("c"), Some("j"), None));
        // Back to its own key: no longer a change. A recipe's key is refused.
        assert_eq!(reset(&mut shortcuts, "crop-tool"), Some("auto"));
        assert!(!shortcuts.custom.contains_key("crop-tool"));
        use_recipe_keys(vec![("ctrl-alt-t".to_owned(), "tidy".to_owned())]);
        assert_eq!(assign(&mut shortcuts, "fit", Some("alt-ctrl-t")), Err("tidy".to_owned()));
        use_recipe_keys(Vec::new());
        // The keys in use follow the changes, written as recipes' keys are.
        use_shortcuts(&shortcuts);
        assert_eq!(key_for("split"), Some("J".to_owned()));
        assert!(app_keys().contains(&"j".to_owned()) && app_keys().contains(&"shift-cmd-e".to_owned()));
        use_shortcuts(&Shortcuts::default());
    }

    #[test]
    fn a_key_that_cant_be_read_is_left_off() {
        let shortcuts = Shortcuts { custom: [("fit".to_owned(), Some("   ".to_owned()))].into() };
        assert!(keys_in_use(&shortcuts).into_iter().any(|(id, key)| id == "fit" && key.is_none()));
    }

    #[test]
    fn no_key_is_used_twice() {
        let mut keys: Vec<&str> = SHORTCUTS.iter().map(|(_, key)| *key).collect();
        keys.sort();
        let count = keys.len();
        keys.dedup();
        assert_eq!(keys.len(), count);
    }
}

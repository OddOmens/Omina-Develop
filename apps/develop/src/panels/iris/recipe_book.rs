//! Changing Iris's recipes from Settings ▸ Recipes, rather than by talking to her: renaming a
//! taught recipe and rewriting its steps, giving any recipe a key, a button or moments it runs
//! at, putting taught ones in order, deleting them, and sharing them as a file. Each change is
//! checked as Iris checks one she is taught, then saved, and the recipe keys bound anew.

use ::iris::recipes::{self, Recipe, Trigger};

use super::conversation::{reserved, save_recipes};
use crate::workspace::Workspace;

/// One change to the recipes.
#[derive(Clone, Debug, PartialEq)]
pub enum RecipeChange {
    /// Give the taught recipe called `name` this name and these steps.
    Edit { name: String, new_name: String, steps: Vec<String> },
    /// Give a recipe this key (as typed or recorded), or none.
    Key { name: String, key: Option<String> },
    /// Give a recipe a button beside the chat, or take it away.
    Button { name: String, on: bool },
    /// Have a recipe run by itself at a moment, or not.
    Trigger { name: String, trigger: Trigger, on: bool },
    /// Move a taught recipe one place up (or down) the list.
    Move { name: String, up: bool },
    /// Delete a taught recipe; one Develop comes with goes back to how it came.
    Delete { name: String },
    /// Put back a recipe deleted from this place in the list.
    Restore { recipe: Recipe, at: usize },
    /// Add the recipes in a shared file's text.
    Import { text: String },
}

/// The taught recipes and the ones Develop comes with (as they are now, with any key, button
/// or moments given them), for Settings to list.
pub fn recipes_page_data(workspace: &Workspace) -> (Vec<Recipe>, Vec<Recipe>) {
    let memory = &workspace.ui.iris.memory;
    let own: Vec<Recipe> = memory.built_in.iter().map(|own| memory.recipe(&own.name).unwrap_or_else(|| own.clone())).collect();
    let taught = memory.taught.iter().filter(|recipe| !memory.built_in.iter().any(|own| own.name == recipe.name)).cloned().collect();
    (taught, own)
}

/// The recipe called `name` as it can be changed: a taught one, or a copy of one Develop comes
/// with put among the taught ones to hold its key, button and moments.
fn changeable<'a>(workspace: &'a mut Workspace, name: &str) -> Option<&'a mut Recipe> {
    let memory = &mut workspace.ui.iris.memory;
    if !memory.taught.iter().any(|recipe| recipe.name == name) {
        let own = memory.built_in.iter().find(|own| own.name == name).cloned()?;
        memory.taught.push(own);
    }
    memory.taught.iter_mut().find(|recipe| recipe.name == name)
}

/// Makes `change`, saves the recipes and binds their keys. Returns what to say, or why the
/// change won't do (nothing is changed then).
pub fn change_recipes(workspace: &mut Workspace, change: RecipeChange, cx: &mut gpui_kit::App) -> Result<String, String> {
    let said = apply(workspace, change)?;
    save_recipes(workspace, cx);
    Ok(said)
}

fn apply(workspace: &mut Workspace, change: RecipeChange) -> Result<String, String> {
    let is_own = |workspace: &Workspace, name: &str| workspace.ui.iris.memory.built_in.iter().any(|own| own.name == name);
    match change {
        RecipeChange::Edit { name, new_name, steps } => {
            if is_own(workspace, &name) {
                return Err(format!("“{name}” comes with Omina Develop, so its steps can't be changed. Teach a recipe of your own instead."));
            }
            let new_name = recipes::tidy_name(&new_name)?;
            let steps: Vec<String> = steps.into_iter().map(|step| step.trim().to_owned()).filter(|step| !step.is_empty()).collect();
            if steps.is_empty() {
                return Err("A recipe needs at least one step.".to_owned());
            }
            if reserved().iter().any(|taken| taken.trim_start_matches('/').eq_ignore_ascii_case(&new_name)) || is_own(workspace, &new_name) {
                return Err(format!("“{new_name}” is already taken, so pick another name."));
            }
            if new_name != name && workspace.ui.iris.memory.taught.iter().any(|recipe| recipe.name == new_name) {
                return Err(format!("There's a recipe called “{new_name}” already."));
            }
            let candidate = Recipe::new(new_name.clone(), steps.clone());
            if steps.iter().any(|step| matches!(recipes::read(step, std::slice::from_ref(&candidate)), Some(recipes::Said::Run { .. }))) {
                return Err("A recipe can't run itself.".to_owned());
            }
            let recipe = workspace.ui.iris.memory.taught.iter_mut().find(|recipe| recipe.name == name).ok_or_else(|| format!("There's no recipe called “{name}”."))?;
            recipe.name = new_name.clone();
            recipe.steps = steps;
            Ok(format!("Saved “{new_name}”."))
        }
        RecipeChange::Key { name, key } => {
            let key = match key.as_deref().map(str::trim).filter(|key| !key.is_empty()) {
                None => None,
                Some(typed) => {
                    let key = recipes::normalize_key(typed)?;
                    if workspace.ui.iris.memory.app_keys.contains(&key) {
                        return Err(format!("{} already does something in Omina Develop. Pick another, such as ⌃⌥ with a letter.", recipes::key_label(&key)));
                    }
                    Some(key)
                }
            };
            let recipe = workspace.ui.iris.memory.recipe(&name).ok_or_else(|| format!("There's no recipe called “{name}”."))?;
            if key.is_some() && !recipe.slots().is_empty() {
                return Err(format!("“{}” needs values each time, so it can't have a key.", recipe.called()));
            }
            // A key another recipe had moves to this one.
            let mut said = match &key {
                Some(key) => format!("{} now runs “{}”.", recipes::key_label(key), recipe.called()),
                None => format!("“{}” has no key now.", recipe.called()),
            };
            if let Some(key) = &key
                && let Some((_, other)) = workspace.ui.iris.memory.keyed().into_iter().find(|(used, other)| used == key && *other != name)
            {
                if let Some(other) = changeable(workspace, &other) {
                    other.key = None;
                }
                said.push_str(&format!(" It was “{other}”'s, which has none now."));
            }
            changeable(workspace, &name).ok_or_else(|| format!("There's no recipe called “{name}”."))?.key = key;
            Ok(said)
        }
        RecipeChange::Button { name, on } => {
            let recipe = changeable(workspace, &name).ok_or_else(|| format!("There's no recipe called “{name}”."))?;
            if on && !recipe.slots().is_empty() {
                return Err(format!("“{}” needs values each time, so it can't have a button.", recipe.called()));
            }
            recipe.button = on;
            Ok(if on { format!("“{}” has a button beside the chat.", recipe.called()) } else { format!("“{}” has no button.", recipe.called()) })
        }
        RecipeChange::Trigger { name, trigger, on } => {
            let recipe = changeable(workspace, &name).ok_or_else(|| format!("There's no recipe called “{name}”."))?;
            recipe.when.retain(|known| *known != trigger);
            if on {
                recipe.when.push(trigger);
            }
            Ok(format!("“{}” {} run by itself {}.", recipe.called(), if on { "will" } else { "won't" }, trigger.phrase()))
        }
        RecipeChange::Move { name, up } => {
            let taught = &mut workspace.ui.iris.memory.taught;
            let at = taught.iter().position(|recipe| recipe.name == name).ok_or_else(|| format!("There's no recipe called “{name}”."))?;
            let to = if up { at.checked_sub(1) } else { Some(at + 1).filter(|to| *to < taught.len()) };
            if let Some(to) = to {
                taught.swap(at, to);
            }
            Ok(String::new())
        }
        RecipeChange::Delete { name } => {
            let own = is_own(workspace, &name);
            let taught = &mut workspace.ui.iris.memory.taught;
            let at = taught.iter().position(|recipe| recipe.name == name).ok_or_else(|| format!("“{name}” has nothing to delete."))?;
            taught.remove(at);
            Ok(if own { format!("“{name}” is back as it came: no key, button or moments.") } else { format!("Deleted “{name}”.") })
        }
        RecipeChange::Restore { recipe, at } => {
            let taught = &mut workspace.ui.iris.memory.taught;
            if taught.iter().any(|known| known.name == recipe.name) {
                return Err(format!("There's a recipe called “{}” again already.", recipe.name));
            }
            let at = at.min(taught.len());
            let name = recipe.name.clone();
            taught.insert(at, recipe);
            Ok(format!("“{name}” is back."))
        }
        RecipeChange::Import { text } => {
            let incoming = recipes::from_share_text(&text, ::iris::App::Develop)?;
            Ok(workspace.ui.iris.memory.import(incoming))
        }
    }
}

/// Every recipe, taught and Develop's own, as a file to share or keep.
pub fn share_text(workspace: &Workspace) -> String {
    recipes::share_text(::iris::App::Develop, &workspace.ui.iris.memory.taught)
}

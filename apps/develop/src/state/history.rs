//! Undo, redo and the History panel. As in Studio and Canvas, the history keeps whole copies
//! of the photo rather than asking each edit to describe its reverse: after each change it
//! records the photo as it was before. The original's pixels are shared between copies
//! (`Arc`), so a step costs only its settings. Each step has a name (the History panel lists
//! them, as Lightroom's does), and clicking one goes back or forward to it.
//!
//! The caller records only when no drag is in progress, so a whole slider drag is one step.

/// A state as it was, its name in the History panel, and its version number.
struct Step<T> {
    project: T,
    name: String,
    version: u64,
}

/// The undo history of a `T` (the open photo).
pub struct History<T> {
    /// The state as of the last recorded change: what the next change is compared against.
    baseline: T,
    /// The name of the baseline's state (what made it).
    baseline_name: String,
    undo: Vec<Step<T>>,
    redo: Vec<Step<T>>,
    version: u64,
    next_version: u64,
    saved_version: u64,
    /// The most undo steps kept; older ones are dropped.
    pub limit: usize,
}

impl<T: Clone + PartialEq> History<T> {
    /// A fresh history for `project`, which counts as saved. `name` is its first state's name
    /// ("Import", "Open").
    pub fn new(project: &T, name: &str) -> Self {
        Self { baseline: project.clone(), baseline_name: name.to_owned(), undo: Vec::new(), redo: Vec::new(), version: 0, next_version: 1, saved_version: 0, limit: 50 }
    }

    /// Records `project` as a new step named `name` if it changed since the last one. Any redo
    /// steps are dropped, since they branched off an earlier state.
    pub fn record(&mut self, project: &T, name: &str) -> bool {
        if *project == self.baseline {
            return false;
        }
        let before = std::mem::replace(&mut self.baseline, project.clone());
        let before_name = std::mem::replace(&mut self.baseline_name, name.to_owned());
        self.undo.push(Step { project: before, name: before_name, version: self.version });
        while self.undo.len() > self.limit.max(1) {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.version = self.next_version;
        self.next_version += 1;
        true
    }

    /// Puts `project` back one step. Returns whether there was anything to undo.
    pub fn undo(&mut self, project: &mut T) -> bool {
        self.record(project, "Edit");
        let Some(step) = self.undo.pop() else { return false };
        let current_name = std::mem::replace(&mut self.baseline_name, step.name);
        self.redo.push(Step { project: std::mem::replace(project, step.project), name: current_name, version: self.version });
        self.baseline = project.clone();
        self.version = step.version;
        true
    }

    /// Re-applies the last undone step. Returns whether there was anything to redo.
    pub fn redo(&mut self, project: &mut T) -> bool {
        self.record(project, "Edit");
        let Some(step) = self.redo.pop() else { return false };
        let current_name = std::mem::replace(&mut self.baseline_name, step.name);
        self.undo.push(Step { project: std::mem::replace(project, step.project), name: current_name, version: self.version });
        self.baseline = project.clone();
        self.version = step.version;
        true
    }

    pub fn can_undo(&self, project: &T) -> bool {
        !self.undo.is_empty() || *project != self.baseline
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// The History panel's list, oldest first: each state's name, and which one is current.
    pub fn states(&self) -> (Vec<String>, usize) {
        let mut names: Vec<String> = self.undo.iter().map(|step| step.name.clone()).collect();
        names.push(self.baseline_name.clone());
        let current = names.len() - 1;
        names.extend(self.redo.iter().rev().map(|step| step.name.clone()));
        (names, current)
    }

    /// Goes back or forward to the state at `index` in `states`' list.
    pub fn go_to(&mut self, index: usize, project: &mut T) -> bool {
        let (_, current) = self.states();
        let mut changed = false;
        if index < current {
            for _ in index..current {
                changed |= self.undo(project);
            }
        } else {
            for _ in current..index {
                changed |= self.redo(project);
            }
        }
        changed
    }

    pub fn mark_saved(&mut self, project: &T) {
        self.record(project, "Edit");
        self.saved_version = self.version;
    }

    /// Counts the state as not saved whatever is undone or redone, until it is next saved: for
    /// work brought back after Develop closed without saving it.
    pub fn mark_unsaved(&mut self) {
        // No step ever has this version, so the state never matches the saved one.
        self.saved_version = u64::MAX;
    }

    pub fn has_unsaved_changes(&self, project: &T) -> bool {
        self.version != self.saved_version || *project != self.baseline
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_are_named_and_jumped_between() {
        let mut project = 1u32;
        let mut history = History::new(&project, "Import");
        project = 2;
        history.record(&project, "First");
        project = 3;
        history.record(&project, "Second");
        assert_eq!(history.states(), (vec!["Import".into(), "First".into(), "Second".into()], 2));
        assert!(history.go_to(0, &mut project));
        assert_eq!(project, 1);
        assert_eq!(history.states().1, 0);
        assert!(history.go_to(2, &mut project));
        assert_eq!(project, 3);
        assert!(history.has_unsaved_changes(&project));
        history.mark_saved(&project);
        assert!(!history.has_unsaved_changes(&project));
        history.undo(&mut project);
        assert!(history.has_unsaved_changes(&project));
    }

    #[test]
    fn old_steps_are_dropped_past_the_limit() {
        let mut project = 1u32;
        let mut history = History::new(&project, "Import");
        history.limit = 3;
        for value in 2..10 {
            project = value;
            history.record(&project, "Step");
        }
        assert_eq!(history.states().0.len(), 4);
    }

    #[test]
    fn nothing_changed_records_nothing() {
        let project = 5u32;
        let mut history = History::new(&project, "Import");
        assert!(!history.record(&project, "Same"));
        assert!(!history.can_undo(&project));
    }
}

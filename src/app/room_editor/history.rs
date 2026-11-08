//! Undo and redo for edits to the scene's object list.
//!
//! Every edit is stored as the operation that reverts it. Applying a
//! [`SceneEdit`] returns its own inverse, so the same type serves both the
//! undo and the redo stacks.

use crate::scene::SceneObject;

/// Maximum number of undo steps kept in memory.
const MAX_UNDO_STEPS: usize = 256;

/// An operation on the object list.
#[derive(Debug, Clone)]
pub enum SceneEdit {
    /// Replace the object at `index` with `object`.
    Replace { index: usize, object: SceneObject },
    /// Insert `object` at `index`.
    Insert { index: usize, object: SceneObject },
    /// Remove the object at `index`.
    Remove { index: usize },
    /// Several edits undone or redone as one step, applied in order.
    Group(Vec<SceneEdit>),
}

impl SceneEdit {
    /// Applies the edit to `objects` and returns the edit that reverses it.
    ///
    /// Returns `None` if the index is out of range, which can only happen if
    /// the history and the scene got out of sync.
    fn apply(self, objects: &mut Vec<SceneObject>) -> Option<SceneEdit> {
        match self {
            Self::Replace { index, object } => {
                let previous = std::mem::replace(objects.get_mut(index)?, object);
                Some(Self::Replace { index, object: previous })
            }
            Self::Insert { index, object } => {
                if index > objects.len() {
                    return None;
                }
                objects.insert(index, object);
                Some(Self::Remove { index })
            }
            Self::Remove { index } => {
                if index >= objects.len() {
                    return None;
                }
                let object = objects.remove(index);
                Some(Self::Insert { index, object })
            }
            Self::Group(edits) => {
                // The inverse undoes the edits in reverse order.
                let mut inverses = Vec::with_capacity(edits.len());
                for edit in edits {
                    inverses.push(edit.apply(objects)?);
                }
                inverses.reverse();
                Some(Self::Group(inverses))
            }
        }
    }
}

/// Undo and redo stacks for one scene.
#[derive(Debug, Default)]
pub struct EditHistory {
    undo_stack: Vec<SceneEdit>,
    redo_stack: Vec<SceneEdit>,
}

impl EditHistory {
    /// Records `revert`, the edit that undoes a change the user just made.
    pub fn record(&mut self, revert: SceneEdit) {
        self.undo_stack.push(revert);
        if self.undo_stack.len() > MAX_UNDO_STEPS {
            self.undo_stack.remove(0);
        }
        self.redo_stack.clear();
    }

    /// Reverts the most recent change. Returns `true` if anything changed.
    pub fn undo(&mut self, objects: &mut Vec<SceneObject>) -> bool {
        Self::transfer(&mut self.undo_stack, &mut self.redo_stack, objects)
    }

    /// Re-applies the most recently undone change. Returns `true` if anything changed.
    pub fn redo(&mut self, objects: &mut Vec<SceneObject>) -> bool {
        Self::transfer(&mut self.redo_stack, &mut self.undo_stack, objects)
    }

    /// Whether there is anything to undo.
    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    /// Whether there is anything to redo.
    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// Pops an edit from `source`, applies it and pushes its inverse onto `destination`.
    fn transfer(
        source: &mut Vec<SceneEdit>,
        destination: &mut Vec<SceneEdit>,
        objects: &mut Vec<SceneObject>,
    ) -> bool {
        let Some(edit) = source.pop() else {
            return false;
        };
        match edit.apply(objects) {
            Some(inverse) => {
                destination.push(inverse);
                true
            }
            None => {
                log::warn!("discarding edit history that no longer matches the scene");
                source.clear();
                destination.clear();
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{LightData, SceneObjectKind};
    use glam::Vec3;

    fn object(name: &str) -> SceneObject {
        SceneObject {
            name: name.to_string(),
            group_index: 0,
            kind: SceneObjectKind::Light { radius: 1.0, data: LightData::default() },
            position: Vec3::ZERO,
            rotation_degrees: Vec3::ZERO,
            visible: true,
            id: 0,
            next: None,
        }
    }

    #[test]
    fn grouped_edits_undo_together() {
        let mut objects = vec![object("a"), object("b")];
        let mut history = EditHistory::default();

        // Rename both objects as one step.
        let reverts = vec![
            SceneEdit::Replace { index: 0, object: objects[0].clone() },
            SceneEdit::Replace { index: 1, object: objects[1].clone() },
        ];
        objects[0].name = "x".to_string();
        objects[1].name = "y".to_string();
        history.record(SceneEdit::Group(reverts));

        assert!(history.undo(&mut objects));
        assert_eq!((objects[0].name.as_str(), objects[1].name.as_str()), ("a", "b"));
        assert!(!history.can_undo());
        assert!(history.redo(&mut objects));
        assert_eq!((objects[0].name.as_str(), objects[1].name.as_str()), ("x", "y"));
    }

    #[test]
    fn undo_and_redo_a_removal() {
        let mut objects = vec![object("a"), object("b")];
        let mut history = EditHistory::default();

        let removed = objects.remove(0);
        history.record(SceneEdit::Insert { index: 0, object: removed });
        assert_eq!(objects.len(), 1);

        assert!(history.undo(&mut objects));
        assert_eq!(objects[0].name, "a");
        assert!(history.redo(&mut objects));
        assert_eq!(objects.len(), 1);
        assert_eq!(objects[0].name, "b");
    }

    #[test]
    fn undo_a_replacement() {
        let mut objects = vec![object("before")];
        let mut history = EditHistory::default();

        let previous = std::mem::replace(&mut objects[0], object("after"));
        history.record(SceneEdit::Replace { index: 0, object: previous });

        assert!(history.undo(&mut objects));
        assert_eq!(objects[0].name, "before");
        assert!(history.redo(&mut objects));
        assert_eq!(objects[0].name, "after");
    }
}

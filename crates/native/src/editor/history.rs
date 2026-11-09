//! Bounded local undo snapshots include selection. Model echoes leave history
//! intact; external replacements clear it so Undo cannot resurrect stale data.

use parley::{editing::Selection, PlainEditor};
use std::collections::VecDeque;

pub(super) struct Snapshot {
    pub value: String,
    pub selection: Selection,
}

impl Snapshot {
    pub fn capture(buffer: &PlainEditor<()>) -> Self {
        Self {
            value: buffer.raw_text().into(),
            selection: *buffer.raw_selection(),
        }
    }
}

#[derive(Default)]
pub(super) struct History {
    undo: VecDeque<Snapshot>,
    redo: VecDeque<Snapshot>,
}

impl History {
    pub fn clear(&mut self) {
        *self = Self::default();
    }
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn record(&mut self, before: Snapshot) {
        self.redo.clear();
        Self::push(&mut self.undo, before);
    }

    pub fn undo(&mut self, current: Snapshot) -> Option<Snapshot> {
        let previous = self.undo.pop_back()?;
        Self::push(&mut self.redo, current);
        Some(previous)
    }

    pub fn redo(&mut self, current: Snapshot) -> Option<Snapshot> {
        let next = self.redo.pop_back()?;
        Self::push(&mut self.undo, current);
        Some(next)
    }

    fn push(stack: &mut VecDeque<Snapshot>, snapshot: Snapshot) {
        stack.push_back(snapshot);
        // Both the count and retained text bytes are bounded. Large individual
        // edits can be non-undoable in this minimal plain-text editor.
        let mut bytes: usize = stack.iter().map(|entry| entry.value.len()).sum();
        while stack.len() > 128 || bytes > 1024 * 1024 {
            bytes -= stack.pop_front().unwrap().value.len();
        }
    }
}

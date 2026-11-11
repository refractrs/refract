//! A shared command path for keyboard shortcuts and context menus. Commands
//! mutate the editor; only text changes emit the existing portable Input event.

use super::{history::Snapshot, TextEditor};
use crate::{platform::Clipboard, text::TextSystem};
use refract_core::Event;
use winit::keyboard::{Key, ModifiersState};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EditCommand {
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
}

impl EditCommand {
    pub fn label(self) -> &'static str {
        match self {
            Self::Undo => "Undo",
            Self::Redo => "Redo",
            Self::Cut => "Cut",
            Self::Copy => "Copy",
            Self::Paste => "Paste",
            Self::SelectAll => "Select All",
        }
    }

    pub fn shortcut(self) -> &'static str {
        if cfg!(target_os = "macos") {
            match self {
                Self::Undo => "⌘ Z",
                Self::Redo => "⇧⌘ Z",
                Self::Cut => "⌘ X",
                Self::Copy => "⌘ C",
                Self::Paste => "⌘ V",
                Self::SelectAll => "⌘ A",
            }
        } else {
            match self {
                Self::Undo => "Ctrl+Z",
                Self::Redo => "Ctrl+Shift+Z",
                Self::Cut => "Ctrl+X",
                Self::Copy => "Ctrl+C",
                Self::Paste => "Ctrl+V",
                Self::SelectAll => "Ctrl+A",
            }
        }
    }

    pub fn from_key(key: &Key, modifiers: ModifiersState) -> Option<Self> {
        let command = if cfg!(target_os = "macos") {
            modifiers.super_key()
        } else {
            modifiers.control_key()
        };
        if !command || modifiers.alt_key() {
            return None;
        }
        let Key::Character(key) = key else {
            return None;
        };
        match key.to_lowercase().as_str() {
            "z" => Some(if modifiers.shift_key() {
                Self::Redo
            } else {
                Self::Undo
            }),
            "y" if !cfg!(target_os = "macos") => Some(Self::Redo),
            "x" => Some(Self::Cut),
            "c" => Some(Self::Copy),
            "v" => Some(Self::Paste),
            "a" => Some(Self::SelectAll),
            _ => None,
        }
    }
}

impl TextEditor {
    pub fn can_command(&self, command: EditCommand) -> bool {
        if self.buffer.is_composing() {
            return false;
        }
        match command {
            EditCommand::Undo => self.history.can_undo(),
            EditCommand::Redo => self.history.can_redo(),
            EditCommand::Cut | EditCommand::Copy => self.buffer.selected_text().is_some(),
            EditCommand::SelectAll => !self.buffer.raw_text().is_empty(),
            // Clipboard availability is checked on execution, without reading
            // clipboard contents merely to display a menu.
            EditCommand::Paste => true,
        }
    }

    pub fn command(
        &mut self,
        command: EditCommand,
        clipboard: &mut dyn Clipboard,
        text: &mut TextSystem,
    ) -> Option<Event> {
        if !self.can_command(command) {
            return None;
        }
        self.reveal_caret = true;
        let before = Snapshot::capture(&self.buffer);
        match command {
            EditCommand::Undo | EditCommand::Redo => {
                let snapshot = if command == EditCommand::Undo {
                    self.history.undo(before)
                } else {
                    self.history.redo(before)
                }?;
                self.buffer.set_text(&snapshot.value);
                self.buffer
                    .driver(&mut text.fonts, &mut text.context)
                    .set_selection(snapshot.selection);
                return Some(Event::Input(self.buffer.raw_text().into()));
            }
            EditCommand::Cut | EditCommand::Copy => {
                if let Some(value) = self.buffer.selected_text() {
                    if clipboard.write(value.into()) && command == EditCommand::Cut {
                        self.buffer
                            .driver(&mut text.fonts, &mut text.context)
                            .delete_selection();
                    }
                }
            }
            EditCommand::Paste => {
                if let Some(value) = clipboard.read() {
                    let value = self.normalize(&value);
                    self.buffer
                        .driver(&mut text.fonts, &mut text.context)
                        .insert_or_replace_selection(&value);
                }
            }
            EditCommand::SelectAll => self
                .buffer
                .driver(&mut text.fonts, &mut text.context)
                .select_all(),
        }
        self.finish_edit(before)
    }

    pub(super) fn finish_edit(&mut self, before: Snapshot) -> Option<Event> {
        if before.value == self.buffer.raw_text() {
            return None;
        }
        self.history.record(before);
        Some(Event::Input(self.buffer.raw_text().into()))
    }
}

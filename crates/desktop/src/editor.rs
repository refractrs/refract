//! Host-owned editing state. Parley handles grapheme boundaries, selection and
//! composition; only committed strings travel back through an Refract binding.

mod commands;
mod history;
pub(crate) use commands::EditCommand;
use history::{History, Snapshot};

use crate::{
    platform::Clipboard,
    renderer::{DrawCommand, Frame, Point, Rect},
    text::{glyph_runs, TextBlock, TextSystem, Typography},
};
use parley::{PlainEditor, StyleProperty};
use refract_core::{Color, Element, Event};
use winit::{
    event::Ime,
    keyboard::{Key, ModifiersState, NamedKey},
};

pub(crate) struct TextEditor {
    pub buffer: PlainEditor<()>,
    multiline: bool,
    model_value: String,
    pending_value: Option<String>,
    composition: Option<(String, parley::editing::Selection)>,
    typography: Option<Typography>,
    placeholder: Option<TextBlock>,
    width: Option<f32>,
    bounds: Rect,
    offset: Point,
    history: History,
    reveal_caret: bool,
}

impl TextEditor {
    pub fn is_composing(&self) -> bool { self.buffer.is_composing() }

    pub fn new(element: &Element) -> Self {
        let mut buffer = PlainEditor::new(14.0);
        // Match TextBlock's fractional metrics; the renderer applies display
        // scale later. Rounding only editable text shifts its placeholder baseline.
        buffer.set_quantize(false);
        buffer.set_text(&element.value);
        Self {
            buffer,
            multiline: element.multiline,
            model_value: element.value.clone(),
            pending_value: None,
            composition: None,
            typography: None,
            placeholder: None,
            width: None,
            bounds: Rect::default(),
            offset: Point::default(),
            history: History::default(),
            reveal_caret: true,
        }
    }

    pub fn sync(&mut self, element: &Element, typography: &Typography, text: &mut TextSystem) {
        if self.model_value != element.value {
            self.model_value.clone_from(&element.value);
            if self.buffer.is_composing() {
                self.pending_value = Some(element.value.clone());
            } else if self.buffer.raw_text() != element.value {
                self.buffer.set_text(&element.value);
                self.history.clear();
                self.reveal_caret = true;
            }
        }
        if self.typography.as_ref() != Some(typography) {
            let styles = self.buffer.edit_styles();
            styles.insert(StyleProperty::FontSize(typography.size));
            styles.insert(StyleProperty::FontFamily(
                parley::FontFamily::from(typography.family.as_str()).into_owned(),
            ));
            styles.insert(StyleProperty::FontWeight(parley::FontWeight::new(
                typography.weight as f32,
            )));
            styles.insert(StyleProperty::LineHeight(parley::LineHeight::Absolute(
                typography.line_height.unwrap_or(typography.size * 1.5),
            )));
            self.typography = Some(typography.clone());
        }
        // Placeholder and editable text must share line metrics so the baseline
        // does not jump when the first character replaces the placeholder.
        let mut placeholder_typography = typography.clone();
        placeholder_typography.line_height =
            Some(typography.line_height.unwrap_or(typography.size * 1.5));
        let placeholder = element.placeholder.as_deref().unwrap_or("");
        if self
            .placeholder
            .as_ref()
            .is_none_or(|block| !block.matches(placeholder, &placeholder_typography))
        {
            self.placeholder = Some(text.shape(placeholder, &placeholder_typography));
        }
    }

    /// Pointer coordinates are transformed into the editor's scrolled text space.
    pub fn pointer(&mut self, point: Point, extend: bool, text: &mut TextSystem) {
        self.reveal_caret = true;
        let origin = self.text_origin();
        let x = point.x - origin.x;
        let y = point.y - origin.y;
        let mut driver = self.buffer.driver(&mut text.fonts, &mut text.context);
        if extend {
            driver.extend_selection_to_point(x, y);
        } else {
            driver.move_to_point(x, y);
        }
    }

    /// Restore the committed selection when composition is cancelled. Parley's
    /// preedit replaces selected text, so clearing it alone would lose that text.
    fn restore_composition(&mut self, text: &mut TextSystem) {
        if let Some((value, selection)) = self.composition.take() {
            self.buffer.set_text(&value);
            self.buffer
                .driver(&mut text.fonts, &mut text.context)
                .set_selection(selection);
        }
    }

    pub fn cancel_composition(&mut self, text: &mut TextSystem) {
        self.restore_composition(text);
        if let Some(value) = self.pending_value.take() {
            self.buffer.set_text(&value);
            self.history.clear();
        }
    }

    pub fn ime(&mut self, event: &Ime, text: &mut TextSystem) -> Option<Event> {
        self.reveal_caret = true;
        match event {
            Ime::Preedit(value, cursor) => {
                if value.is_empty() {
                    self.restore_composition(text);
                } else {
                    if self.composition.is_none() {
                        self.composition =
                            Some((self.buffer.raw_text().into(), *self.buffer.raw_selection()));
                    }
                    self.buffer
                        .driver(&mut text.fonts, &mut text.context)
                        .set_compose(value, *cursor);
                }
                None
            }
            Ime::Commit(value) => {
                let value = self.normalize(value);
                self.restore_composition(text);
                let before = Snapshot::capture(&self.buffer);
                let mut driver = self.buffer.driver(&mut text.fonts, &mut text.context);
                driver.clear_compose();
                driver.insert_or_replace_selection(&value);
                if let Some(value) = self.pending_value.take() {
                    self.buffer.set_text(&value);
                    self.history.clear();
                    return Some(Event::Input(self.buffer.raw_text().into()));
                }
                self.finish_edit(before)
            }
            Ime::Disabled => {
                self.cancel_composition(text);
                None
            }
            _ => None,
        }
    }

    fn normalize(&self, value: &str) -> String {
        if self.multiline {
            value.replace("\r\n", "\n").replace('\r', "\n")
        } else {
            value.replace("\r\n", "\n").replace(['\r', '\n'], " ")
        }
    }

    pub fn key(
        &mut self,
        key: &Key,
        value: Option<&str>,
        modifiers: ModifiersState,
        clipboard: &mut dyn Clipboard,
        text: &mut TextSystem,
    ) -> Option<Event> {
        if self.buffer.is_composing() {
            return None;
        }
        self.reveal_caret = true;
        if let Some(command) = EditCommand::from_key(key, modifiers) {
            return self.command(command, clipboard, text);
        }
        let command = if cfg!(target_os = "macos") {
            modifiers.super_key()
        } else {
            modifiers.control_key()
        };
        let line = cfg!(target_os = "macos") && command;
        let word = if cfg!(target_os = "macos") {
            modifiers.alt_key()
        } else {
            modifiers.control_key()
        };
        let shift = modifiers.shift_key();
        let before = Snapshot::capture(&self.buffer);
        let normalized = value.map(|v| self.normalize(v));
        let mut driver = self.buffer.driver(&mut text.fonts, &mut text.context);
        match key {
            Key::Named(NamedKey::Enter)
                if !self.multiline || command || modifiers.control_key() =>
            {
                return Some(Event::Submit)
            }
            Key::Named(NamedKey::Enter) => driver.insert_or_replace_selection("\n"),
            Key::Named(NamedKey::Backspace) if word => driver.backdelete_word(),
            Key::Named(NamedKey::Backspace) => driver.backdelete(),
            Key::Named(NamedKey::Delete) if word => driver.delete_word(),
            Key::Named(NamedKey::Delete) => driver.delete(),
            Key::Named(NamedKey::ArrowLeft) => match (shift, word, line) {
                (true, _, true) => driver.select_to_line_start(),
                (false, _, true) => driver.move_to_line_start(),
                (true, true, _) => driver.select_word_left(),
                (false, true, _) => driver.move_word_left(),
                (true, false, _) => driver.select_left(),
                _ => driver.move_left(),
            },
            Key::Named(NamedKey::ArrowRight) => match (shift, word, line) {
                (true, _, true) => driver.select_to_line_end(),
                (false, _, true) => driver.move_to_line_end(),
                (true, true, _) => driver.select_word_right(),
                (false, true, _) => driver.move_word_right(),
                (true, false, _) => driver.select_right(),
                _ => driver.move_right(),
            },
            Key::Named(NamedKey::ArrowUp) => {
                if shift {
                    driver.select_up();
                } else {
                    driver.move_up();
                }
            }
            Key::Named(NamedKey::ArrowDown) => {
                if shift {
                    driver.select_down();
                } else {
                    driver.move_down();
                }
            }
            Key::Named(NamedKey::Home) => {
                if shift {
                    driver.select_to_text_start();
                } else {
                    driver.move_to_text_start();
                }
            }
            Key::Named(NamedKey::End) => {
                if shift {
                    driver.select_to_text_end();
                } else {
                    driver.move_to_text_end();
                }
            }
            _ if !command && !modifiers.control_key() => {
                if let Some(value) = normalized.filter(|v| !v.chars().any(char::is_control)) {
                    driver.insert_or_replace_selection(&value);
                }
            }
            _ => {}
        }
        self.finish_edit(before)
    }

    pub fn paint(&mut self, bounds: Rect, focused: bool, text: &mut TextSystem, frame: &mut Frame) {
        self.bounds = bounds;
        let width = self.multiline.then_some(bounds.width);
        if self.width != width {
            self.width = width;
            self.buffer.set_width(width);
            self.reveal_caret = true;
        }
        self.buffer
            .refresh_layout(&mut text.fonts, &mut text.context);
        // Keep the active caret visible without changing application scroll state.
        if focused && self.reveal_caret {
            self.reveal_caret = false;
            if let Some(caret) = self.buffer.cursor_geometry(1.5) {
                self.offset.x = self
                    .offset
                    .x
                    .min(caret.x0 as f32)
                    .max(caret.x1 as f32 - bounds.width)
                    .max(0.0);
                self.offset.y = self
                    .offset
                    .y
                    .min(caret.y0 as f32)
                    .max(caret.y1 as f32 - bounds.height)
                    .max(0.0);
            }
        }
        let (_, _, max) = self.scroll_geometry();
        self.offset.x = self.offset.x.clamp(0.0, max.x);
        self.offset.y = self.offset.y.clamp(0.0, max.y);
        let origin = self.text_origin();
        frame.commands.push(DrawCommand::PushLayer {
            clip: bounds,
            radius: 0.0.into(),
            opacity: 1.0,
        });
        let rect = |b: parley::BoundingBox| Rect {
            x: origin.x + b.x0 as f32,
            y: origin.y + b.y0 as f32,
            width: b.x1 as f32 - b.x0 as f32,
            height: b.y1 as f32 - b.y0 as f32,
        };
        if focused {
            for (selection, _) in self.buffer.selection_geometry() {
                frame.commands.push(DrawCommand::Fill {
                    rect: rect(selection),
                    radius: 0.0.into(),
                    color: Color::rgb(198, 215, 201),
                });
            }
        }
        let color = self.typography.as_ref().map_or(Color::BLACK, |t| t.color);
        if self.buffer.raw_text().is_empty() {
            if let Some(placeholder) = &mut self.placeholder {
                placeholder.measure(width, refract_core::Alignment::Start);
                for run in placeholder.glyph_runs() {
                    frame.commands.push(DrawCommand::Glyphs {
                        origin,
                        color: Color::rgb(125, 125, 120),
                        run: run.clone(),
                    });
                }
            }
        } else {
            for run in glyph_runs(self.buffer.try_layout().expect("refreshed editor layout")) {
                frame
                    .commands
                    .push(DrawCommand::Glyphs { origin, color, run });
            }
        }
        if focused {
            if let Some(caret) = self.buffer.cursor_geometry(1.5) {
                frame.commands.push(DrawCommand::Fill {
                    rect: rect(caret),
                    radius: 0.0.into(),
                    color,
                });
            }
        }
        frame.commands.push(DrawCommand::PopLayer);
    }

    /// Editor scrolling shares host gesture routing and scrollbar painting.
    /// A wheel/drag must not snap back to the caret on the next expose frame.
    pub fn scroll_geometry(&self) -> (Rect, Point, Point) {
        let max = self
            .buffer
            .try_layout()
            .map_or(Point::default(), |layout| Point {
                x: (layout.full_width() + 2.0 - self.bounds.width).max(0.0),
                y: (layout.height() - self.bounds.height).max(0.0),
            });
        (self.bounds, self.offset, max)
    }

    pub fn set_scroll(&mut self, offset: Point) {
        self.offset = offset;
        self.reveal_caret = false;
    }

    /// One transform for paint, pointer hit testing, selection, and IME geometry.
    /// Center the line box within the already inset content bounds. Oversized
    /// lines retain the normal clipped/scrollable origin; text areas stay at top.
    fn text_origin(&self) -> Point {
        let inset = if self.multiline {
            0.0
        } else {
            self.buffer.try_layout().map_or(0.0, |layout| {
                ((self.bounds.height - layout.height()) / 2.0).max(0.0)
            })
        };
        Point {
            x: self.bounds.x - self.offset.x,
            y: self.bounds.y + inset - self.offset.y,
        }
    }

    pub fn ime_area(&self) -> Rect {
        let b = self.buffer.ime_cursor_area();
        let origin = self.text_origin();
        Rect {
            x: origin.x + b.x0 as f32,
            y: origin.y + b.y0 as f32,
            width: b.x1 as f32 - b.x0 as f32,
            height: b.y1 as f32 - b.y0 as f32,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup(multiline: bool) -> (TextEditor, TextSystem, Element) {
        use refract_core::View;
        let refract_core::Description::Element(element, _, _) =
            refract_core::text("").describe().unwrap()
        else {
            unreachable!()
        };
        let mut element = *element;
        element.kind = refract_core::ElementKind::TextInput;
        element.multiline = multiline;
        element.value = "café 👩‍💻".into();
        let mut editor = TextEditor::new(&element);
        let mut text = TextSystem::default();
        editor.sync(&element, &Typography::default(), &mut text);
        editor
            .buffer
            .driver(&mut text.fonts, &mut text.context)
            .move_to_text_end();
        (editor, text, element)
    }

    #[test]
    fn edits_preserve_unicode_and_bound_values_do_not_reset_selection() {
        let (mut editor, mut text, mut element) = setup(true);
        let message = editor.key(
            &Key::Named(NamedKey::Backspace),
            None,
            ModifiersState::empty(),
            &mut None,
            &mut text,
        );
        assert!(matches!(message, Some(Event::Input(ref value)) if value == "café "));
        element.value = "café ".into();
        editor.sync(&element, &Typography::default(), &mut text);
        editor.key(
            &Key::Named(NamedKey::ArrowLeft),
            None,
            ModifiersState::empty(),
            &mut None,
            &mut text,
        );
        editor.key(
            &Key::Character("!".into()),
            Some("!"),
            ModifiersState::empty(),
            &mut None,
            &mut text,
        );
        assert_eq!(editor.buffer.raw_text(), "café! ");
        editor
            .buffer
            .driver(&mut text.fonts, &mut text.context)
            .select_all();
        editor.key(
            &Key::Character("水".into()),
            Some("水"),
            ModifiersState::empty(),
            &mut None,
            &mut text,
        );
        assert_eq!(editor.buffer.raw_text(), "水");
    }

    #[test]
    fn preedit_is_local_commit_is_an_input_and_external_updates_are_deferred() {
        let (mut editor, mut text, mut element) = setup(true);
        assert!(editor
            .ime(&Ime::Preedit("に".into(), Some((3, 3))), &mut text)
            .is_none());
        assert!(editor.buffer.is_composing());
        assert!(editor
            .key(
                &Key::Named(NamedKey::Enter),
                None,
                ModifiersState::SUPER,
                &mut None,
                &mut text
            )
            .is_none());
        let message = editor.ime(&Ime::Commit("日本".into()), &mut text);
        assert!(matches!(message, Some(Event::Input(ref value)) if value == "café 👩‍💻日本"));
        editor.ime(&Ime::Preedit("語".into(), Some((3, 3))), &mut text);
        element.value = "external".into();
        editor.sync(&element, &Typography::default(), &mut text);
        assert!(editor.buffer.is_composing());
        let message = editor.ime(&Ime::Commit("語".into()), &mut text);
        assert!(matches!(message, Some(Event::Input(ref value)) if value == "external"));
        assert!(!editor.buffer.is_composing());
        editor
            .buffer
            .driver(&mut text.fonts, &mut text.context)
            .select_all();
        editor.ime(&Ime::Preedit("語".into(), Some((3, 3))), &mut text);
        editor.ime(&Ime::Disabled, &mut text);
        assert_eq!(
            editor.buffer.raw_text(),
            "external",
            "cancelling preedit restores the selected committed text"
        );
        assert_eq!(editor.buffer.selected_text(), Some("external"));
    }

    #[test]
    fn multiline_enter_and_submit_have_distinct_semantics() {
        let (mut editor, mut text, _) = setup(true);
        assert!(matches!(
            editor.key(
                &Key::Named(NamedKey::Enter),
                None,
                ModifiersState::empty(),
                &mut None,
                &mut text
            ),
            Some(Event::Input(_))
        ));
        assert!(editor.buffer.raw_text().ends_with('\n'));
        assert!(matches!(
            editor.key(
                &Key::Named(NamedKey::Enter),
                None,
                ModifiersState::CONTROL,
                &mut None,
                &mut text
            ),
            Some(Event::Submit)
        ));
        let (mut editor, mut text, _) = setup(false);
        assert!(matches!(
            editor.key(
                &Key::Named(NamedKey::Enter),
                None,
                ModifiersState::empty(),
                &mut None,
                &mut text
            ),
            Some(Event::Submit)
        ));
        editor.ime(&Ime::Commit("a\r\nb".into()), &mut text);
        assert!(
            editor.buffer.raw_text().ends_with("a  b") || editor.buffer.raw_text().ends_with("a b")
        );
    }

    #[derive(Default)]
    struct MemoryClipboard {
        value: Option<String>,
        fail_write: bool,
    }
    impl Clipboard for MemoryClipboard {
        fn read(&mut self) -> Option<String> {
            self.value.clone()
        }
        fn write(&mut self, value: String) -> bool {
            if self.fail_write {
                return false;
            }
            self.value = Some(value);
            true
        }
    }

    #[test]
    fn commands_preserve_selection_and_echoes_but_external_values_reset_history() {
        let (mut editor, mut text, mut element) = setup(true);
        let original = editor.buffer.raw_text().to_owned();
        let mut clipboard = MemoryClipboard::default();
        editor.command(EditCommand::SelectAll, &mut clipboard, &mut text);
        clipboard.fail_write = true;
        assert!(editor
            .command(EditCommand::Cut, &mut clipboard, &mut text)
            .is_none());
        assert_eq!(editor.buffer.raw_text(), original);
        clipboard.fail_write = false;
        assert!(
            matches!(editor.command(EditCommand::Cut, &mut clipboard, &mut text), Some(Event::Input(v)) if v.is_empty())
        );
        element.value.clear();
        editor.sync(&element, &Typography::default(), &mut text);
        editor.command(EditCommand::Undo, &mut clipboard, &mut text);
        assert_eq!(editor.buffer.raw_text(), original);
        assert_eq!(editor.buffer.selected_text(), Some(original.as_str()));
        editor.command(EditCommand::Redo, &mut clipboard, &mut text);
        assert_eq!(editor.buffer.raw_text(), "");
        editor.command(EditCommand::Paste, &mut clipboard, &mut text);
        assert_eq!(editor.buffer.raw_text(), original);
        assert!(!editor.can_command(EditCommand::Redo));
        element.value = "replacement".into();
        editor.sync(&element, &Typography::default(), &mut text);
        assert!(!editor.can_command(EditCommand::Undo));
    }

    #[test]
    fn composition_is_one_undo_step_and_preedit_never_enters_history() {
        let (mut editor, mut text, _) = setup(true);
        let original = editor.buffer.raw_text().to_owned();
        let mut clipboard = MemoryClipboard::default();
        editor.ime(&Ime::Preedit("に".into(), Some((3, 3))), &mut text);
        assert!(!editor.can_command(EditCommand::Undo));
        editor.ime(&Ime::Preedit("日本".into(), Some((6, 6))), &mut text);
        editor.ime(&Ime::Commit("日本".into()), &mut text);
        editor.command(EditCommand::Undo, &mut clipboard, &mut text);
        assert_eq!(editor.buffer.raw_text(), original);
        assert!(!editor.can_command(EditCommand::Undo));
        editor.command(EditCommand::Redo, &mut clipboard, &mut text);
        assert_eq!(editor.buffer.raw_text(), format!("{original}日本"));
    }

    #[test]
    fn manual_editor_scroll_survives_paint_until_caret_moves() {
        let (mut editor, mut text, _) = setup(true);
        editor.buffer.set_text(&"line\n".repeat(30));
        editor
            .buffer
            .driver(&mut text.fonts, &mut text.context)
            .move_to_text_end();
        let mut frame = Frame {
            commands: vec![],
            background: Color::WHITE,
            scale_factor: 1.0,
        };
        let bounds = Rect {
            x: 0.0,
            y: 0.0,
            width: 200.0,
            height: 80.0,
        };
        editor.paint(bounds, true, &mut text, &mut frame);
        assert!(editor.offset.y > 0.0);
        editor.set_scroll(Point::default());
        editor.paint(bounds, true, &mut text, &mut frame);
        assert_eq!(editor.offset.y, 0.0);
        editor.key(
            &Key::Named(NamedKey::ArrowLeft),
            None,
            ModifiersState::empty(),
            &mut MemoryClipboard::default(),
            &mut text,
        );
        editor.paint(bounds, true, &mut text, &mut frame);
        assert!(editor.offset.y > 0.0);
    }

    #[test]
    fn single_line_centering_keeps_placeholder_caret_selection_and_ime_aligned() {
        let (mut editor, mut text, mut element) = setup(false);
        element.value.clear();
        element.placeholder = Some("Search conversations".into());
        let typography = Typography {
            size: 12.0,
            ..Typography::default()
        };
        editor.sync(&element, &typography, &mut text);
        let mut frame = Frame {
            commands: vec![],
            background: Color::WHITE,
            scale_factor: 2.0,
        };
        let bounds = Rect {
            x: 20.0,
            y: 30.0,
            width: 200.0,
            height: 40.0,
        };
        editor.paint(bounds, true, &mut text, &mut frame);
        let glyph_position = |frame: &Frame| {
            frame
                .commands
                .iter()
                .find_map(|command| {
                    if let DrawCommand::Glyphs { origin, run, .. } = command {
                        Some((origin.x, origin.y, run.glyphs[0].y))
                    } else {
                        None
                    }
                })
                .expect("glyphs")
        };
        let placeholder = glyph_position(&frame);
        let line_height = editor.buffer.try_layout().unwrap().height();
        assert!(line_height < bounds.height);
        assert!((placeholder.1 - (bounds.y + (bounds.height - line_height) / 2.0)).abs() < 0.001);

        element.value = "Search conversations".into();
        editor.sync(&element, &typography, &mut text);
        frame.commands.clear();
        editor.paint(bounds, true, &mut text, &mut frame);
        let value = glyph_position(&frame);
        assert_eq!(
            placeholder, value,
            "typing must preserve the placeholder baseline"
        );
        let caret = editor.buffer.cursor_geometry(1.5).unwrap();
        let area = editor.ime_area();
        assert!((area.y - (value.1 + caret.y0 as f32)).abs() < 0.001);
        assert!(frame.commands.iter().any(|command| matches!(command,
            DrawCommand::Fill { rect, color: Color::BLACK, .. }
            if (rect.y - area.y).abs() < 0.001)));

        // Hit testing uses the same translated coordinates as the rendered line.
        editor.pointer(
            Point {
                x: bounds.x,
                y: bounds.y + bounds.height / 2.0,
            },
            false,
            &mut text,
        );
        editor.pointer(
            Point {
                x: bounds.x + bounds.width,
                y: bounds.y + bounds.height / 2.0,
            },
            true,
            &mut text,
        );
        assert_eq!(editor.buffer.selected_text(), Some("Search conversations"));
        frame.commands.clear();
        editor.paint(bounds, true, &mut text, &mut frame);
        let selection = editor.buffer.selection_geometry()[0].0;
        assert!(frame.commands.iter().any(|command| matches!(command,
            DrawCommand::Fill { rect, color, .. }
            if *color == Color::rgb(198, 215, 201)
                && (rect.y - (value.1 + selection.y0 as f32)).abs() < 0.001)));

        // Height changes recompute centering without reshaping the placeholder.
        let shapes = text.shapes;
        let taller = Rect {
            height: 60.0,
            ..bounds
        };
        frame.commands.clear();
        editor.paint(taller, true, &mut text, &mut frame);
        assert!((glyph_position(&frame).1 - value.1 - 10.0).abs() < 0.001);
        assert_eq!(text.shapes, shapes);
        editor.multiline = true;
        frame.commands.clear();
        editor.paint(taller, false, &mut text, &mut frame);
        assert_eq!(
            glyph_position(&frame).1,
            bounds.y,
            "text areas remain top-aligned"
        );
    }
}

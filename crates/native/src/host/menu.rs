//! The minimal editing context menu is painted by Refract, like GPUI's in-app
//! menus. It owns pointer/keyboard input while open and revalidates its target
//! at execution time. Application menus can later reuse the command boundary.

use super::Host;
use crate::{
    editor::EditCommand,
    platform::Clipboard,
    renderer::{DrawCommand, Frame, Point, Rect},
    text::{TextBlock, Typography},
};
use refract_core::{Alignment, BoxShadow, Color, ElementId, Event};
use winit::keyboard::{Key, ModifiersState, NamedKey};

const PADDING: f32 = 6.0;
const ROW: f32 = 24.0;
const TEXT_INSET: f32 = 10.0;
const COLUMN_GAP: f32 = 24.0;
const RADIUS: f32 = 12.0;
const MENU_COMMANDS: [EditCommand; 5] = [
    EditCommand::Undo,
    EditCommand::Redo,
    EditCommand::Cut,
    EditCommand::Copy,
    EditCommand::Paste,
];

pub(super) struct EditMenu {
    target: ElementId,
    bounds: Rect,
    labels: Vec<TextBlock>,
    shortcuts: Vec<TextBlock>,
    selected: Option<usize>,
}

impl Host {
    pub fn menu_is_open(&self) -> bool {
        self.menu.is_some()
    }

    pub fn close_menu(&mut self) -> bool {
        self.menu.take().is_some()
    }

    pub fn open_edit_menu(&mut self, point: Point, keyboard: bool) -> bool {
        self.menu = None;
        let target = if keyboard {
            self.focus
        } else {
            self.hit_test(point)
        };
        let Some(target) = target.filter(|id| self.is_editor(*id)) else {
            return false;
        };
        if self.entries[&target]
            .editor
            .as_ref()
            .unwrap()
            .buffer
            .is_composing()
        {
            return false;
        }
        if !keyboard {
            self.keyboard_focus = false;
        }
        if self.focus != Some(target) {
            self.set_focus(Some(target));
            self.editor_pointer(point, false);
        }
        self.cancel_scroll();
        self.set_pressed(None);
        self.set_hover(None);
        let typography = Typography {
            size: 13.0,
            weight: 500,
            line_height: Some(18.0),
            ..Default::default()
        };
        let mut labels: Vec<_> = MENU_COMMANDS
            .iter()
            .map(|c| self.text.shape(c.label(), &typography))
            .collect();
        let mut shortcuts: Vec<_> = MENU_COMMANDS
            .iter()
            .map(|c| self.text.shape(c.shortcut(), &typography))
            .collect();
        // Measure both columns so platform shortcut names never collide with labels.
        let label_width = labels
            .iter_mut()
            .map(|label| label.measure(None, Alignment::Start).0)
            .fold(0.0_f32, f32::max);
        let shortcut_width = shortcuts
            .iter_mut()
            .map(|label| label.measure(None, Alignment::Start).0)
            .fold(0.0_f32, f32::max);
        let width = (label_width + shortcut_width + COLUMN_GAP + 2.0 * (PADDING + TEXT_INSET))
            .ceil()
            .max(144.0);
        let height = ROW * MENU_COMMANDS.len() as f32 + PADDING * 2.0;
        let bounds = Rect {
            x: point.x.min((self.viewport.0 - width).max(0.0)).max(0.0),
            y: point.y.min((self.viewport.1 - height).max(0.0)).max(0.0),
            width,
            height,
        };
        self.menu = Some(EditMenu {
            target,
            bounds,
            labels,
            shortcuts,
            selected: None,
        });
        if keyboard {
            self.menu_step(false);
        }
        true
    }

    fn menu_enabled(&self, target: ElementId, index: usize) -> bool {
        self.entries
            .get(&target)
            .filter(|e| !e.element.disabled)
            .and_then(|e| e.editor.as_ref())
            .is_some_and(|editor| editor.can_command(MENU_COMMANDS[index]))
    }

    pub fn menu_pointer(&mut self, point: Point) -> bool {
        let Some(menu) = &self.menu else { return false };
        let selected = menu
            .bounds
            .contains(point)
            .then(|| {
                let row = ((point.y - menu.bounds.y - PADDING) / ROW).floor();
                (0.0..MENU_COMMANDS.len() as f32)
                    .contains(&row)
                    .then_some(row as usize)
            })
            .flatten()
            .filter(|&i| self.menu_enabled(menu.target, i));
        let menu = self.menu.as_mut().unwrap();
        let changed = menu.selected != selected;
        menu.selected = selected;
        changed
    }

    fn menu_step(&mut self, backwards: bool) {
        let Some(menu) = &self.menu else { return };
        let count = MENU_COMMANDS.len();
        let current = menu
            .selected
            .unwrap_or(if backwards { 0 } else { count - 1 });
        let next = (1..=count)
            .map(|n| {
                if backwards {
                    (current + count - n) % count
                } else {
                    (current + n) % count
                }
            })
            .find(|&i| self.menu_enabled(menu.target, i));
        self.menu.as_mut().unwrap().selected = next;
    }

    pub fn menu_click(
        &mut self,
        point: Point,
        clipboard: &mut dyn Clipboard,
    ) -> Option<(ElementId, Event)> {
        self.menu_pointer(point);
        self.execute_menu(clipboard)
    }

    fn execute_menu(&mut self, clipboard: &mut dyn Clipboard) -> Option<(ElementId, Event)> {
        let menu = self.menu.take()?;
        let command = MENU_COMMANDS[menu.selected?];
        self.edit_command(menu.target, command, clipboard)
            .map(|event| (menu.target, event))
    }

    pub fn menu_key(
        &mut self,
        key: &Key,
        modifiers: ModifiersState,
        clipboard: &mut dyn Clipboard,
    ) -> Option<(ElementId, Event)> {
        match key {
            Key::Named(NamedKey::ArrowDown) => self.menu_step(false),
            Key::Named(NamedKey::ArrowUp) => self.menu_step(true),
            Key::Named(NamedKey::Enter | NamedKey::Space) => return self.execute_menu(clipboard),
            Key::Named(NamedKey::Escape | NamedKey::Tab) => {
                self.close_menu();
            }
            _ => {
                if let Some(command) = EditCommand::from_key(key, modifiers) {
                    let target = self.menu.take()?.target;
                    return self
                        .edit_command(target, command, clipboard)
                        .map(|event| (target, event));
                }
            }
        }
        None
    }

    pub fn edit_command(
        &mut self,
        id: ElementId,
        command: EditCommand,
        clipboard: &mut dyn Clipboard,
    ) -> Option<Event> {
        let entry = self.entries.get_mut(&id)?;
        if entry.element.disabled {
            return None;
        }
        entry
            .editor
            .as_mut()?
            .command(command, clipboard, &mut self.text)
    }

    pub(super) fn paint_menu(&mut self, frame: &mut Frame) {
        let Some(menu) = &self.menu else { return };
        if !self.is_editor(menu.target) {
            self.menu = None;
            return;
        }
        let enabled: Vec<_> = (0..MENU_COMMANDS.len())
            .map(|i| self.menu_enabled(menu.target, i))
            .collect();
        let menu = self.menu.as_mut().unwrap();
        let bounds = menu.bounds;
        frame.commands.push(DrawCommand::PushLayer {
            clip: Rect {
                x: 0.0,
                y: 0.0,
                width: self.viewport.0,
                height: self.viewport.1,
            },
            radius: 0.0.into(),
            opacity: 1.0,
        });
        frame.commands.push(DrawCommand::BoxShadow {
            rect: bounds,
            radius: RADIUS.into(),
            shadow: BoxShadow::new(0.0, 5.0, 18.0, Color(0, 0, 0, 85)),
        });
        frame.commands.push(DrawCommand::PushEffects {
            rect: bounds,
            radius: RADIUS.into(),
            blur: 0.0,
            backdrop_blur: 18.0,
            opacity: 1.0,
        });
        frame.commands.push(DrawCommand::Fill {
            rect: bounds,
            radius: RADIUS.into(),
            color: Color(38, 38, 38, 220),
        });
        frame.commands.push(DrawCommand::Stroke {
            rect: bounds,
            radius: RADIUS.into(),
            width: 1.0,
            color: Color(0, 0, 0, 160),
        });
        frame.commands.push(DrawCommand::Stroke {
            rect: Rect {
                x: bounds.x + 1.0,
                y: bounds.y + 1.0,
                width: bounds.width - 2.0,
                height: bounds.height - 2.0,
            },
            radius: (RADIUS - 1.0).into(),
            width: 0.5,
            color: Color(255, 255, 255, 65),
        });
        for (i, (label, shortcut)) in menu.labels.iter_mut().zip(&mut menu.shortcuts).enumerate() {
            let row = Rect {
                x: bounds.x + PADDING,
                y: bounds.y + PADDING + i as f32 * ROW,
                width: bounds.width - 2.0 * PADDING,
                height: ROW,
            };
            if enabled[i] && menu.selected == Some(i) {
                frame.commands.push(DrawCommand::Fill {
                    rect: row,
                    radius: 6.0.into(),
                    // Lighten the resolved backdrop instead of matching its gray.
                    color: Color(255, 255, 255, 28),
                });
            }
            let (_, label_height) = label.measure(None, Alignment::Start);
            let (shortcut_width, shortcut_height) = shortcut.measure(None, Alignment::Start);
            for run in label.glyph_runs() {
                frame.commands.push(DrawCommand::Glyphs {
                    origin: Point {
                        x: row.x + TEXT_INSET,
                        y: row.y + (ROW - label_height) / 2.0,
                    },
                    color: if enabled[i] {
                        Color::rgb(235, 235, 235)
                    } else {
                        Color::rgb(105, 105, 105)
                    },
                    run: run.clone(),
                });
            }
            for run in shortcut.glyph_runs() {
                frame.commands.push(DrawCommand::Glyphs {
                    origin: Point {
                        x: row.x + row.width - TEXT_INSET - shortcut_width,
                        y: row.y + (ROW - shortcut_height) / 2.0,
                    },
                    color: if enabled[i] {
                        Color::rgb(175, 175, 175)
                    } else {
                        Color::rgb(105, 105, 105)
                    },
                    run: run.clone(),
                });
            }
        }
        frame.commands.push(DrawCommand::PopLayer);
        frame.commands.push(DrawCommand::PopLayer);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use refract::prelude::*;
    use refract_core::App;
    use winit::keyboard::Key;

    struct MemoryClipboard;
    impl Clipboard for MemoryClipboard {
        fn read(&mut self) -> Option<String> {
            Some("pasted 🌍".into())
        }
        fn write(&mut self, _: String) -> bool {
            true
        }
    }

    fn setup() -> (App, Host, ElementId, Point) {
        let mut app = App::new(view_fn(|cx| {
            let value = cx.state("value", || "hello".to_string());
            Ok(field(value))
        }));
        let mut host = Host::new().unwrap();
        let pending = app.prepare().unwrap().unwrap();
        host.apply(&pending.update).unwrap();
        app.commit(pending).unwrap();
        host.frame(600.0, 400.0, 1.0, Color::WHITE).unwrap();
        let id = host
            .paint_order
            .iter()
            .copied()
            .find(|id| host.is_editor(*id))
            .unwrap();
        let rect = host.entries[&id].rect;
        let point = Point {
            x: rect.x + 10.0,
            y: rect.y + 10.0,
        };
        (app, host, id, point)
    }

    #[test]
    fn menu_preserves_selection_and_dispatches_the_same_edit_commands() {
        let (mut app, mut host, id, point) = setup();
        host.set_focus(Some(id));
        host.edit_command(id, EditCommand::SelectAll, &mut MemoryClipboard);
        assert!(host.open_edit_menu(point, false));
        assert_eq!(
            host.entries[&id]
                .editor
                .as_ref()
                .unwrap()
                .buffer
                .selected_text(),
            Some("hello")
        );
        let menu = host.menu.as_ref().unwrap();
        let paste = Point {
            x: menu.bounds.x + 20.0,
            y: menu.bounds.y + PADDING + ROW * 4.5,
        };
        let (target, event) = host.menu_click(paste, &mut MemoryClipboard).unwrap();
        assert_eq!(target, id);
        assert!(matches!(&event, Event::Input(v) if v == "pasted 🌍"));
        app.dispatch(id, event);
        let pending = app.prepare().unwrap().unwrap();
        assert_eq!(pending.update.patches.len(), 1);
        host.apply(&pending.update).unwrap();
        app.commit(pending).unwrap();
        let event = host.edit_command(id, EditCommand::Undo, &mut MemoryClipboard);
        assert!(matches!(event, Some(Event::Input(v)) if v == "hello"));
    }

    #[test]
    fn keyboard_skips_disabled_rows_and_dismissal_does_not_touch_graph() {
        let (mut app, mut host, id, point) = setup();
        host.set_focus(Some(id));
        assert!(host.open_edit_menu(point, true));
        assert_eq!(host.menu.as_ref().unwrap().selected, Some(4));
        host.menu_key(
            &Key::Named(NamedKey::ArrowDown),
            ModifiersState::empty(),
            &mut MemoryClipboard,
        );
        assert_eq!(host.menu.as_ref().unwrap().selected, Some(4));
        // Select All remains a keyboard command, but is not a menu row.
        let command = if cfg!(target_os = "macos") {
            ModifiersState::SUPER
        } else {
            ModifiersState::CONTROL
        };
        host.menu_key(&Key::Character("a".into()), command, &mut MemoryClipboard);
        assert!(!host.menu_is_open());
        assert_eq!(
            host.entries[&id]
                .editor
                .as_ref()
                .unwrap()
                .buffer
                .selected_text(),
            Some("hello")
        );
        host.open_edit_menu(point, true);
        assert_eq!(host.menu.as_ref().unwrap().selected, Some(2));
        host.menu_key(
            &Key::Named(NamedKey::ArrowUp),
            ModifiersState::empty(),
            &mut MemoryClipboard,
        );
        assert_eq!(host.menu.as_ref().unwrap().selected, Some(4));
        let stats = host.stats();
        host.frame(600.0, 400.0, 1.0, Color::WHITE).unwrap();
        assert_eq!(host.stats().text_shapes, stats.text_shapes);
        assert_eq!(host.stats().layout_passes, stats.layout_passes);
        host.menu_key(
            &Key::Named(NamedKey::Escape),
            ModifiersState::empty(),
            &mut MemoryClipboard,
        );
        assert!(!host.menu_is_open());
        assert!(app.prepare().unwrap().is_none());
        host.open_edit_menu(point, true);
        assert!(host
            .menu_click(Point::default(), &mut MemoryClipboard)
            .is_none());
        assert!(!host.menu_is_open());
    }

    #[test]
    fn disabled_or_removed_targets_cannot_execute_a_stale_menu() {
        let (_, mut host, id, point) = setup();
        host.set_focus(Some(id));
        host.open_edit_menu(point, true);
        host.entries.get_mut(&id).unwrap().element.disabled = true;
        assert!(host
            .menu_key(
                &Key::Named(NamedKey::Enter),
                ModifiersState::empty(),
                &mut MemoryClipboard
            )
            .is_none());
        assert_eq!(
            host.entries[&id].editor.as_ref().unwrap().buffer.raw_text(),
            "hello"
        );
        host.entries.get_mut(&id).unwrap().element.disabled = false;
        host.open_edit_menu(point, true);
        host.entries.remove(&id);
        assert!(host
            .menu_key(
                &Key::Named(NamedKey::Enter),
                ModifiersState::empty(),
                &mut MemoryClipboard
            )
            .is_none());
    }
}

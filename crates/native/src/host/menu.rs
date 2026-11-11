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
use refract_core::{Alignment, Color, ElementId, Event};
use winit::keyboard::{Key, ModifiersState, NamedKey};

const PADDING: f32 = 6.0;
const ROW: f32 = 28.0;

pub(super) struct EditMenu {
    target: ElementId,
    bounds: Rect,
    labels: Vec<TextBlock>,
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
        let bounds = Rect {
            x: point.x.min((self.viewport.0 - 190.0).max(0.0)).max(0.0),
            y: point
                .y
                .min((self.viewport.1 - (ROW * 6.0 + PADDING * 2.0)).max(0.0))
                .max(0.0),
            width: 190.0,
            height: ROW * 6.0 + PADDING * 2.0,
        };
        let typography = Typography {
            size: 14.0,
            ..Default::default()
        };
        let labels = EditCommand::ALL
            .iter()
            .map(|c| self.text.shape(c.label(), &typography))
            .collect();
        self.menu = Some(EditMenu {
            target,
            bounds,
            labels,
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
            .is_some_and(|editor| editor.can_command(EditCommand::ALL[index]))
    }

    pub fn menu_pointer(&mut self, point: Point) -> bool {
        let Some(menu) = &self.menu else { return false };
        let selected = menu
            .bounds
            .contains(point)
            .then(|| {
                let row = ((point.y - menu.bounds.y - PADDING) / ROW).floor();
                (0.0..6.0).contains(&row).then_some(row as usize)
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
        let current = menu.selected.unwrap_or(if backwards { 0 } else { 5 });
        let next = (1..=6)
            .map(|n| {
                if backwards {
                    (current + 6 - n) % 6
                } else {
                    (current + n) % 6
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
        let command = EditCommand::ALL[menu.selected?];
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
        let enabled: Vec<_> = (0..6).map(|i| self.menu_enabled(menu.target, i)).collect();
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
        frame.commands.push(DrawCommand::Fill {
            rect: bounds,
            radius: 8.0.into(),
            color: Color::rgb(37, 39, 41),
        });
        frame.commands.push(DrawCommand::Stroke {
            rect: bounds,
            radius: 8.0.into(),
            width: 1.0,
            color: Color::rgb(95, 97, 99),
        });
        for (i, label) in menu.labels.iter_mut().enumerate() {
            let row = Rect {
                x: bounds.x + PADDING,
                y: bounds.y + PADDING + i as f32 * ROW,
                width: bounds.width - 2.0 * PADDING,
                height: ROW,
            };
            if enabled[i] && menu.selected == Some(i) {
                frame.commands.push(DrawCommand::Fill {
                    rect: row,
                    radius: 4.0.into(),
                    color: Color::rgb(63, 81, 75),
                });
            }
            label.measure(Some(row.width - 16.0), Alignment::Start);
            for run in label.glyph_runs() {
                frame.commands.push(DrawCommand::Glyphs {
                    origin: Point {
                        x: row.x + 8.0,
                        y: row.y + 3.0,
                    },
                    color: if enabled[i] {
                        Color::rgb(245, 245, 245)
                    } else {
                        Color::rgb(130, 130, 130)
                    },
                    run: run.clone(),
                });
            }
        }
        frame.commands.push(DrawCommand::PopLayer);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use refract::prelude::*;
    use winit::keyboard::Key;
    use refract_core::App;

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
        let stats = host.stats();
        host.menu_key(
            &Key::Named(NamedKey::ArrowDown),
            ModifiersState::empty(),
            &mut MemoryClipboard,
        );
        assert_eq!(host.menu.as_ref().unwrap().selected, Some(5));
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

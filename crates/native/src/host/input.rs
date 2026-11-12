//! Input targets use retained geometry in reverse painter order. Entries can
//! disappear after a commit and before the next paint, so lookups tolerate stale IDs.

use super::Host;
use crate::renderer::Point;
use refract_core::{ElementId, ElementKind, Interaction};

impl Host {
    /// Pointer feedback belongs to the host, not to application graph state.
    pub fn set_hover(&mut self, target: Option<ElementId>) -> bool {
        let target = target.filter(|id| {
            self.entries.get(id).is_some_and(|e| {
                e.element.interaction == Some(Interaction::Activate) && !e.element.disabled
            })
        });
        let changed = self.hover != target;
        self.hover = target;
        changed
    }

    pub fn set_pressed(&mut self, target: Option<ElementId>) -> bool {
        let target = target.filter(|id| {
            self.entries.get(id).is_some_and(|e| {
                e.element.interaction == Some(Interaction::Activate) && !e.element.disabled
            })
        });
        let changed = self.pressed != target;
        self.pressed = target;
        changed
    }

    pub fn hit_test(&self, point: Point) -> Option<ElementId> {
        self.paint_order.iter().rev().copied().find(|id| {
            let Some(e) = self.entries.get(id) else {
                return false;
            };
            (matches!(e.element.interaction, Some(Interaction::Activate))
                || e.element.kind == ElementKind::TextInput
                || e.element.focusable.is_some()
                || e.element.focus_target)
                && !e.element.disabled
                && self.point_inside(*id, point)
        })
    }

    pub(super) fn local_point(&self, id: ElementId, point: Point) -> Option<Point> {
        let (x, y) = self.entries.get(&id)?.transform.inverse()?.point(point.x, point.y);
        Some(Point { x, y })
    }

    pub(super) fn point_inside(&self, id: ElementId, point: Point) -> bool {
        let Some(local) = self.local_point(id, point) else { return false; };
        if !self.entries[&id].rect.contains(local) { return false; }
        let mut cursor = Some(id);
        while let Some(id) = cursor {
            let e = &self.entries[&id];
            if e.element.style.clipped || matches!(e.element.style.layout, refract_core::Layout::Scroll(_)) {
                let Some(local) = self.local_point(id, point) else { return false; };
                if !e.rect.contains(local) { return false; }
            }
            cursor = e.parent;
        }
        point.x >= 0.0 && point.y >= 0.0 && point.x < self.viewport.0 && point.y < self.viewport.1
    }

    pub fn activation(&self, id: ElementId) -> Option<refract_core::Event> {
        let e = self.entries.get(&id)?;
        if e.element.disabled {
            return None;
        }
        match e.element.interaction {
            Some(Interaction::Activate) => Some(refract_core::Event::Press),
            _ => None,
        }
    }

    pub fn focus_next(&mut self, backwards: bool) {
        self.keyboard_focus = true;
        self.cancel_scroll();
        let mut order = self.element_order();
        if backwards {
            order.reverse();
        }
        // A programmatically focused element may not be a tab stop. Continue
        // from its tree position instead of restarting at the window boundary.
        let start = self
            .focus
            .and_then(|id| order.iter().position(|candidate| *candidate == id))
            .map_or(0, |index| index + 1);
        let next = order
            .iter()
            .cycle()
            .skip(start)
            .take(order.len())
            .copied()
            .find(|id| self.entries[id].element.is_focusable());
        self.set_focus(next);
        self.reveal_focus = next.is_some();
    }
}

impl Host {
    pub(super) fn has_focus_within(&self, id: ElementId) -> bool {
        let mut current = self.focus;
        while let Some(target) = current {
            if target == id { return true; }
            current = self.entries.get(&target).and_then(|entry| entry.parent);
        }
        false
    }

    pub fn is_editor(&self, id: ElementId) -> bool {
        self.entries
            .get(&id)
            .is_some_and(|e| e.editor.is_some() && !e.element.disabled)
    }

    /// Pointer focus keeps editing feedback without the keyboard navigation ring.
    pub fn focus_pointer(&mut self, target: Option<ElementId>) {
        self.keyboard_focus = false;
        self.reveal_focus = false;
        self.set_focus(target);
    }

    /// Apply a committed request without changing the user's input modality.
    pub fn request_focus(&mut self, target: Option<ElementId>) {
        if target.is_some_and(|id| self.entries.get(&id).is_none_or(|e| e.element.disabled)) {
            return;
        }
        self.cancel_scroll();
        self.set_focus(target);
        self.reveal_focus = target.is_some();
    }

    pub fn set_focus(&mut self, target: Option<ElementId>) {
        if self.focus != target {
            if let Some(editor) = self
                .focus
                .and_then(|id| self.entries.get_mut(&id))
                .and_then(|e| e.editor.as_mut())
            {
                editor.cancel_composition(&mut self.text);
            }
            if let Some(previous) = self.focus {
                self.input.pending.push((previous, refract_core::Event::Focus(false)));
            }
            self.focus = target;
            if let Some(target) = target {
                self.input.pending.push((target, refract_core::Event::Focus(true)));
            }
        }
    }

    pub fn editor_pointer(&mut self, point: Point, extend: bool) {
        let Some(point) = self.focus.and_then(|id| self.local_point(id, point)) else { return; };
        if let Some(editor) = self
            .focus
            .and_then(|id| self.entries.get_mut(&id))
            .and_then(|e| e.editor.as_mut())
        {
            editor.pointer(point, extend, &mut self.text);
        }
    }

    pub fn editor_key(
        &mut self,
        event: &winit::event::KeyEvent,
        modifiers: winit::keyboard::ModifiersState,
        clipboard: &mut dyn crate::platform::Clipboard,
    ) -> Option<refract_core::Event> {
        let entry = self.entries.get_mut(&self.focus?)?;
        if entry.element.disabled {
            return None;
        }
        entry.editor.as_mut()?.key(
            &event.logical_key,
            event.text.as_deref(),
            modifiers,
            clipboard,
            &mut self.text,
        )
    }

    pub fn editor_ime(&mut self, event: &winit::event::Ime) -> Option<refract_core::Event> {
        let entry = self.entries.get_mut(&self.focus?)?;
        if entry.element.disabled {
            return None;
        }
        entry.editor.as_mut()?.ime(event, &mut self.text)
    }

    pub fn ime_area(&self) -> Option<crate::renderer::Rect> {
        let entry = self.entries.get(&self.focus?)?;
        let r = entry.editor.as_ref()?.ime_area();
        let corners = [(r.x,r.y),(r.x+r.width,r.y),(r.x,r.y+r.height),(r.x+r.width,r.y+r.height)].map(|(x,y)| entry.transform.point(x,y));
        let x = corners.iter().map(|p|p.0).fold(f32::INFINITY,f32::min);
        let y = corners.iter().map(|p|p.1).fold(f32::INFINITY,f32::min);
        Some(crate::renderer::Rect { x, y, width: corners.iter().map(|p|p.0).fold(f32::NEG_INFINITY,f32::max)-x, height: corners.iter().map(|p|p.1).fold(f32::NEG_INFINITY,f32::max)-y })
    }
}

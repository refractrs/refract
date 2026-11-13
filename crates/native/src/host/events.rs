//! Pointer routing and focus notifications independent of the window backend.
use super::Host;
use crate::renderer::Point;
use refract_core::{ElementId, Event, EventResponse, PointerEvent, PointerKind, PointerPhase};
use std::collections::HashMap;

#[derive(Default)]
pub(super) struct InputState {
    pub capture: HashMap<u64, ElementId>,
    pub active: HashMap<u64, PointerEvent>,
    paths: HashMap<u64, Vec<ElementId>>,
    pub pending: Vec<(ElementId, Event)>,
}

impl Host {
    pub fn geometry(&self, id: ElementId) -> Option<refract_core::ElementGeometry> {
        let entry=self.entries.get(&id)?;
        let rect=entry.rect;
        Some(refract_core::ElementGeometry {
            bounds:refract_core::Rect::new(0.0,0.0,rect.width.into(),rect.height.into()),
            transform:entry.transform.then(refract_core::Affine2d([1.0,0.0,0.0,1.0,rect.x.into(),rect.y.into()])),
        })
    }

    pub fn input_hit_test(&self, point: Point) -> Option<ElementId> {
        self.paint_order.iter().rev().copied().find(|id| {
            self.point_inside(*id, point)
        })
    }

    fn input_path(&self, target: Option<ElementId>) -> Vec<ElementId> {
        let mut path = Vec::new();
        let mut current = target;
        while let Some(id) = current {
            let Some(entry) = self.entries.get(&id) else {
                break;
            };
            path.push(id);
            current = entry.parent;
        }
        path
    }

    pub fn pointer_events(&mut self, event: PointerEvent) -> Vec<(ElementId, Event)> {
        let point = Point {
            x: event.position.x as f32,
            y: event.position.y as f32,
        };
        if event.kind == PointerKind::Touch && event.phase == PointerPhase::Down {
            if let Some(target) = self.input_hit_test(point) {
                self.input.capture.insert(event.id, target);
            }
        }
        let captured = self.input.capture.get(&event.id).copied();
        let target = captured.or_else(|| self.input_hit_test(point));
        if event.phase == PointerPhase::Down || self.input.active.contains_key(&event.id) {
            self.input.active.insert(event.id, event);
        }
        let path = if event.phase == PointerPhase::Cancel
            || (event.phase == PointerPhase::Leave && captured.is_none())
        {
            vec![]
        } else {
            self.input_path(target)
        };
        let old = self
            .input
            .paths
            .insert(event.id, path.clone())
            .unwrap_or_default();
        let mut messages = Vec::new();
        if event.phase == PointerPhase::Cancel {
            if let Some(id) = target {
                messages.push((id, Event::Pointer(event)));
            }
        }
        for id in old.iter().filter(|id| !path.contains(id)) {
            messages.push((
                *id,
                Event::Pointer(PointerEvent {
                    phase: PointerPhase::Leave,
                    ..event
                }),
            ));
            if event.kind != PointerKind::Touch {
                messages.push((*id, Event::Hover(false)));
            }
        }
        for id in path.iter().rev().filter(|id| !old.contains(id)) {
            messages.push((
                *id,
                Event::Pointer(PointerEvent {
                    phase: PointerPhase::Enter,
                    ..event
                }),
            ));
            if event.kind != PointerKind::Touch {
                messages.push((*id, Event::Hover(true)));
            }
        }
        if !matches!(
            event.phase,
            PointerPhase::Enter | PointerPhase::Leave | PointerPhase::Cancel
        ) {
            if let Some(id) = target {
                messages.push((id, Event::Pointer(event)));
            }
        }
        messages
    }

    pub fn apply_event_response(&mut self, response: EventResponse) {
        if let Some(request) = response.pointer_capture {
            if request.capture
                && self.input.active.contains_key(&request.pointer_id)
                && self
                    .entries
                    .get(&request.target)
                    .is_some_and(|e| !e.element.disabled)
            {
                self.input
                    .capture
                    .insert(request.pointer_id, request.target);
            } else if !request.capture
                && self.input.capture.get(&request.pointer_id) == Some(&request.target)
            {
                self.input.capture.remove(&request.pointer_id);
            }
        }
    }

    pub fn finish_pointer(&mut self, event: PointerEvent) {
        if event.phase == PointerPhase::Cancel
            || (event.phase == PointerPhase::Up && event.buttons.0 == 0)
        {
            self.input.capture.remove(&event.id);
            self.input.active.remove(&event.id);
            if event.kind == PointerKind::Touch || event.phase == PointerPhase::Cancel {
                for id in self.input.paths.remove(&event.id).unwrap_or_default() {
                    self.input.pending.push((
                        id,
                        Event::Pointer(PointerEvent {
                            phase: PointerPhase::Leave,
                            ..event
                        }),
                    ));
                    if event.kind != PointerKind::Touch {
                        self.input.pending.push((id, Event::Hover(false)));
                    }
                }
            }
        }
    }

    pub fn cancel_pointers(&mut self) -> Vec<(ElementId, Event)> {
        let active: Vec<_> = self.input.active.values().copied().collect();
        let mut messages = Vec::new();
        for pointer in active {
            let event = PointerEvent {
                phase: PointerPhase::Cancel,
                ..pointer
            };
            messages.extend(self.pointer_events(event));
            self.finish_pointer(event);
        }
        messages.extend(self.take_input_events());
        messages
    }

    pub fn refresh_mouse_boundary(&mut self, position: Point, inside: bool) {
        let event = PointerEvent {
            id: 0,
            kind: PointerKind::Mouse,
            phase: if inside {
                PointerPhase::Enter
            } else {
                PointerPhase::Leave
            },
            position: refract_core::EventPoint {
                x: position.x.into(),
                y: position.y.into(),
            },
            button: None,
            buttons: self
                .input
                .active
                .get(&0)
                .map_or(refract_core::PointerButtons::default(), |e| e.buttons),
            modifiers: self
                .input
                .active
                .get(&0)
                .map_or(refract_core::Modifiers::default(), |e| e.modifiers),
            pressure: 0.0,
        };
        let messages = self.pointer_events(event);
        self.input.pending.extend(messages);
    }

    pub fn take_input_events(&mut self) -> Vec<(ElementId, Event)> {
        std::mem::take(&mut self.input.pending)
    }

    pub fn editor_composing(&self) -> bool {
        self.focus
            .and_then(|id| self.entries.get(&id))
            .and_then(|e| e.editor.as_ref())
            .is_some_and(|e| e.is_composing())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use refract_core::{prelude::*, App, Color};
    use std::{cell::RefCell, rc::Rc};

    #[test]
    fn captured_pointer_moves_outside_and_releases_after_up_or_window_cancel() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let received = log.clone();
        let mut app = App::new(view().width(100.0).height(80.0).focusable(true).on_pointer(
            move |cx, e| {
                received.borrow_mut().push(e.phase);
                if e.phase == PointerPhase::Down {
                    cx.capture_pointer();
                    cx.prevent_default();
                }
            },
        ));
        let mut host = Host::new().unwrap();
        let pending = app.prepare().unwrap().unwrap();
        host.apply(&pending.update).unwrap();
        app.commit(pending).unwrap();
        host.frame(300.0, 200.0, 1.0, Color::WHITE).unwrap();
        let id = host.roots[0];
        let rect = host.entries[&id].rect;
        let mut e = PointerEvent {
            id: 0,
            kind: PointerKind::Mouse,
            phase: PointerPhase::Down,
            position: EventPoint {
                x: f64::from(rect.x + 10.0),
                y: f64::from(rect.y + 10.0),
            },
            button: Some(PointerButton::Primary),
            buttons: PointerButtons(1),
            modifiers: Modifiers::default(),
            pressure: 0.5,
        };
        for (id, event) in host.pointer_events(e) {
            host.apply_event_response(app.dispatch_event(id, event));
        }
        assert_eq!(host.input.capture.get(&0), Some(&id));
        e.phase = PointerPhase::Move;
        e.position = EventPoint { x: 900.0, y: 900.0 };
        let messages = host.pointer_events(e);
        assert!(messages.iter().any(|(target, event)| *target == id
            && matches!(event, Event::Pointer(p) if p.phase == PointerPhase::Move)));
        for (target, event) in messages {
            app.dispatch(target, event);
        }
        e.phase = PointerPhase::Up;
        e.buttons = PointerButtons(0);
        for (target, event) in host.pointer_events(e) {
            app.dispatch(target, event);
        }
        host.finish_pointer(e);
        assert!(host.input.capture.is_empty());
        assert!(log.borrow().ends_with(&[
            PointerPhase::Down,
            PointerPhase::Move,
            PointerPhase::Up
        ]));
        e.phase = PointerPhase::Down;
        e.buttons = PointerButtons(1);
        e.position = EventPoint {
            x: f64::from(rect.x + 10.0),
            y: f64::from(rect.y + 10.0),
        };
        for (target, event) in host.pointer_events(e) {
            host.apply_event_response(app.dispatch_event(target, event));
        }
        let cancelled = host.cancel_pointers();
        assert!(cancelled.iter().any(|(target, event)| *target == id
            && matches!(event, Event::Pointer(p) if p.phase == PointerPhase::Cancel)));
        assert!(host.input.capture.is_empty());
        assert!(host.input.active.is_empty());
    }

    #[test]
    fn custom_focusable_views_participate_in_tab_order_and_report_transitions() {
        let mut app = App::new(
            refract_core::v_stack()
                .child(refract_core::text("first").focusable(true))
                .child(refract_core::text("skipped").focusable(false))
                .child(
                    refract_core::text("disabled")
                        .focusable(true)
                        .disabled(true),
                )
                .child(refract_core::text("last").focusable(true)),
        );
        let mut host = Host::new().unwrap();
        let pending = app.prepare().unwrap().unwrap();
        host.apply(&pending.update).unwrap();
        app.commit(pending).unwrap();
        host.frame(300.0, 200.0, 1.0, Color::WHITE).unwrap();
        host.focus_next(false);
        let first = host.focus.unwrap();
        assert_eq!(host.entries[&first].element.text, "first");
        host.focus_next(false);
        let last = host.focus.unwrap();
        assert_eq!(host.entries[&last].element.text, "last");
        let notifications = host.take_input_events();
        assert_eq!(notifications.len(), 3);
        assert!(matches!(notifications[0], (id, Event::Focus(true)) if id == first));
        assert!(matches!(notifications[1], (id, Event::Focus(false)) if id == first));
        assert!(matches!(notifications[2], (id, Event::Focus(true)) if id == last));
        host.focus_next(true);
        assert_eq!(host.focus, Some(first));
    }
    #[test]
    fn hover_tracks_subtree_boundaries_and_touch_has_no_hover() {
        let mut app = App::new(
            v_stack()
                .width(200.0)
                .child(view().label("first").height(40.0).fill_width())
                .child(view().label("second").height(40.0).fill_width()),
        );
        let mut host = Host::new().unwrap();
        let pending = app.prepare().unwrap().unwrap();
        host.apply(&pending.update).unwrap();
        app.commit(pending).unwrap();
        host.frame(300.0, 200.0, 1.0, Color::WHITE).unwrap();
        let root = host.roots[0];
        let first = host.entries[&root].children[0];
        let second = host.entries[&root].children[1];
        let point = |host: &Host, id| {
            let r = host.entries[&id].rect;
            EventPoint {
                x: (r.x + 5.0).into(),
                y: (r.y + 5.0).into(),
            }
        };
        let mut e = PointerEvent {
            id: 0,
            kind: PointerKind::Mouse,
            phase: PointerPhase::Move,
            position: point(&host, first),
            button: None,
            buttons: PointerButtons(0),
            modifiers: Modifiers::default(),
            pressure: 0.0,
        };
        let hover = |messages: Vec<(ElementId, Event)>| {
            messages
                .into_iter()
                .filter_map(|(id, event)| {
                    if let Event::Hover(inside) = event {
                        Some((id, inside))
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(hover(host.pointer_events(e)), [(root, true), (first, true)]);
        e.position = point(&host, second);
        assert_eq!(
            hover(host.pointer_events(e)),
            [(first, false), (second, true)]
        );
        e.phase = PointerPhase::Leave;
        assert_eq!(
            hover(host.pointer_events(e)),
            [(second, false), (root, false)]
        );
        e.id = 9;
        e.kind = PointerKind::Touch;
        e.phase = PointerPhase::Down;
        e.buttons = PointerButtons(1);
        assert!(hover(host.pointer_events(e)).is_empty());
        assert_eq!(host.input.capture.get(&9), Some(&second));
        e.phase = PointerPhase::Cancel;
        let messages = host.pointer_events(e);
        assert!(
            matches!(messages[0], (id, Event::Pointer(p)) if id == second && p.phase == PointerPhase::Cancel)
        );
        host.finish_pointer(e);
        assert!(!host.input.capture.contains_key(&9));
    }
}

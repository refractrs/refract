//! Translate Winit input before native editor, focus, and activation defaults.
use super::*;
use refract_core::{
    Event, EventPoint, Key as LogicalKey, KeyEvent, KeyState, Modifiers, PointerButton,
    PointerButtons, PointerEvent, PointerKind, PointerPhase,
};

impl NativeApp {
    fn event_modifiers(&self) -> Modifiers {
        Modifiers {
            shift: self.modifiers.shift_key(),
            control: self.modifiers.control_key(),
            alt: self.modifiers.alt_key(),
            meta: self.modifiers.super_key(),
        }
    }

    fn send_pointer(&mut self, pointer: PointerEvent) -> bool {
        let mut prevented = false;
        for (id, event) in self.host.pointer_events(pointer) {
            let original = matches!(&event, Event::Pointer(e) if e.phase == pointer.phase);
            let response = self.app.dispatch_event_with_geometry(id, event, |id|self.host.geometry(id));
            self.host.apply_event_response(response);
            if original {
                prevented |= response.default_prevented;
            }
        }
        self.host.finish_pointer(pointer);
        self.app.observe_focus(self.host.focus);
        for (id, event) in self.host.take_input_events() {
            if !matches!(event, Event::Focus(_)) {
                self.app.dispatch(id, event);
            }
        }
        prevented
    }

    pub(super) fn input_event(&mut self, event: &WindowEvent, scale: f64) -> bool {
        if self.host.menu_is_open() && !matches!(event, WindowEvent::Focused(_)) {
            return false;
        }
        let mouse = |this: &Self, phase, button| PointerEvent {
            id: 0,
            kind: PointerKind::Mouse,
            phase,
            position: EventPoint {
                x: f64::from(this.cursor.x),
                y: f64::from(this.cursor.y),
            },
            button,
            buttons: this.pointer_buttons,
            modifiers: this.event_modifiers(),
            pressure: if this.pointer_buttons.0 == 0 {
                0.0
            } else {
                0.5
            },
        };
        match event {
            WindowEvent::KeyboardInput { event, .. } => {
                let Some(id) = self.host.focus else {
                    return false;
                };
                let key = match &event.logical_key {
                    Key::Character(value) if value == " " => LogicalKey::Space,
                    Key::Character(value) => LogicalKey::Character(value.to_string()),
                    Key::Named(key) => LogicalKey::from_name(&format!("{key:?}")),
                    Key::Dead(value) => LogicalKey::Named(format!("Dead({value:?})")),
                    Key::Unidentified(_) => LogicalKey::Named("Unidentified".into()),
                };
                let code = match event.physical_key {
                    winit::keyboard::PhysicalKey::Code(code) => format!("{code:?}"),
                    _ => "Unidentified".into(),
                };
                self.app
                    .dispatch_event(
                        id,
                        Event::Key(KeyEvent {
                            key,
                            code,
                            state: if event.state == ElementState::Pressed {
                                KeyState::Down
                            } else {
                                KeyState::Up
                            },
                            modifiers: self.event_modifiers(),
                            repeat: event.repeat,
                            is_composing: self.host.editor_composing(),
                        }),
                    )
                    .default_prevented
            }
            WindowEvent::CursorMoved { position, .. } => {
                let p = position.to_logical::<f32>(scale);
                self.cursor = Point { x: p.x, y: p.y };
                self.cursor_inside = true;
                self.send_pointer(mouse(self, PointerPhase::Move, None))
            }
            WindowEvent::CursorLeft { .. } => {
                self.send_pointer(mouse(self, PointerPhase::Leave, None));
                false
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let button = match button {
                    MouseButton::Left => PointerButton::Primary,
                    MouseButton::Right => PointerButton::Secondary,
                    MouseButton::Middle => PointerButton::Middle,
                    MouseButton::Back => PointerButton::Back,
                    MouseButton::Forward => PointerButton::Forward,
                    MouseButton::Other(n) => PointerButton::Other(*n),
                };
                let phase = if *state == ElementState::Pressed {
                    self.pointer_buttons.0 |= button.mask();
                    PointerPhase::Down
                } else {
                    self.pointer_buttons.0 &= !button.mask();
                    PointerPhase::Up
                };
                let prevented = self.send_pointer(mouse(self, phase, Some(button)));
                if prevented && button == PointerButton::Primary {
                    self.pressed = None;
                    self.needs_frame |= self.host.set_pressed(None);
                }
                prevented
            }
            WindowEvent::Touch(touch) => {
                let p = touch.location.to_logical::<f64>(scale);
                let point = Point {
                    x: p.x as f32,
                    y: p.y as f32,
                };
                let phase = match touch.phase {
                    winit::event::TouchPhase::Started => PointerPhase::Down,
                    winit::event::TouchPhase::Moved => PointerPhase::Move,
                    winit::event::TouchPhase::Ended => PointerPhase::Up,
                    winit::event::TouchPhase::Cancelled => PointerPhase::Cancel,
                };
                let prevented = self.send_pointer(PointerEvent {
                    id: touch.id.wrapping_add(1),
                    kind: PointerKind::Touch,
                    phase,
                    position: EventPoint { x: p.x, y: p.y },
                    button: Some(PointerButton::Primary),
                    buttons: PointerButtons(
                        if matches!(phase, PointerPhase::Down | PointerPhase::Move) {
                            1
                        } else {
                            0
                        },
                    ),
                    pressure: touch.force.map_or(0.5, |f| f.normalized() as f32),
                    modifiers: self.event_modifiers(),
                });
                match phase {
                    PointerPhase::Down if !prevented => {
                        let target = self.host.hit_test(point);
                        self.touch_presses.insert(touch.id, (target, point));
                        self.host.focus_pointer(target);
                    }
                    PointerPhase::Move => {
                        if self.touch_presses.get(&touch.id).is_some_and(|(_, start)| {
                            (point.x - start.x).hypot(point.y - start.y) > 8.0
                        }) {
                            self.touch_presses.remove(&touch.id);
                        }
                    }
                    PointerPhase::Up => {
                        if let Some((Some(id), _)) = self.touch_presses.remove(&touch.id) {
                            if !prevented && self.host.hit_test(point) == Some(id) {
                                self.activate(id);
                            }
                        }
                    }
                    PointerPhase::Cancel => {
                        self.touch_presses.remove(&touch.id);
                    }
                    _ => {}
                }
                self.needs_frame = true;
                true
            }
            WindowEvent::Focused(false) => {
                for (id, event) in self.host.cancel_pointers() {
                    self.app.dispatch(id, event);
                }
                self.send_pointer(mouse(self, PointerPhase::Leave, None));
                self.pointer_buttons = PointerButtons::default();
                self.touch_presses.clear();
                false
            }
            _ => false,
        }
    }
}

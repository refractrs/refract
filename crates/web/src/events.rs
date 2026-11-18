//! Browser-native event translation and synchronous default/capture controls.
use super::*;
use refract_core::{
    EventPoint, EventResponse, Key, KeyEvent, KeyState, Modifiers, PointerButton, PointerButtons,
    PointerEvent, PointerKind, PointerPhase,
};

fn owner(inner: &Inner, event: &web_sys::Event) -> Option<ElementId> {
    let dom = inner.dom.try_borrow().ok()?;
    for target in event.composed_path().iter() {
        let Ok(node) = target.dyn_into::<HtmlElement>() else {
            continue;
        };
        let Some(id) = node
            .get_attribute("data-refract-id")
            .and_then(|v| v.parse().ok())
            .map(ElementId)
        else {
            continue;
        };
        if dom.entries.get(&id).is_some_and(|entry| entry.node == node) {
            return Some(id);
        }
    }
    None
}

pub(super) fn send(inner: &Rc<Inner>, id: ElementId, event: UiEvent) -> EventResponse {
    let Ok(mut app) = inner.app.try_borrow_mut() else {
        // Focus/capture notifications can be reentrant during a DOM patch.
        if inner.queue.borrow().len() < 1024 {
            inner.queue.borrow_mut().push_back(Queued::Event(id, event));
            schedule(inner);
        }
        return EventResponse::default();
    };
    let Some(app) = app.as_mut() else {
        return EventResponse::default();
    };
    // Preserve FIFO with model updates already admitted before this native event.
    for _ in 0..1024 {
        let queued = inner.queue.borrow_mut().pop_front();
        match queued {
            Some(Queued::Mutation(scope, f)) => {
                if scope.is_none_or(|scope| scope.is_alive()) {
                    app.mutate(f);
                }
            }
            Some(Queued::Event(id, event)) => {
                let point = match &event {
                    UiEvent::Pointer(pointer) => Some(pointer.position),
                    _ => None,
                };
                app.dispatch_event_with_geometry(id, event, |id| {
                    inner.dom.borrow().geometry_at(id, point)
                });
            }
            Some(Queued::Focus(target)) => app.observe_focus(target),
            None => break,
        }
    }
    let point = match &event {
        UiEvent::Pointer(pointer) => Some(pointer.position),
        _ => None,
    };
    app.dispatch_event_with_geometry(id, event, |id| inner.dom.borrow().geometry_at(id, point))
}

fn apply(inner: &Rc<Inner>, native: &web_sys::Event, response: EventResponse) {
    if response.default_prevented {
        native.prevent_default();
    }
    if response.propagation_stopped {
        native.stop_propagation();
    }
    if let Some(request) = response.pointer_capture {
        let node = inner
            .dom
            .borrow()
            .entries
            .get(&request.target)
            .filter(|e| !e.element.disabled)
            .map(|e| e.node.clone());
        if let Some(node) = node {
            let id = request.pointer_id as i32;
            if request.capture {
                if node.set_pointer_capture(id).is_ok() {
                    inner
                        .captures
                        .borrow_mut()
                        .insert(request.pointer_id, request.target);
                }
            } else if inner.captures.borrow().get(&request.pointer_id) == Some(&request.target) {
                inner.captures.borrow_mut().remove(&request.pointer_id);
                let _ = node.release_pointer_capture(id);
            }
        }
    }
}

pub(super) fn dispatch(inner: &Rc<Inner>, native: &web_sys::Event) -> bool {
    if native.type_() == "contextmenu" {
        // Browsers dispatch this separately from pointerdown, sometimes after
        // pointerup. Honor an application-owned secondary click without
        // disabling ordinary field or keyboard-triggered browser menus.
        let suppressed = inner.suppressed_context_menu.replace(false);
        let pointer_triggered = native
            .dyn_ref::<web_sys::MouseEvent>()
            .is_some_and(|event| event.button() == 2 || event.ctrl_key());
        if suppressed && pointer_triggered {
            native.prevent_default();
        }
        return true;
    }
    // Typing in a pointer-focused editor must not promote its editing cue to
    // a navigation ring. Tab is the explicit switch back to keyboard navigation.
    if native.type_() == "pointerdown" {
        inner.suppressed_context_menu.set(false);
        let _ = inner
            .dom
            .borrow()
            .root
            .set_attribute("data-refract-input", "pointer");
    }
    let Some(id) = owner(inner, native) else {
        return true;
    };
    let name = native.type_();
    if name == "select" {
        publish_selection(inner, id);
        return true;
    }
    if matches!(name.as_str(), "keydown" | "keyup") {
        let Some(key) = native.dyn_ref::<web_sys::KeyboardEvent>() else {
            return true;
        };
        publish_selection(inner, id);
        let event = KeyEvent {
            key: Key::from_name(&key.key()),
            code: key.code(),
            state: if name == "keydown" {
                KeyState::Down
            } else {
                KeyState::Up
            },
            modifiers: Modifiers {
                shift: key.shift_key(),
                control: key.ctrl_key(),
                alt: key.alt_key(),
                meta: key.meta_key(),
            },
            repeat: key.repeat(),
            is_composing: key.is_composing() || inner.dom.borrow().entries[&id].composing,
        };
        let response = send(inner, id, UiEvent::Key(event));
        apply(inner, native, response);
        return name == "keyup" || response.default_prevented;
    }
    if matches!(name.as_str(), "focusin" | "focusout") {
        let target = if name == "focusin" {
            Some(id)
        } else {
            let related = native
                .dyn_ref::<web_sys::FocusEvent>()
                .and_then(|e| e.related_target())
                .and_then(|node| node.dyn_into::<web_sys::Element>().ok());
            let dom = inner.dom.borrow();
            // OS/window deactivation retains the logical target.
            if related.is_none() && !dom.document.has_focus().unwrap_or(true) {
                return true;
            }
            related.as_ref().and_then(|node| dom.owned_id(node))
        };
        if let Ok(mut app) = inner.app.try_borrow_mut() {
            if let Some(app) = app.as_mut() {
                app.observe_focus(target);
            }
        } else {
            inner.queue.borrow_mut().push_back(Queued::Focus(target));
            schedule(inner);
        }
        return true;
    }
    if name.starts_with("pointer") || name == "lostpointercapture" {
        let Some(pointer) = native.dyn_ref::<web_sys::PointerEvent>() else {
            return true;
        };
        let pointer_id = pointer.pointer_id() as u64;
        let phase = match name.as_str() {
            "pointerdown" => PointerPhase::Down,
            "pointermove" => PointerPhase::Move,
            "pointerup" => PointerPhase::Up,
            "pointercancel" => PointerPhase::Cancel,
            "pointerenter" => PointerPhase::Enter,
            "pointerleave" => PointerPhase::Leave,
            "lostpointercapture" => {
                // A transfer also releases the old owner; it must not cancel
                // capture that has already moved to another element.
                if inner.captures.borrow().get(&pointer_id) != Some(&id) {
                    return true;
                }
                inner.captures.borrow_mut().remove(&pointer_id);
                PointerPhase::Cancel
            }
            _ => return true,
        };
        let root = inner.dom.borrow().root.get_bounding_client_rect();
        let event = PointerEvent {
            id: pointer_id,
            phase,
            kind: match pointer.pointer_type().as_str() {
                "mouse" => PointerKind::Mouse,
                "touch" => PointerKind::Touch,
                "pen" => PointerKind::Pen,
                _ => PointerKind::Unknown,
            },
            position: EventPoint {
                x: f64::from(pointer.client_x()) - root.x(),
                y: f64::from(pointer.client_y()) - root.y(),
            },
            button: match pointer.button() {
                -1 => None,
                0 => Some(PointerButton::Primary),
                1 => Some(PointerButton::Middle),
                2 => Some(PointerButton::Secondary),
                3 => Some(PointerButton::Back),
                4 => Some(PointerButton::Forward),
                n => Some(PointerButton::Other(n as u16)),
            },
            buttons: PointerButtons(pointer.buttons()),
            pressure: pointer.pressure(),
            modifiers: Modifiers {
                shift: pointer.shift_key(),
                control: pointer.ctrl_key(),
                alt: pointer.alt_key(),
                meta: pointer.meta_key(),
            },
        };
        if phase == PointerPhase::Down
            && (event.button == Some(PointerButton::Secondary)
                || (event.button == Some(PointerButton::Primary) && event.modifiers.control))
        {
            publish_selection(inner, id);
        }
        let response = send(inner, id, UiEvent::Pointer(event));
        apply(inner, native, response);
        if phase == PointerPhase::Down {
            let secondary = event.button == Some(PointerButton::Secondary)
                || (event.button == Some(PointerButton::Primary) && event.modifiers.control);
            inner
                .suppressed_context_menu
                .set(secondary && response.default_prevented);
        } else if phase == PointerPhase::Cancel {
            inner.suppressed_context_menu.set(false);
        }
        if phase == PointerPhase::Down && response.default_prevented {
            inner
                .suppressed_clicks
                .borrow_mut()
                .insert(pointer_id, Rc::new(()));
        } else if phase == PointerPhase::Down {
            inner.suppressed_clicks.borrow_mut().remove(&pointer_id);
        }
        if matches!(phase, PointerPhase::Enter | PointerPhase::Leave)
            && event.kind != PointerKind::Touch
        {
            send(inner, id, UiEvent::Hover(phase == PointerPhase::Enter));
        }
        if matches!(phase, PointerPhase::Up | PointerPhase::Cancel) {
            let owner = inner.captures.borrow_mut().remove(&pointer_id);
            if let Some(node) =
                owner.and_then(|id| inner.dom.borrow().entries.get(&id).map(|e| e.node.clone()))
            {
                let _ = node.release_pointer_capture(pointer.pointer_id());
            }
            if phase == PointerPhase::Cancel {
                inner.suppressed_clicks.borrow_mut().remove(&pointer_id);
            } else {
                // The native click follows pointerup before the next timer task.
                // Expire suppression even when a drag produces no click at all.
                let token = inner.suppressed_clicks.borrow().get(&pointer_id).cloned();
                if let (Some(token), Some(window)) = (token, web_sys::window()) {
                    let weak = Rc::downgrade(inner);
                    let callback = Closure::once_into_js(move || {
                        if let Some(inner) = weak.upgrade() {
                            let matches = inner
                                .suppressed_clicks
                                .borrow()
                                .get(&pointer_id)
                                .is_some_and(|current| Rc::ptr_eq(current, &token));
                            if matches {
                                inner.suppressed_clicks.borrow_mut().remove(&pointer_id);
                            }
                        }
                    });
                    let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(
                        callback.unchecked_ref(),
                        0,
                    );
                }
            }
        }
        return true;
    }
    if name == "click" {
        if let Some(pointer) = native.dyn_ref::<web_sys::PointerEvent>() {
            if inner
                .suppressed_clicks
                .borrow_mut()
                .remove(&(pointer.pointer_id() as u64))
                .is_some()
            {
                native.prevent_default();
                return true;
            }
        }
    }
    if name == "wheel" {
        let Some(wheel) = native.dyn_ref::<web_sys::WheelEvent>() else {
            return true;
        };
        let root = inner.dom.borrow().root.get_bounding_client_rect();
        let event = refract_core::WheelEvent {
            position: EventPoint {
                x: f64::from(wheel.client_x()) - root.left(),
                y: f64::from(wheel.client_y()) - root.top(),
            },
            delta: refract_core::ScrollOffset {
                x: wheel.delta_x(),
                y: wheel.delta_y(),
            },
            unit: match wheel.delta_mode() {
                1 => refract_core::WheelUnit::Line,
                2 => refract_core::WheelUnit::Page,
                _ => refract_core::WheelUnit::Pixel,
            },
            phase: None,
            modifiers: Modifiers {
                shift: wheel.shift_key(),
                control: wheel.ctrl_key(),
                alt: wheel.alt_key(),
                meta: wheel.meta_key(),
            },
        };
        let response = send(inner, id, UiEvent::Wheel(event));
        apply(inner, native, response);
        return true;
    }
    if matches!(name.as_str(), "copy" | "cut" | "paste") {
        let Some(clipboard) = native.dyn_ref::<web_sys::ClipboardEvent>() else {
            return true;
        };
        let Some(data) = clipboard.clipboard_data() else {
            return true;
        };
        publish_selection(inner, id);
        let command = match name.as_str() {
            "copy" => refract_core::ClipboardCommand::Copy,
            "cut" => refract_core::ClipboardCommand::Cut,
            _ => refract_core::ClipboardCommand::Paste,
        };
        let content = (command == refract_core::ClipboardCommand::Paste).then(|| {
            refract_core::ClipboardContent {
                text: data.get_data("text/plain").unwrap_or_default(),
                html: data
                    .get_data("text/html")
                    .ok()
                    .filter(|html| !html.is_empty()),
            }
        });
        let event = refract_core::ClipboardEvent::new(command, content);
        let response = send(inner, id, UiEvent::Clipboard(event.clone()));
        if response.default_prevented && command != refract_core::ClipboardCommand::Paste {
            if let Some(content) = event.content() {
                let _ = data.set_data("text/plain", &content.text);
                if let Some(html) = content.html {
                    let _ = data.set_data("text/html", &html);
                }
            }
        }
        apply(inner, native, response);
        return true;
    }
    false
}

// Observe the host caret before commands without echoing an older selection
// back into the field during the resulting model update.
pub(super) fn publish_selection(inner: &Rc<Inner>, id: ElementId) {
    let selection = {
        let Ok(mut dom) = inner.dom.try_borrow_mut() else {
            return;
        };
        let Some(entry) = dom.entries.get_mut(&id) else {
            return;
        };
        if entry.composing || entry.element.disabled {
            return;
        }
        let Some(input) = &mut entry.element.input else {
            return;
        };
        let selection = super::input_selection(&entry.node);
        if selection.is_none() || selection == input.selection {
            return;
        }
        if let Some(selection) = selection {
            input.selection = Some(selection);
        }
        selection
    };
    if let Some(selection) = selection {
        send(inner, id, UiEvent::TextSelection(selection));
    }
}

/// Observe only the focused control belonging to this mount. Document selection
/// notifications can originate in other mounts or during programmatic updates.
pub(super) fn observe_selection(inner: &Rc<Inner>) {
    if !inner.active.get() {
        return;
    }
    let id = inner.dom.try_borrow().ok().and_then(|dom| dom.focused_id());
    if let Some(id) = id {
        publish_selection(inner, id);
    }
}

pub(super) fn reconcile_capture(inner: &Inner) {
    let invalid: Vec<_> = inner
        .captures
        .borrow()
        .iter()
        .filter_map(|(&pointer, &id)| {
            let dom = inner.dom.borrow();
            match dom.entries.get(&id) {
                Some(entry) if !entry.element.disabled => None,
                entry => Some((pointer, entry.map(|entry| entry.node.clone()))),
            }
        })
        .collect();
    for (pointer, node) in invalid {
        inner.captures.borrow_mut().remove(&pointer);
        if let Some(node) = node {
            let _ = node.release_pointer_capture(pointer as i32);
        }
    }
}

impl Dom {
    pub(super) fn geometry(&self, id: ElementId) -> Option<refract_core::ElementGeometry> {
        self.geometry_at(id, None)
    }

    fn geometry_at(
        &self,
        id: ElementId,
        point: Option<EventPoint>,
    ) -> Option<refract_core::ElementGeometry> {
        let entry = self.entries.get(&id)?;
        let width = f64::from(entry.node.offset_width());
        let height = f64::from(entry.node.offset_height());
        let mut linear = refract_core::Affine2d::IDENTITY;
        let mut node: Option<web_sys::Element> = Some(entry.node.clone().into());
        while let Some(current) = node {
            if let Some(e) = current
                .get_attribute("data-refract-id")
                .and_then(|v| v.parse().ok())
                .and_then(|id| self.entries.get(&ElementId(id)))
            {
                let m = e.element.style.transform.matrix(0.0, 0.0, 0.0, 0.0);
                let [a, b, c, d, _, _] = m.0;
                linear = refract_core::Affine2d([a, b, c, d, 0.0, 0.0]).then(linear);
            }
            if current == *self.root {
                break;
            }
            node = current.parent_element();
        }

        // Recover translation from the DOM bounds so layout and scrolling are
        // included without treating the rotated bounding box as local geometry.
        let bounds = refract_core::Rect::new(0.0, 0.0, width, height);
        let projected = bounds.transformed(linear);
        let actual = entry.node.get_bounding_client_rect();
        let root = self.root.get_bounding_client_rect();
        linear.0[4] = actual.left() - root.left() - projected.x;
        linear.0[5] = actual.top() - root.top() - projected.y;
        let text_offset = point.and_then(|p| {
            entry
                .element
                .input
                .as_ref()
                .filter(|input| input.rich_text)
                .and_then(|_| {
                    crate::rich_text::hit_offset(&entry.node, p.x + root.left(), p.y + root.top())
                })
        });
        Some(refract_core::ElementGeometry {
            text_offset,
            bounds,
            transform: linear,
        })
    }
}

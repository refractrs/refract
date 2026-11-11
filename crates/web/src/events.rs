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
                app.dispatch(id, event);
            }
            Some(Queued::Focus(target)) => app.observe_focus(target),
            None => break,
        }
    }
    app.dispatch_event(id, event)
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
    // Typing in a pointer-focused editor must not promote its editing cue to
    // a navigation ring. Tab is the explicit switch back to keyboard navigation.
    if native.type_() == "pointerdown" {
        let _ = inner.dom.borrow().root.set_attribute("data-refract-input", "pointer");
    }
    let Some(id) = owner(inner, native) else {
        return true;
    };
    let name = native.type_();
    if matches!(name.as_str(), "keydown" | "keyup") {
        let Some(key) = native.dyn_ref::<web_sys::KeyboardEvent>() else {
            return true;
        };
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
        let response = send(inner, id, UiEvent::Pointer(event));
        apply(inner, native, response);
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
    false
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

use refract_core::{prelude::*, App, ElementKind, Event};
use refract_test::TestHost;
use std::{cell::RefCell, rc::Rc};

fn key() -> Event {
    Event::Key(KeyEvent {
        key: Key::Escape,
        code: "Escape".into(),
        state: KeyState::Down,
        modifiers: Modifiers::default(),
        repeat: false,
        is_composing: false,
    })
}

#[test]
fn keyboard_bubbles_in_order_and_can_stop_ancestors_without_stopping_siblings() {
    for stop in [false, true] {
        let calls = Rc::new(RefCell::new(Vec::new()));
        let parent = calls.clone();
        let first = calls.clone();
        let second = calls.clone();
        let mut app = App::new(
            view()
                .on_key(move |cx, _| {
                    parent
                        .borrow_mut()
                        .push(("parent", cx.target(), cx.current_target()));
                })
                .child(
                    text("target")
                        .focusable(true)
                        .on_key(move |cx, event| {
                            assert_eq!(event.key, Key::Escape);
                            first
                                .borrow_mut()
                                .push(("first", cx.target(), cx.current_target()));
                            cx.prevent_default();
                            if stop {
                                cx.stop_propagation();
                            }
                        })
                        .on_key(move |cx, _| {
                            assert!(cx.default_prevented());
                            second
                                .borrow_mut()
                                .push(("second", cx.target(), cx.current_target()));
                        }),
                ),
        );
        let mut host = TestHost::default();
        host.flush(&mut app).unwrap();
        let root = host.children[&None][0];
        let child = host.children[&Some(root)][0];
        let response = app.dispatch_event(child, key());
        assert!(response.default_prevented);
        assert_eq!(response.propagation_stopped, stop);
        assert_eq!(
            *calls.borrow(),
            if stop {
                vec![("first", child, child), ("second", child, child)]
            } else {
                vec![
                    ("first", child, child),
                    ("second", child, child),
                    ("parent", child, root),
                ]
            }
        );
    }
}

#[test]
fn field_updates_binding_before_callbacks_and_does_not_observe_programmatic_writes() {
    let captured = Rc::new(RefCell::new(None));
    let changes = Rc::new(RefCell::new(Vec::new()));
    let holder = captured.clone();
    let received = changes.clone();
    let mut app = App::new(view_fn(move |cx| {
        let value = cx.state("value", || "initial".to_owned());
        *holder.borrow_mut() = Some(value.clone());
        let current = value.clone();
        let received = received.clone();
        let field: Field = field(value)
            .on_input(move |cx, text| {
                assert_eq!(current.get(cx.runtime()), text);
                received.borrow_mut().push(text.to_owned());
            })
            .on_key(|cx, _| cx.prevent_default())
            .multiline(true);
        Ok(field)
    }));
    let mut host = TestHost::default();
    host.flush(&mut app).unwrap();
    let (&id, element) = host
        .elements
        .iter()
        .find(|(_, e)| e.kind == ElementKind::TextInput)
        .unwrap();
    assert!(element.multiline);
    app.dispatch(id, Event::Input("typed".into()));
    host.flush(&mut app).unwrap();
    app.mutate(|rt| {
        captured
            .borrow()
            .as_ref()
            .unwrap()
            .set(rt, "external".into())
    });
    host.flush(&mut app).unwrap();
    assert_eq!(*changes.borrow(), ["typed"]);
    assert!(app.dispatch_event(id, key()).default_prevented);
    assert_eq!(host.elements[&id].value, "external");
}

#[test]
fn focus_and_hover_are_targeted_and_disabled_handlers_do_not_run() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let a = calls.clone();
    let b = calls.clone();
    let mut app = App::new(
        view()
            .on_hover(|_, _| panic!("hover must not bubble"))
            .on_focus_change(|_, _| panic!("focus must not bubble"))
            .child(
                image("photo.png")
                    .on_hover(move |_, inside| a.borrow_mut().push(inside))
                    .on_focus_change(move |_, focused| b.borrow_mut().push(focused)),
            )
            .child(view().disabled(true).on_key(|_, _| panic!("disabled"))),
    );
    let mut host = TestHost::default();
    host.flush(&mut app).unwrap();
    let root = host.children[&None][0];
    let children = &host.children[&Some(root)];
    app.dispatch(children[0], Event::Hover(true));
    app.dispatch(children[0], Event::Focus(false));
    assert_eq!(*calls.borrow(), [true, false]);
    assert!(!app.dispatch_event(children[1], key()).handled);
    app.set_view(Empty);
    host.flush(&mut app).unwrap();
    assert!(!app.dispatch(root, key()));
}

#[test]
fn capture_requests_identify_the_current_pointer_and_handler_owner() {
    let mut app = App::new(
        view()
            .on_pointer(|cx, event| {
                if event.phase == PointerPhase::Down {
                    cx.capture_pointer();
                }
                if event.phase == PointerPhase::Up {
                    cx.release_pointer();
                }
            })
            .child(text("child")),
    );
    let mut host = TestHost::default();
    host.flush(&mut app).unwrap();
    let root = host.children[&None][0];
    let child = host.children[&Some(root)][0];
    let mut event = PointerEvent {
        id: 42,
        kind: PointerKind::Mouse,
        phase: PointerPhase::Down,
        position: EventPoint { x: 20.0, y: 30.0 },
        button: Some(PointerButton::Primary),
        buttons: PointerButtons(1),
        modifiers: Modifiers::default(),
        pressure: 0.5,
    };
    let request = app
        .dispatch_event(child, Event::Pointer(event))
        .pointer_capture
        .unwrap();
    assert_eq!(request.target, root);
    assert_eq!(request.pointer_id, 42);
    assert!(request.capture);
    event.phase = PointerPhase::Up;
    assert!(
        !app.dispatch_event(child, Event::Pointer(event))
            .pointer_capture
            .unwrap()
            .capture
    );
}

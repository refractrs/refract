//! Activation is a capability of visual primitives, not a separate element kind.
use refract_core::{prelude::*, App, ElementKind, Event, Interaction};
use refract_test::TestHost;
use std::{cell::RefCell, rc::Rc};

#[test]
fn primitives_support_press_before_and_after_style_modifiers() {
    for kind in [ElementKind::Box, ElementKind::Text, ElementKind::Image] {
        for styled_first in [false, true] {
            let calls = Rc::new(RefCell::new(Vec::new()));
            let callback = calls.clone();
            macro_rules! activate {
                ($node:expr) => {{
                    let node = $node;
                    let node = if styled_first {
                        node.width(100.0).on_press(move |_| callback.borrow_mut().push(1))
                    } else {
                        node.on_press(move |_| callback.borrow_mut().push(1)).width(100.0)
                    };
                    node.into_any()
                }};
            }
            let node = match kind {
                ElementKind::Box => activate!(view()),
                ElementKind::Text => activate!(text("Activate")),
                ElementKind::Image => activate!(image("/photo.png").label("Activate")),
                _ => unreachable!(),
            };
            let mut app = App::new(node);
            let mut host = TestHost::default();
            host.flush(&mut app).unwrap();
            assert_eq!(
                host.elements.len(),
                1,
                "activation must not add a layout box"
            );
            let (&id, element) = host.elements.iter().next().unwrap();
            assert_eq!(element.kind, kind);
            assert_eq!(element.interaction, Some(Interaction::Activate));
            assert_eq!(element.role.as_deref(), Some("button"));
            assert!(app.dispatch(id, Event::Press));
            app.dispatch(id, Event::Focus(true));
            assert_eq!(*calls.borrow(), [1]);
            app.set_view(Empty);
            host.flush(&mut app).unwrap();
            assert!(!app.dispatch(id, Event::Press));
        }
    }
}

#[test]
fn press_callbacks_compose_in_order_and_keep_scroll_observers() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let scroll = calls.clone();
    let first = calls.clone();
    let second = calls.clone();
    let mut app = App::new(
        scroll_view(text("content"))
            .on_scroll(move |_, _| scroll.borrow_mut().push("scroll"))
            .on_press(move |_| first.borrow_mut().push("first"))
            .width(100.0)
            .on_press(move |_| second.borrow_mut().push("second")),
    );
    let mut host = TestHost::default();
    host.flush(&mut app).unwrap();
    let id = host.children[&None][0];
    app.dispatch(
        id,
        Event::Scroll(ScrollEvent {
            metrics: ScrollMetrics::default(),
            delta: ScrollOffset::default(),
        }),
    );
    assert_eq!(*calls.borrow(), ["scroll"]);
    app.dispatch(id, Event::Press);
    assert_eq!(*calls.borrow(), ["scroll", "first", "second"]);
}

#[test]
fn typed_press_callbacks_share_composition_and_disabled_behavior() {
    for disabled in [false, true] {
        let calls = Rc::new(RefCell::new(Vec::new()));
        let first = calls.clone();
        let second = calls.clone();
        let third = calls.clone();
        let mut app = App::new(
            text("press")
                .on_press(move |_| first.borrow_mut().push(1))
                .on_press(move |_| second.borrow_mut().push(2))
                .font_size(16.0)
                .on_press(move |_| third.borrow_mut().push(3))
                .disabled(disabled),
        );
        let mut host = TestHost::default();
        host.flush(&mut app).unwrap();
        let id = host.children[&None][0];
        assert_eq!(app.dispatch(id, Event::Press), !disabled);
        assert_eq!(
            *calls.borrow(),
            if disabled { vec![] } else { vec![1, 2, 3] }
        );
    }
}

#[test]
fn structural_order_defines_activation_target() {
    for outer in [false, true] {
        let node = if outer {
            text("press").padding(12.0).on_press(|_| {})
        } else {
            text("press").on_press(|_| {}).padding(12.0)
        };
        let mut app = App::new(node);
        let mut host = TestHost::default();
        host.flush(&mut app).unwrap();
        assert_eq!(host.elements.len(), 2);
        let root = host.children[&None][0];
        let child = host.children[&Some(root)][0];
        let target = if outer { root } else { child };
        let passive = if outer { child } else { root };
        assert_eq!(
            host.elements[&target].interaction,
            Some(Interaction::Activate)
        );
        assert_eq!(host.elements[&passive].interaction, None);
        assert!(app.dispatch(target, Event::Press));
        assert!(!app.dispatch(passive, Event::Press));
    }
}

#[test]
fn opaque_views_receive_an_activation_wrapper_and_keep_existing_roles() {
    let mut app = App::new(view_fn(|_| Ok(text("content"))).on_press(|_| {}));
    let mut host = TestHost::default();
    host.flush(&mut app).unwrap();
    assert_eq!(host.elements.len(), 2);
    assert_eq!(
        host.elements[&host.children[&None][0]].interaction,
        Some(Interaction::Activate)
    );

    app.set_view(text("Title").heading(2).on_press(|_| {}));
    host.flush(&mut app).unwrap();
    assert_eq!(
        host.elements[&host.children[&None][0]].role.as_deref(),
        Some("heading")
    );
}

#[test]
fn activation_keeps_nested_control_validation() {
    let mut host = TestHost::default();
    let mut app = App::new(
        view()
            .child(text("child").on_press(|_| {}))
            .on_press(|_| {}),
    );
    assert!(host.flush(&mut app).unwrap_err().0.contains("nested"));
    assert!(host.elements.is_empty());

    app.set_view(
        view_fn(|cx| {
            let value = cx.state("value", String::new);
            Ok(field(value))
        })
        .on_press(|_| {}),
    );
    assert!(host.flush(&mut app).unwrap_err().0.contains("nested"));
    assert!(host.elements.is_empty());
}

#[test]
fn replacement_handlers_publish_without_remounting_interactive_text() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let make_view = |value| {
        let calls = calls.clone();
        text("stable")
            .font_size(16.0)
            .on_press(move |_| calls.borrow_mut().push(value))
    };
    let mut app = App::new(make_view(1));
    let mut host = TestHost::default();
    host.flush(&mut app).unwrap();
    let id = host.children[&None][0];
    app.dispatch(id, Event::Press);
    app.set_view(make_view(2));
    host.flush(&mut app).unwrap();
    assert!(host.updates.last().unwrap().patches.is_empty());
    assert!(app.dispatch(id, Event::Press));
    assert_eq!(*calls.borrow(), [1, 2]);
}

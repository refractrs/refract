use refract_core::{prelude::*, App, ElementId};
use refract_test::TestHost;
use std::{cell::RefCell, rc::Rc};

fn id(host: &TestHost, label: &str) -> ElementId {
    *host
        .elements
        .iter()
        .find(|(_, e)| e.label.as_deref() == Some(label))
        .unwrap()
        .0
}

fn observe(app: &mut App, host: &mut TestHost, target: Option<ElementId>) {
    host.focused = target;
    app.observe_focus(target);
    host.flush(app).unwrap();
}

#[test]
fn requests_and_observations_share_state_without_remounting() {
    let mut app = App::new(Empty);
    let focus = State::new(app.runtime_mut(), None::<u8>);
    let draft = State::new(app.runtime_mut(), String::new());
    app.set_view(
        v_stack()
            .child(
                field(draft.binding())
                    .label("editor")
                    .focused_value(focus.binding(), 1),
            )
            .child(view().label("custom").focused_value(focus.binding(), 2)),
    );
    let mut host = TestHost::default();
    host.flush(&mut app).unwrap();
    let editor = id(&host, "editor");
    let custom = id(&host, "custom");
    focus.set(app.runtime_mut(), Some(1));
    host.flush(&mut app).unwrap();
    assert_eq!(host.focused, Some(editor));
    observe(&mut app, &mut host, Some(custom));
    assert_eq!(*focus.get(app.runtime()), Some(2));
    focus.set(app.runtime_mut(), None);
    host.flush(&mut app).unwrap();
    assert_eq!(host.focused, None);
    assert_eq!(id(&host, "editor"), editor);
    assert!(!host.flush(&mut app).unwrap());
}

#[test]
fn nested_observers_do_not_blur_between_siblings_and_can_style_the_parent() {
    let mut app = App::new(Empty);
    let within = State::new(app.runtime_mut(), false);
    let events = Rc::new(RefCell::new(Vec::new()));
    app.set_view(view_fn({
        let within = within.clone();
        let events = events.clone();
        move |cx| {
            let active = cx.read_state(&within);
            let state = within.clone();
            let events = events.clone();
            Ok(v_stack()
                .label("parent")
                .background(if active { Color::WHITE } else { Color::BLACK })
                .child(view().label("a").focusable(true))
                .child(view().label("b").focusable(true))
                .on_focus_within_change(move |cx, value| {
                    events.borrow_mut().push(value);
                    state.set(cx.runtime(), value);
                }))
        }
    }));
    let mut host = TestHost::default();
    host.flush(&mut app).unwrap();
    let a = id(&host, "a");
    let b = id(&host, "b");
    let parent = id(&host, "parent");
    observe(&mut app, &mut host, Some(a));
    assert_eq!(host.elements[&parent].style.background, Some(Color::WHITE));
    observe(&mut app, &mut host, Some(b));
    assert_eq!(&*events.borrow(), &[true]);
    observe(&mut app, &mut host, None);
    assert_eq!(&*events.borrow(), &[true, false]);
    assert_eq!(id(&host, "a"), a);
}

#[test]
fn focus_callbacks_can_request_another_target() {
    let mut app = App::new(Empty);
    let focus = State::new(app.runtime_mut(), Some(1_u8));
    app.set_view(
        v_stack()
            .child(
                view()
                    .label("a")
                    .focused_value(focus.binding(), 1)
                    .on_focus_change({
                        let focus = focus.clone();
                        move |cx, active| {
                            if active {
                                focus.set(cx.runtime(), Some(2));
                            }
                        }
                    }),
            )
            .child(view().label("b").focused_value(focus.binding(), 2)),
    );
    let mut host = TestHost::default();
    host.flush(&mut app).unwrap();
    assert_eq!(host.focused, Some(id(&host, "b")));
    assert_eq!(*focus.get(app.runtime()), Some(2));
}

#[test]
fn invalid_requests_reconcile_and_do_not_activate_later() {
    let mut app = App::new(Empty);
    let focus = State::new(app.runtime_mut(), Some(2_u8));
    let disabled = State::new(app.runtime_mut(), true);
    app.set_view(view_fn({
        let focus = focus.clone();
        let disabled = disabled.clone();
        move |cx| {
            Ok(v_stack()
                .child(view().label("a").focused_value(focus.binding(), 1))
                .child(
                    view()
                        .label("b")
                        .disabled(cx.read_state(&disabled))
                        .focused_value(focus.binding(), 2),
                ))
        }
    }));
    let mut host = TestHost::default();
    host.flush(&mut app).unwrap();
    assert_eq!(*focus.get(app.runtime()), None);
    disabled.set(app.runtime_mut(), false);
    host.flush(&mut app).unwrap();
    assert_eq!(host.focused, None);
    focus.set(app.runtime_mut(), Some(99));
    host.flush(&mut app).unwrap();
    assert_eq!(*focus.get(app.runtime()), None);
    focus.set(app.runtime_mut(), Some(1));
    host.flush(&mut app).unwrap();
    app.set_view(Empty);
    host.flush(&mut app).unwrap();
    assert_eq!(host.focused, None);
    assert_eq!(*focus.get(app.runtime()), None);
}

#[test]
fn registrations_are_transactional_and_duplicates_are_rejected() {
    let mut app = App::new(Empty);
    let focus = State::new(app.runtime_mut(), true);
    app.set_view(view().focused(focus.binding()));
    drop(app.prepare().unwrap());
    assert_eq!(app.take_focus_request().unwrap(), None);
    app.set_view(
        v_stack()
            .child(view().focused(focus.binding()))
            .child(view().focused(focus.binding())),
    );
    assert!(app
        .prepare()
        .err()
        .unwrap()
        .0
        .contains("ambiguous focus attachment"));
    assert_eq!(app.take_focus_request().unwrap(), None);
    app.set_view(view().focused(focus.binding()));
    let mut host = TestHost::default();
    host.flush(&mut app).unwrap();
    assert!(host.focused.is_some());
    app.dispose();
    assert!(!*focus.get(app.runtime()));
}

#[test]
fn conflicting_requests_are_diagnosed_and_reconciled() {
    let mut app = App::new(Empty);
    let a = State::new(app.runtime_mut(), true);
    let b = State::new(app.runtime_mut(), true);
    app.set_view(
        v_stack()
            .child(view().focused(a.binding()))
            .child(view().focused(b.binding())),
    );
    let mut host = TestHost::default();
    assert!(host
        .flush(&mut app)
        .unwrap_err()
        .0
        .contains("conflicting focus requests"));
    assert!(!*a.get(app.runtime()) && !*b.get(app.runtime()));
    host.flush(&mut app).unwrap();
    a.set(app.runtime_mut(), true);
    host.flush(&mut app).unwrap();
    let owner = host.focused;
    b.set(app.runtime_mut(), false);
    host.flush(&mut app).unwrap();
    assert_eq!(
        host.focused, owner,
        "false on another binding cannot blur the owner"
    );
}

#[test]
fn window_activation_does_not_change_logical_focus() {
    let mut app = App::new(Empty);
    let focus = State::new(app.runtime_mut(), true);
    app.set_view(view().focused(focus.binding()));
    let mut host = TestHost::default();
    let surface = host.install_surface(&mut app, "Focus", None);
    host.flush(&mut app).unwrap();
    let owner = host.focused;
    host.surface
        .as_ref()
        .unwrap()
        .publish_active(app.runtime_mut(), false);
    host.flush(&mut app).unwrap();
    assert!(!surface.is_active.get(app.runtime()));
    assert_eq!(host.focused, owner);
    assert!(*focus.get(app.runtime()));
}

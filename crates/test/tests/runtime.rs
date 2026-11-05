use refract_core::{prelude::*, App, ElementId, ElementKind, Event, Patch};
use refract_test::TestHost;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

fn counter() -> impl View {
    view_fn(|cx| {
        let count = cx.state("count", || 0);
        let value = cx.read(&count);
        Ok(h_stack()
            .alignment(Alignment::Center)
            .justify(Alignment::Center)
            .child(text(value.to_string()))
            .on_press(move |cx| count.update(cx.runtime(), |n| *n += 1))
            .label("increment"))
    })
}
#[test]
fn counter_updates_text_without_replacing_dom_identity() {
    let mut app = App::new(counter());
    let mut host = TestHost::default();
    host.flush(&mut app).unwrap();
    let button = host.button("increment");
    let initial = host.elements.len();
    host.press(&mut app, "increment").unwrap();
    host.press(&mut app, "increment").unwrap();
    assert_eq!(host.text(), "2");
    assert_eq!(host.button("increment"), button);
    assert_eq!(host.elements.len(), initial);
    assert!(host
        .updates
        .last()
        .unwrap()
        .patches
        .iter()
        .all(|p| matches!(p,Patch::Update(_,e) if e.kind==ElementKind::Text)));
}
#[test]
fn dirty_leaf_skips_wide_siblings() {
    let calls = Rc::new(Cell::new(0));
    let mut app = App::new(v_stack().child(counter()).children((0..1000).map(|_| {
        let calls = calls.clone();
        view_fn(move |_| {
            calls.set(calls.get() + 1);
            Ok(text("stable"))
        })
    })));
    let mut host = TestHost::default();
    host.flush(&mut app).unwrap();
    assert_eq!(calls.get(), 1000);
    host.press(&mut app, "increment").unwrap();
    assert_eq!(calls.get(), 1000);
    assert!(app.stats().visited < 10, "{:?}", app.stats());
}
fn row(id: i32) -> impl View {
    view_fn(move |cx| {
        let state = cx.state("value", || id * 10);
        let value = cx.read(&state);
        Ok(h_stack()
            .alignment(Alignment::Center)
            .justify(Alignment::Center)
            .child(text(value.to_string()))
            .on_press(move |cx| state.update(cx.runtime(), |v| *v += 1))
            .label(id.to_string()))
    })
}
#[test]
fn keyed_reorder_preserves_state_and_removal_resets_it() {
    let external = Rc::new(RefCell::new(None));
    let saved = external.clone();
    let mut app = App::new(view_fn(move |cx| {
        let order = cx.state("order", || vec![1, 2, 3]);
        *saved.borrow_mut() = Some(order.clone());
        let rows = cx.read(&order);
        Ok(v_stack().children(rows.into_iter().map(|id| row(id).key(id))))
    }));
    let mut host = TestHost::default();
    host.flush(&mut app).unwrap();
    host.press(&mut app, "2").unwrap();
    let id = host.button("2");
    let order = external.borrow().clone().unwrap();
    app.mutate(|rt| order.set(rt, vec![3, 2, 1]));
    host.flush(&mut app).unwrap();
    assert_eq!(host.text(), "30 21 10");
    assert_eq!(host.button("2"), id);
    app.mutate(|rt| order.set(rt, vec![3, 1]));
    host.flush(&mut app).unwrap();
    assert!(!app.dispatch(id, Event::Press));
    app.mutate(|rt| order.set(rt, vec![3, 2, 1]));
    host.flush(&mut app).unwrap();
    assert_eq!(host.text(), "30 20 10");
}
#[test]
fn duplicate_keys_and_invalid_styles_fail_before_mutation() {
    let mut host = TestHost::default();
    let mut app = App::new(counter());
    host.flush(&mut app).unwrap();
    let before = host.text();
    app.set_view(
        v_stack()
            .child(text("a").key("same"))
            .child(text("b").key("same")),
    );
    assert!(host.flush(&mut app).unwrap_err().0.contains("duplicate"));
    assert_eq!(host.text(), before);
    app.set_view(text("bad").padding(-1.0));
    assert!(host.flush(&mut app).is_err());
    assert_eq!(host.text(), before);
}
#[test]
fn changed_handler_publishes_even_without_platform_patches() {
    fn action(value: i32, result: Rc<Cell<i32>>) -> impl View {
        h_stack()
            .alignment(Alignment::Center)
            .justify(Alignment::Center)
            .child(text("unchanged"))
            .on_press(move |_| result.set(value))
            .label("action")
    }
    let result = Rc::new(Cell::new(0));
    let mut app = App::new(action(1, result.clone()));
    let mut host = TestHost::default();
    host.flush(&mut app).unwrap();
    app.set_view(action(2, result.clone()));
    host.flush(&mut app).unwrap();
    assert!(host.updates.last().unwrap().patches.is_empty());
    host.press(&mut app, "action").unwrap();
    assert_eq!(result.get(), 2);
}
struct Theme;
impl EnvironmentKey for Theme {
    type Value = i32;
    fn default() -> i32 {
        1
    }
}
fn theme() -> impl View {
    view_fn(|cx| {
        let theme = cx.environment::<Theme>();
        Ok(text(theme.to_string()))
    })
}
#[test]
fn environment_nearest_provider_and_replacement() {
    let mut app = App::new(
        v_stack()
            .child(theme())
            .child(theme().environment::<Theme>(8)),
    );
    let mut host = TestHost::default();
    host.flush(&mut app).unwrap();
    assert_eq!(host.text(), "1 8");
    app.set_environment::<Theme>(2);
    host.flush(&mut app).unwrap();
    assert_eq!(host.text(), "2 8");
    app.set_view(
        v_stack()
            .child(theme())
            .child(theme().environment::<Theme>(9)),
    );
    host.flush(&mut app).unwrap();
    assert_eq!(host.text(), "2 9");
}
#[test]
fn effects_only_start_after_commit_and_clean_up_once() {
    let starts = Rc::new(Cell::new(0));
    let stops = Rc::new(Cell::new(0));
    let mut app = App::new(view_fn({
        let starts = starts.clone();
        let stops = stops.clone();
        move |cx| {
            let a = starts.clone();
            let b = stops.clone();
            cx.effect("subscription", (), move |_| {
                a.set(a.get() + 1);
                let b = b.clone();
                Box::new(move || b.set(b.get() + 1))
            });
            Ok(text("effect"))
        }
    }));
    drop(app.prepare().unwrap());
    assert_eq!(starts.get(), 0);
    let mut host = TestHost::default();
    host.flush(&mut app).unwrap();
    assert_eq!(starts.get(), 1);
    app.dispose();
    app.dispose();
    assert_eq!(stops.get(), 1);
    assert_eq!(app.runtime().stats().attributes, 0);
}
#[test]
fn stale_or_foreign_candidate_cannot_publish() {
    let mut a = App::new(counter());
    let mut b = App::new(counter());
    let pending = a.prepare().unwrap().unwrap();
    assert!(b.commit(pending).is_err());
    assert_eq!(b.revision(), 0);
}
#[test]
fn mounted_state_names_and_types_are_validated() {
    let mut app = App::new(view_fn(|cx| {
        cx.state("x", || 1);
        cx.state("x", || 2);
        Ok(text("bad"))
    }));
    assert!(app.prepare().err().unwrap().0.contains("duplicate"));
}
#[test]
fn optional_children_do_not_shift_state() {
    fn tree(show: bool) -> impl View {
        v_stack()
            .maybe_child(show.then(|| text("optional")))
            .child(counter())
    }
    let mut app = App::new(tree(false));
    let mut host = TestHost::default();
    host.flush(&mut app).unwrap();
    host.press(&mut app, "increment").unwrap();
    app.set_view(tree(true));
    host.flush(&mut app).unwrap();
    assert_eq!(host.text(), "optional 1");
}
#[test]
fn button_cannot_contain_interactive_control() {
    let mut app = App::new(
        h_stack()
            .alignment(Alignment::Center)
            .justify(Alignment::Center)
            .child(
                h_stack()
                    .alignment(Alignment::Center)
                    .justify(Alignment::Center)
                    .child(text("nested"))
                    .on_press(|_| {}),
            )
            .on_press(|_| {}),
    );
    assert!(app.prepare().err().unwrap().0.contains("interactive"));
}
#[test]
fn graph_storage_does_not_grow_across_remounts() {
    let mut app = App::new(counter());
    let mut host = TestHost::default();
    host.flush(&mut app).unwrap();
    let baseline = app.runtime().stats().attributes;
    for n in 0..100 {
        app.set_view(counter().key(n));
        host.flush(&mut app).unwrap();
    }
    assert!(app.runtime().stats().attributes <= baseline + 2);
    app.dispose();
    assert_eq!(app.runtime().stats().attributes, 0);
}
#[test]
fn state_write_after_preparation_rejects_candidate() {
    let mut app = App::new(counter());
    let mut host = TestHost::default();
    host.flush(&mut app).unwrap();
    let id = host.button("increment");
    app.dispatch(id, Event::Press);
    let pending = app.prepare().unwrap().unwrap();
    app.dispatch(id, Event::Press);
    assert!(app.commit(pending).is_err());
    host.flush(&mut app).unwrap();
    assert_eq!(host.text(), "2");
}
#[test]
fn partial_host_failure_does_not_publish_core() {
    let mut app = App::new(counter());
    let pending = app.prepare().unwrap().unwrap();
    drop(pending);
    assert_eq!(app.revision(), 0);
    assert!(app.elements().is_empty());
}
#[test]
fn unknown_events_are_ignored() {
    let mut app = App::new(counter());
    assert!(!app.dispatch(ElementId(999), Event::Press));
}

#[test]
fn memo_skips_equal_props_but_keeps_reactive_dependencies() {
    fn child(calls: Rc<Cell<usize>>, value: Binding<i32>) -> impl View {
        view_fn(move |cx| {
            calls.set(calls.get() + 1);
            Ok(text(cx.read(&value).to_string()))
        })
    }
    let captured = Rc::new(RefCell::new(None));
    let calls = Rc::new(Cell::new(0));
    let mut app = App::new(view_fn({
        let captured = captured.clone();
        let calls = calls.clone();
        move |cx| {
            let parent = cx.state("parent", || 0);
            let value = cx.state("value", || 0);
            *captured.borrow_mut() = Some((parent.clone(), value.clone()));
            let _ = cx.read(&parent);
            Ok(child(calls.clone(), value).memo(()))
        }
    }));
    let mut host = TestHost::default();
    host.flush(&mut app).unwrap();
    assert_eq!(calls.get(), 1);
    let (parent, value) = captured.borrow().clone().unwrap();
    app.mutate(|rt| parent.set(rt, 1));
    host.flush(&mut app).unwrap();
    assert_eq!(calls.get(), 1);
    app.mutate(|rt| value.set(rt, 9));
    host.flush(&mut app).unwrap();
    assert_eq!(calls.get(), 2);
    assert_eq!(host.text(), "9");
}
#[test]
fn memo_detects_provider_identity_changes() {
    fn screen(value: i32) -> impl View {
        theme().memo(()).environment::<Theme>(value)
    }
    let mut app = App::new(screen(1));
    let mut host = TestHost::default();
    host.flush(&mut app).unwrap();
    app.set_view(screen(2));
    host.flush(&mut app).unwrap();
    assert_eq!(host.text(), "2");
}
#[test]
fn same_key_with_different_element_kind_remounts() {
    fn content(button_kind: bool) -> impl View {
        if button_kind {
            h_stack()
                .alignment(Alignment::Center)
                .justify(Alignment::Center)
                .child(text("b"))
                .on_press(|_| {})
                .key("same")
                .into_any()
        } else {
            text("a").key("same").into_any()
        }
    }
    let mut app = App::new(content(false));
    let mut host = TestHost::default();
    host.flush(&mut app).unwrap();
    app.set_view(content(true));
    host.flush(&mut app).unwrap();
    assert_eq!(host.text(), "b");
}
#[test]
fn same_key_with_different_interaction_remounts() {
    let mut app = App::new(view().child(text("action")).key("same"));
    let mut host = TestHost::default();
    host.flush(&mut app).unwrap();
    let original = host.children[&None][0];

    app.set_view(
        h_stack()
            .alignment(Alignment::Center)
            .justify(Alignment::Center)
            .child(text("action"))
            .on_press(|_| {})
            .label("action")
            .key("same"),
    );
    host.flush(&mut app).unwrap();
    let action = host.button("action");
    assert_ne!(action, original);
    assert_eq!(host.elements[&action].kind, ElementKind::Box);
}
#[test]
fn generated_keyed_edits_match_independent_model() {
    let captured = Rc::new(RefCell::new(None));
    let mut app = App::new(view_fn({
        let captured = captured.clone();
        move |cx| {
            let order = cx.state("order", || vec![1, 2, 3]);
            *captured.borrow_mut() = Some(order.clone());
            Ok(v_stack().children(cx.read(&order).into_iter().map(|id| row(id).key(id))))
        }
    }));
    let mut host = TestHost::default();
    host.flush(&mut app).unwrap();
    let binding = captured.borrow().clone().unwrap();
    let mut order = vec![1, 2, 3];
    let mut values = std::collections::HashMap::from([(1, 10), (2, 20), (3, 30)]);
    let mut seed = 17u64;
    for _ in 0..300 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        match (seed >> 32) % 4 {
            0 => order.reverse(),
            1 if !order.is_empty() => {
                let id = order[(seed as usize) % order.len()];
                host.press(&mut app, &id.to_string()).unwrap();
                *values.get_mut(&id).unwrap() += 1;
            }
            2 if order.len() > 1 => {
                let id = order.remove((seed as usize) % order.len());
                values.remove(&id);
            }
            _ => {
                let id = ((seed >> 16) % 8 + 1) as i32;
                if !order.contains(&id) {
                    order.insert(0, id);
                    values.insert(id, id * 10);
                }
            }
        }
        app.mutate(|rt| binding.set(rt, order.clone()));
        host.flush(&mut app).unwrap();
        let expected = order
            .iter()
            .map(|id| values[id].to_string())
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(host.text(), expected);
    }
}

#[test]
fn root_replacement_or_disposal_invalidates_prepared_work() {
    let mut app = App::new(counter());
    let pending = app.prepare().unwrap().unwrap();
    app.set_view(text("new root"));
    assert!(app.commit(pending).is_err());
    let pending = app.prepare().unwrap().unwrap();
    app.dispose();
    assert!(app.commit(pending).is_err());
    assert!(app.elements().is_empty());
}

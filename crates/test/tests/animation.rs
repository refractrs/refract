use refract_core::{prelude::*, App, Patch};
use refract_test::TestHost;
use std::{cell::RefCell, rc::Rc, time::Duration};

#[test]
fn explicit_context_survives_discard_and_stays_in_its_branch() {
    let saved = Rc::new(RefCell::new(Vec::new()));
    let mut app = App::new(view_fn({
        let saved = saved.clone();
        move |cx| {
            let a = cx.state("a", || false);
            let b = cx.state("b", || false);
            *saved.borrow_mut() = vec![a.clone(), b.clone()];
            Ok(
                h_stack().children([a, b].into_iter().enumerate().map(|(i, state)| {
                    view_fn(move |cx| {
                        Ok(text(i.to_string()).opacity(if cx.read(&state) { 0.5 } else { 1.0 }))
                    })
                })),
            )
        }
    }));
    let mut host = TestHost::default();
    host.flush(&mut app).unwrap();
    let states = saved.borrow().clone();
    app.mutate(|rt| {
        with_animation(rt, Animation::spring(), |rt| states[0].set(rt, true));
        states[1].set(rt, true);
    });
    drop(app.prepare().unwrap().unwrap());
    let pending = app.prepare().unwrap().unwrap();
    for patch in pending.patches() {
        if let Patch::Update(id, element) = patch {
            assert_eq!(
                pending.update.animations[id].animation.is_some(),
                element.text == "0"
            );
        }
    }
    host.apply(&pending.update).unwrap();
    app.commit(pending).unwrap();
    assert!(app.prepare().unwrap().is_none());
}

#[test]
fn value_scope_is_staged_and_transaction_override_reaches_dirty_memo() {
    let saved = Rc::new(RefCell::new(None));
    let mut app = App::new(view_fn({
        let saved = saved.clone();
        move |cx| {
            let value = cx.state("value", || false);
            *saved.borrow_mut() = Some(value.clone());
            let selected = cx.read(&value);
            Ok(text("target")
                .opacity(if selected { 0.3 } else { 1.0 })
                .animation(Animation::linear(Duration::from_secs(1)), selected))
        }
    }));
    let mut host = TestHost::default();
    host.flush(&mut app).unwrap();
    assert!(host.updates[0]
        .animations
        .values()
        .all(|tx| tx.animation.is_none()));
    saved
        .borrow()
        .as_ref()
        .unwrap()
        .set(app.runtime_mut(), true);
    drop(app.prepare().unwrap().unwrap());
    let p = app.prepare().unwrap().unwrap();
    assert!(p.update.patches.iter().any(|patch| match patch {
        Patch::Update(id, _) => p.update.animations[id].animation.is_some(),
        _ => false,
    }));
    host.apply(&p.update).unwrap();
    app.commit(p).unwrap();

    let saved = Rc::new(RefCell::new(None));
    let mut app = App::new(
        view_fn({
            let saved = saved.clone();
            move |cx| {
                let value = cx.state("value", || false);
                *saved.borrow_mut() = Some(value.clone());
                Ok(text("disabled").opacity(if cx.read(&value) { 0.3 } else { 1.0 }))
            }
        })
        .memo(())
        .transaction(|tx| tx.disables_animations = true),
    );
    host = TestHost::default();
    host.flush(&mut app).unwrap();
    with_animation(app.runtime_mut(), Animation::spring(), |rt| {
        saved.borrow().as_ref().unwrap().set(rt, true)
    });
    let p = app.prepare().unwrap().unwrap();
    assert!(p
        .update
        .animations
        .values()
        .all(|tx| tx.animation.is_none()));
}

#[test]
fn presentation_frames_retarget_without_graph_work_and_ignore_text_updates() {
    let saved = Rc::new(RefCell::new(None));
    let mut app = App::new(view_fn({
        let saved = saved.clone();
        move |cx| {
            let value = cx.state("value", || 0.0f32);
            let label = cx.state("label", || String::from("first"));
            *saved.borrow_mut() = Some((value.clone(), label.clone()));
            Ok(text(cx.read(&label)).offset(cx.read(&value), 0.0))
        }
    }));
    let mut host = TestHost::default();
    host.flush(&mut app).unwrap();
    let id = app.elements()[0].0;
    let (value, label) = saved.borrow().clone().unwrap();
    let animation = Animation::linear(Duration::from_secs(1));
    with_animation(app.runtime_mut(), animation, |rt| value.set(rt, 100.0));
    host.flush(&mut app).unwrap();
    let revision = app.revision();
    let epoch = app.runtime().epoch();
    host.advance_time(Duration::from_millis(400));
    assert_eq!(host.presented(id).unwrap().style.transform.offset_x, 40.0);
    assert_eq!(app.revision(), revision);
    assert_eq!(app.runtime().epoch(), epoch);
    label.set(app.runtime_mut(), String::from("streamed"));
    host.flush(&mut app).unwrap();
    host.advance_time(Duration::from_millis(100));
    assert_eq!(host.presented(id).unwrap().style.transform.offset_x, 50.0);
    with_animation(app.runtime_mut(), animation, |rt| value.set(rt, 0.0));
    host.flush(&mut app).unwrap();
    assert_eq!(host.presented(id).unwrap().style.transform.offset_x, 50.0);
    host.advance_time(Duration::from_millis(500));
    assert_eq!(host.presented(id).unwrap().style.transform.offset_x, 25.0);
    value.set(app.runtime_mut(), 75.0);
    host.flush(&mut app).unwrap();
    assert_eq!(host.presented(id).unwrap().style.transform.offset_x, 75.0);
    assert!(!host.presentation.is_animating());
}

#[test]
fn opacity_reaches_absent_default_and_springs_stay_valid() {
    let mut app = App::new(text("fade").opacity(0.0));
    let mut host = TestHost::default();
    host.flush(&mut app).unwrap();
    let id = app.elements()[0].0;
    // set_view replaces descriptions of the same type; a transaction scope supplies intent.
    let mut update = host.updates[0].clone();
    update.base_revision = 1;
    update.revision = 2;
    let mut element = app.elements()[0].1.clone();
    element.style.opacity = None;
    update.patches = vec![Patch::Update(id, element)];
    update.animations.insert(
        id,
        Transaction {
            animation: Some(Animation::spring()),
            disables_animations: false,
        },
    );
    host.apply(&update).unwrap();
    for _ in 0..120 {
        for (_, element) in host.advance_time(Duration::from_millis(16)) {
            element.validate().unwrap();
        }
    }
    assert!(!host.presentation.is_animating());
    assert_eq!(host.presented(id).unwrap().style.opacity, None);
}

#[test]
fn transitions_keep_only_visual_lifetime_and_reinsert_with_fresh_identity() {
    use std::cell::Cell;
    let saved = Rc::new(RefCell::new(None));
    let stopped = Rc::new(Cell::new(0));
    let mut app = App::new(view_fn({
        let saved = saved.clone();
        let stopped = stopped.clone();
        move |cx| {
            let visible = cx.state("visible", || true);
            *saved.borrow_mut() = Some(visible.clone());
            let stopped = stopped.clone();
            Ok(v_stack().child(if cx.read(&visible) {
                view_fn(move |cx| {
                    let stopped = stopped.clone();
                    cx.effect("lifetime", (), move |_| {
                        let stopped = stopped.clone();
                        Box::new(move || stopped.set(stopped.get() + 1))
                    });
                    Ok(text("notice")
                        .on_press(|_| {})
                        .transition(Transition::opacity()))
                })
                .key("notice")
                .into_any()
            } else {
                Empty.into_any()
            }))
        }
    }));
    let mut host = TestHost::default();
    host.flush(&mut app).unwrap();
    let old = app
        .elements()
        .into_iter()
        .find(|(_, e)| e.text == "notice")
        .unwrap()
        .0;
    assert!(!host.presentation.is_animating());
    let visible = saved.borrow().clone().unwrap();
    with_animation(
        app.runtime_mut(),
        Animation::linear(Duration::from_secs(1)),
        |rt| visible.set(rt, false),
    );
    drop(app.prepare().unwrap().unwrap());
    assert_eq!(stopped.get(), 0);
    assert!(!host.presentation.is_animating());
    host.flush(&mut app).unwrap();
    assert_eq!(stopped.get(), 1);
    assert!(!app.dispatch(old, refract_core::Event::Press));
    assert!(host.presentation.exit_effect(old, host.time).is_some());
    host.advance_time(Duration::from_millis(500));
    assert_eq!(
        host.presentation
            .exit_effect(old, host.time)
            .unwrap()
            .opacity,
        0.5
    );
    with_animation(
        app.runtime_mut(),
        Animation::linear(Duration::from_secs(1)),
        |rt| visible.set(rt, true),
    );
    host.flush(&mut app).unwrap();
    let new = app
        .elements()
        .into_iter()
        .find(|(_, e)| e.text == "notice")
        .unwrap()
        .0;
    assert_ne!(old, new);
    assert_eq!(host.presented(new).unwrap().style.opacity, Some(0.0));
    host.advance_time(Duration::from_secs(2));
    assert!(!host.presentation.is_animating());
    assert_eq!(stopped.get(), 1);
}

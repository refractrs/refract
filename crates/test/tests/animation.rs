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

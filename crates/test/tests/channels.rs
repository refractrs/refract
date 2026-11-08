use refract_core::{prelude::*, App, Patch};
use refract_test::TestHost;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

struct Model {
    result: Binding<String>,
}

#[test]
fn effect_channels_update_environment_consumers_and_reject_replaced_requests() {
    let requests = Rc::new(RefCell::new(None));
    let senders = Rc::new(RefCell::new(Vec::new()));
    let stable_evaluations = Rc::new(Cell::new(0));
    let mut app = App::new(view_fn({
        let requests = requests.clone();
        let senders = senders.clone();
        let stable_evaluations = stable_evaluations.clone();
        move |cx| {
            let request = cx.state("request", || 0);
            *requests.borrow_mut() = Some(request.clone());
            let request = cx.read(&request);
            let model = cx.remember("model", |rt| Model {
                result: State::new(rt, "loading".to_owned()).binding(),
            });
            cx.effect("load", request, {
                let model = model.clone();
                let senders = senders.clone();
                move |rt| {
                    let model = model.clone();
                    let (sender, subscription) = rt.channel(move |rt, result| {
                        model.result.set_if_changed(rt, result);
                    });
                    senders.borrow_mut().push(sender);
                    subscription.into_cleanup()
                }
            });
            let stable_evaluations = stable_evaluations.clone();
            Ok(v_stack()
                .child(view_fn(|cx| {
                    let model = cx.object::<Model>()?;
                    Ok(text(cx.read(&model.result)))
                }))
                .child(view_fn(move |_| {
                    stable_evaluations.set(stable_evaluations.get() + 1);
                    Ok(text("stable"))
                }))
                .environment_object(model))
        }
    }));
    // Speculative view evaluation must not start producers or register callbacks.
    drop(app.prepare().unwrap());
    assert!(senders.borrow().is_empty());
    let mut host = TestHost::default();
    host.flush(&mut app).unwrap();
    assert_eq!(senders.borrow().len(), 1);
    assert_eq!(host.text(), "loading stable");
    let initial_stable = stable_evaluations.get();
    let first = senders.borrow()[0].clone();
    std::thread::spawn({
        let first = first.clone();
        move || first.try_send("loaded".into()).unwrap()
    })
    .join()
    .unwrap();
    assert_eq!(host.text(), "loading stable");
    host.flush(&mut app).unwrap();
    assert_eq!(host.text(), "loaded stable");
    assert_eq!(stable_evaluations.get(), initial_stable);
    assert!(
        matches!(host.updates.last().unwrap().patches.as_slice(), [Patch::Update(_, element)] if element.text == "loaded")
    );
    first.try_send("loaded".into()).unwrap();
    assert!(
        !host.flush(&mut app).unwrap(),
        "equal result should not prepare a tree"
    );

    // Queue an old completion, then commit replacement before its delivery.
    first.try_send("stale".into()).unwrap();
    let request = requests.borrow().clone().unwrap();
    app.mutate(|rt| request.set(rt, 1));
    let pending = app.prepare().unwrap().unwrap();
    host.apply(&pending.update).unwrap();
    app.commit(pending).unwrap();
    assert_eq!(senders.borrow().len(), 2);
    assert_eq!(
        first.try_send("late".into()),
        Err(SendError::Closed("late".into()))
    );
    assert!(!host.flush(&mut app).unwrap());
    assert_eq!(host.text(), "loaded stable");
    let second = senders.borrow()[1].clone();
    second.try_send("new request".into()).unwrap();
    host.flush(&mut app).unwrap();
    assert_eq!(host.text(), "new request stable");

    app.set_view(Empty);
    host.flush(&mut app).unwrap();
    assert_eq!(
        second.try_send("unmounted".into()),
        Err(SendError::Closed("unmounted".into()))
    );
}

#[test]
fn disposal_closes_even_externally_owned_subscriptions_and_releases_models() {
    let mut app = App::new(Empty);
    let model = Rc::new(Model {
        result: State::new(app.runtime_mut(), "initial".to_owned()).binding(),
    });
    let weak = Rc::downgrade(&model);
    let (sender, subscription) = app.runtime_mut().channel(move |rt, result| {
        model.result.set(rt, result);
    });
    sender.try_send("queued".into()).unwrap();
    app.dispose();
    assert!(weak.upgrade().is_none());
    assert_eq!(
        sender.try_send("late".into()),
        Err(SendError::Closed("late".into()))
    );
    assert!(!app.runtime().has_pending_messages());
    assert_eq!(app.runtime().stats().attributes, 0);
    drop(subscription);
}

use refract_core::{prelude::*, App};
use refract_test::TestHost;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

#[test]
fn clean_memo_branches_bail_out_but_keep_dirty_descendants_and_effects() {
    let saved = Rc::new(RefCell::new(None));
    let bodies = Rc::new(Cell::new(0));
    let setups = Rc::new(Cell::new(0));
    let cleanups = Rc::new(Cell::new(0));
    let mut app = App::new(view_fn({
        let saved = saved.clone();
        let bodies = bodies.clone();
        let setups = setups.clone();
        let cleanups = cleanups.clone();
        move |cx| {
            let parent = cx.state("parent", || 0);
            let leaf = cx.state("leaf", || 0);
            *saved.borrow_mut() = Some((parent.clone(), leaf.clone()));
            let _ = cx.read(&parent);
            Ok(v_stack().children((0..100).map(|_| {
                let leaf = leaf.clone();
                let bodies = bodies.clone();
                let setups = setups.clone();
                let cleanups = cleanups.clone();
                view_fn(move |cx| {
                    bodies.set(bodies.get() + 1);
                    let setups = setups.clone();
                    let cleanups = cleanups.clone();
                    cx.effect("lifetime", (), move |_| {
                        setups.set(setups.get() + 1);
                        let cleanups = cleanups.clone();
                        Box::new(move || cleanups.set(cleanups.get() + 1))
                    });
                    Ok(text(cx.read(&leaf).to_string()))
                })
                .memo(())
            })))
        }
    }));
    let mut host = TestHost::default();
    host.flush(&mut app).unwrap();
    let (parent, leaf) = saved.borrow().clone().unwrap();
    assert_eq!(setups.get(), 100);
    for value in 1..=3 {
        app.mutate(|rt| parent.set(rt, value));
        host.flush(&mut app).unwrap();
        assert_eq!(bodies.get(), 100);
        assert!(app.stats().visited < 10, "{:?}", app.stats());
        assert_eq!(app.stats().patches, 0);
    }
    // Dirty descendants must still run when their parent also supplies equal memo props.
    app.mutate(|rt| {
        parent.set(rt, 4);
        leaf.set(rt, 9);
    });
    host.flush(&mut app).unwrap();
    assert_eq!(bodies.get(), 200);
    assert!(host.text().split_whitespace().all(|text| text == "9"));
    assert_eq!(setups.get(), 100);
    assert_eq!(cleanups.get(), 0);
    app.dispose();
    assert_eq!(cleanups.get(), 100);
}

#[test]
fn changed_memo_props_publish_new_handlers_without_resetting_local_state() {
    fn screen(prop: i32, result: Rc<Cell<i32>>) -> impl View {
        view_fn(move |cx| {
            let local = cx.state("local", || 0);
            let value = cx.read(&local);
            let result = result.clone();
            Ok(text(value.to_string())
                .on_press(move |cx| {
                    result.set(prop);
                    local.update(cx.runtime(), |value| *value += 1);
                })
                .label("change"))
        })
        .memo(prop)
    }
    let result = Rc::new(Cell::new(0));
    let mut app = App::new(screen(1, result.clone()));
    let mut host = TestHost::default();
    host.flush(&mut app).unwrap();
    host.press(&mut app, "change").unwrap();
    let identity = host.button("change");
    app.set_view(screen(1, result.clone()));
    host.flush(&mut app).unwrap();
    assert_eq!(app.stats().visited, 0);
    app.set_view(screen(2, result.clone()));
    host.flush(&mut app).unwrap();
    assert_eq!(host.button("change"), identity);
    assert_eq!(host.text(), "1");
    host.press(&mut app, "change").unwrap();
    assert_eq!(result.get(), 2);
    assert_eq!(host.text(), "2");
}

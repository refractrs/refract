use refract_core::prelude::*;
use refract_core::App;
use refract_test::TestHost;
use std::{cell::Cell, rc::Rc};

struct Inputs {
    name: Read<String>,
    count: Read<u32>,
}

#[test]
fn inputs_are_available_on_first_render_and_invalidate_only_readers() {
    let names = Rc::new(Cell::new(0));
    let counts = Rc::new(Cell::new(0));
    let (mut app, (name, count)) = App::with_inputs(|rt| {
        let name = State::new(rt, "initial".to_owned());
        let count = State::new(rt, 7_u32);
        let names = names.clone();
        let counts = counts.clone();
        let root = v_stack()
            .child(view_fn(move |cx| {
                let inputs = cx.object::<Inputs>()?;
                names.set(names.get() + 1);
                Ok::<_, ViewError>(text(cx.read(&inputs.name)))
            }))
            .child(view_fn(move |cx| {
                let inputs = cx.object::<Inputs>()?;
                counts.set(counts.get() + 1);
                Ok::<_, ViewError>(text(cx.read(&inputs.count).to_string()))
            }))
            .environment_object(Rc::new(Inputs {
                name: name.read_only(),
                count: count.read_only(),
            }));
        (root, (name, count))
    });
    let mut host = TestHost::default();
    host.flush(&mut app).unwrap();
    assert!(host.text().contains("initial"));
    assert!(host.text().contains('7'));
    let elements = host.elements.keys().copied().collect::<Vec<_>>();

    app.mutate(|rt| name.set_if_changed(rt, "changed".into()));
    host.flush(&mut app).unwrap();
    assert_eq!((names.get(), counts.get()), (2, 1));
    assert_eq!(host.elements.keys().copied().collect::<Vec<_>>(), elements);

    app.mutate(|rt| name.set_if_changed(rt, "changed".into()));
    assert!(app.prepare().unwrap().is_none());
    assert_eq!((names.get(), counts.get()), (2, 1));

    app.mutate(|rt| {
        name.set_if_changed(rt, "together".into());
        count.set_if_changed(rt, 8);
    });
    host.flush(&mut app).unwrap();
    assert!(host.text().contains("together"));
    assert!(host.text().contains('8'));
    assert_eq!((names.get(), counts.get()), (3, 2));
}

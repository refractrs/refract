use refract_core::{prelude::*, App};
use refract_test::TestHost;
use std::rc::Rc;

#[test]
fn context_reads_accept_read_only_and_wrapped_binding_handles() {
    let mut app = App::new(text(""));
    let state = State::new(app.runtime_mut(), String::from("one"));
    app.set_view(view_fn({
        let binding = state.binding();
        let wrapped = Rc::new(binding.clone());
        let read = binding.read_only();
        move |cx| {
            let value = cx.read(&binding);
            assert_eq!(cx.read(&wrapped), value);
            assert_eq!(cx.read(&read), value);
            assert_eq!(cx.with(&binding, String::len), value.len());
            assert_eq!(cx.with(&wrapped, String::len), value.len());
            assert_eq!(cx.with(&read, String::len), value.len());
            Ok(text(value))
        }
    }));
    let mut host = TestHost::default();
    host.flush(&mut app).unwrap();
    assert_eq!(host.text(), "one");
    state.set(app.runtime_mut(), String::from("second"));
    host.flush(&mut app).unwrap();
    assert_eq!(host.text(), "second");
}

#[test]
fn borrowed_context_reads_subscribe_and_accept_non_clone_values() {
    struct Label(String);

    let mut app = App::new(text(""));
    let items = State::new(app.runtime_mut(), vec![1]);
    let label = State::new(app.runtime_mut(), Label("first".into()));
    let number = State::new(app.runtime_mut(), 1);
    let derived = Derived::new(app.runtime_mut(), {
        let number = number.clone();
        move |rt| Label(format!("derived {}", number.get(rt)))
    });
    app.set_view(view_fn({
        let items = items.binding();
        let label = label.clone();
        move |cx| {
            let count = cx.with(&items, Vec::len);
            let label = cx.read_state_ref(&label);
            let derived = cx.read_derived_ref(&derived);
            Ok(text(format!("{count}:{}:{}", label.0, derived.0)))
        }
    }));
    let mut host = TestHost::default();
    host.flush(&mut app).unwrap();
    assert_eq!(host.text(), "1:first:derived 1");

    app.mutate(|rt| items.set(rt, vec![1, 2]));
    host.flush(&mut app).unwrap();
    assert_eq!(host.text(), "2:first:derived 1");

    app.mutate(|rt| label.set(rt, Label("second".into())));
    host.flush(&mut app).unwrap();
    assert_eq!(host.text(), "2:second:derived 1");

    app.mutate(|rt| number.set(rt, 2));
    host.flush(&mut app).unwrap();
    assert_eq!(host.text(), "2:second:derived 2");
}

use super::*;
use std::rc::Rc;

#[test]
fn conditional_dependencies_and_equal_outputs() {
    let mut rt = Runtime::new();
    let choose = State::new(&mut rt, true);
    let a = State::new(&mut rt, 2);
    let b = State::new(&mut rt, 2);
    let derived = Derived::distinct(&mut rt, {
        let choose = choose.clone();
        let a = a.clone();
        let b = b.clone();
        move |rt| {
            if *choose.get(rt) {
                *a.get(rt)
            } else {
                *b.get(rt)
            }
        }
    });
    let observer = rt.observer();
    let (_, deps) = rt.capture(|rt| *derived.get(rt));
    rt.accept(&observer, deps);
    choose.set(&mut rt, false);
    assert!(!observer.is_dirty(&rt));
    a.set(&mut rt, 8);
    assert_eq!(*derived.get(&rt), 2);
    b.set(&mut rt, 9);
    assert!(observer.is_dirty(&rt));
    assert_eq!(*derived.get(&rt), 9);
}
#[test]
fn batch_diamond_and_ownership() {
    let mut rt = Runtime::new();
    let a = State::new(&mut rt, 1);
    let b = Derived::new(&mut rt, {
        let a = a.clone();
        move |rt| *a.get(rt) * 2
    });
    let c = Derived::new(&mut rt, {
        let a = a.clone();
        move |rt| *a.get(rt) + 10
    });
    let d = Derived::new(&mut rt, {
        let b = b.clone();
        let c = c.clone();
        move |rt| *b.get(rt) + *c.get(rt)
    });
    rt.batch(|rt| {
        a.set(rt, 2);
        rt.batch(|rt| a.set(rt, 3));
        assert_eq!(*d.get(rt), 13);
    });
    assert_eq!(*d.get(&rt), 19);
    drop((a, b, c, d));
    rt.collect();
    assert_eq!(rt.stats().attributes, 0);
    assert_eq!(rt.stats().edges, 0);
}
#[test]
fn equal_writes_do_not_wake_and_subscriptions_are_staged() {
    let mut rt = Runtime::new();
    let a = State::new(&mut rt, 1);
    let obs = rt.observer();
    let (_, deps) = rt.capture(|rt| *a.get(rt));
    rt.accept(&obs, deps);
    let wakes = Rc::new(std::cell::Cell::new(0));
    rt.set_waker({
        let wakes = wakes.clone();
        move || wakes.set(wakes.get() + 1)
    });
    a.set_if_changed(&mut rt, 1);
    assert_eq!(wakes.get(), 0);
    rt.batch(|rt| {
        a.set(rt, 2);
        a.set(rt, 3);
    });
    assert_eq!(wakes.get(), 1);
    assert!(obs.is_dirty(&rt));
}
#[test]
#[should_panic(expected = "another or disposed runtime")]
fn foreign_handles_fail() {
    let mut a = Runtime::new();
    let b = Runtime::new();
    let value = State::new(&mut a, 1);
    value.get(&b);
}
#[test]
fn mapped_bindings_share_source() {
    let mut rt = Runtime::new();
    let s = State::new(&mut rt, (1, 2));
    let b = s.binding().map(|v| v.0, |v, n| v.0 = n);
    b.set(&mut rt, 3);
    assert_eq!(*s.get(&rt), (3, 2));
}

#[derive(PartialEq)]
struct CloneCounted {
    clones: Rc<std::cell::Cell<usize>>,
    values: Vec<i32>,
}

impl Clone for CloneCounted {
    fn clone(&self) -> Self {
        self.clones.set(self.clones.get() + 1);
        Self {
            clones: self.clones.clone(),
            values: self.values.clone(),
        }
    }
}

#[test]
fn read_only_handles_borrow_track_and_keep_state_alive() {
    let mut rt = Runtime::new();
    let clones = Rc::new(std::cell::Cell::new(0));
    let state = State::new(
        &mut rt,
        CloneCounted {
            clones: clones.clone(),
            values: vec![1, 2],
        },
    );
    let binding = state.binding();
    let read = binding.read_only();
    let observer = rt.observer();
    let label = String::from("length");
    let ((label, len), deps) = rt.capture(|rt| read.with(rt, |v| (label, v.values.len())));
    assert_eq!((label.as_str(), len), ("length", 2));
    assert_eq!(clones.get(), 0);
    rt.accept(&observer, deps);

    binding.set(
        &mut rt,
        CloneCounted {
            clones: clones.clone(),
            values: vec![3],
        },
    );
    assert!(observer.is_dirty(&rt));
    let retained = read.clone();
    drop((state, binding, read, observer));
    rt.collect();
    assert_eq!(rt.stats().attributes, 1);
    assert_eq!(retained.get(&rt).values, vec![3]);
    assert_eq!(clones.get(), 1);
    drop(retained);
    rt.collect();
    assert_eq!(rt.stats().attributes, 0);
}

#[test]
fn read_only_projections_support_non_clone_values_and_release_writers() {
    struct Selected(i32);

    let mut rt = Runtime::new();
    let state = State::new(&mut rt, (1, 2));
    let writer_resource = Rc::new(());
    let writer_lifetime = Rc::downgrade(&writer_resource);
    let selected = state.binding().map(
        |v| Selected(v.0),
        move |v, selected| {
            let _keep_alive = &writer_resource;
            v.0 = selected.0;
        },
    );
    let field = selected.map(|v| v.0, |v, n| v.0 = n);
    let read_selected = selected.read_only();
    let read_field = field.read_only();
    drop((selected, field));
    assert!(writer_lifetime.upgrade().is_none());
    assert_eq!(rt.stats().attributes, 1);

    let observer = rt.observer();
    let (value, deps) = rt.capture(|rt| read_field.get(rt));
    rt.accept(&observer, deps);
    assert_eq!(value, 1);
    state.set(&mut rt, (3, 2));
    assert!(observer.is_dirty(&rt));
    assert_eq!(read_selected.get(&rt).0, 3);
    assert_eq!(read_selected.with(&rt, |v| v.0), 3);
    assert_eq!(read_field.get(&rt), 3);
}

#[test]
fn borrowed_binding_reads_track_without_cloning_and_keep_owned_reads() {
    let mut rt = Runtime::new();
    let clones = Rc::new(std::cell::Cell::new(0));
    let state = State::new(
        &mut rt,
        CloneCounted {
            clones: clones.clone(),
            values: vec![1, 2],
        },
    );
    let binding = state.binding();
    let observer = rt.observer();
    let consumed = String::from("length");
    let ((label, len), deps) = rt.capture(|rt| binding.with(rt, |v| (consumed, v.values.len())));
    assert_eq!((label.as_str(), len), ("length", 2));
    assert_eq!(clones.get(), 0);
    rt.accept(&observer, deps);

    let owned = binding.get(&rt);
    assert_eq!(clones.get(), 1);
    clones.set(0);
    binding.set_if_changed(&mut rt, owned);
    assert_eq!(clones.get(), 0);
    assert!(!observer.is_dirty(&rt));
    binding.set(
        &mut rt,
        CloneCounted {
            clones: clones.clone(),
            values: vec![3],
        },
    );
    assert!(observer.is_dirty(&rt));
    assert_eq!(binding.with(&rt, |v| v.values.len()), 1);
    drop((binding, state, observer));
    rt.collect();
    assert_eq!(rt.stats().attributes, 0);
}

#[test]
fn chained_projections_borrow_parents_and_support_non_clone_outputs() {
    struct Selected(i32);
    let mut rt = Runtime::new();
    let clones = Rc::new(std::cell::Cell::new(0));
    let state = State::new(
        &mut rt,
        CloneCounted {
            clones: clones.clone(),
            values: vec![1, 2],
        },
    );
    let selected = state.binding().map(
        |v| Selected(v.values[0]),
        |v, selected| v.values[0] = selected.0,
    );
    let field = selected.map(|v| v.0, |v, n| v.0 = n);
    let observer = rt.observer();
    let (value, deps) = rt.capture(|rt| field.get(rt));
    rt.accept(&observer, deps);
    assert_eq!(value, 1);
    assert_eq!(selected.get(&rt).0, 1);
    assert_eq!(clones.get(), 0);
    field.set_if_changed(&mut rt, 1);
    assert_eq!(clones.get(), 0);
    assert!(!observer.is_dirty(&rt));

    field.update(&mut rt, |v| *v += 10);
    assert_eq!(
        clones.get(),
        1,
        "only the parent write should clone storage"
    );
    assert_eq!(state.get(&rt).values, [11, 2]);
    assert!(observer.is_dirty(&rt));
    assert_eq!(field.with(&rt, |v| *v), 11);
    drop((state, selected, field, observer));
    rt.collect();
    assert_eq!(rt.stats().attributes, 0);
}

#[test]
fn panicking_borrowed_read_leaves_storage_and_tracking_usable() {
    let mut rt = Runtime::new();
    let state = State::new(&mut rt, String::from("before"));
    let binding = state.binding();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        rt.capture(|rt| binding.with(rt, |_| panic!("reader failed")));
    }));
    assert!(result.is_err());
    binding.set(&mut rt, String::from("after"));
    let observer = rt.observer();
    let (len, deps) = rt.capture(|rt| binding.with(rt, String::len));
    assert_eq!(len, 5);
    rt.accept(&observer, deps);
    binding.set(&mut rt, String::new());
    assert!(observer.is_dirty(&rt));
}

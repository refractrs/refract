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

#[test]
fn wide_captures_and_observer_sets_deduplicate_without_losing_edges() {
    let mut rt = Runtime::new();
    let states: Vec<_> = (0..128).map(|_| State::new(&mut rt, 0)).collect();
    let observer = rt.observer();
    let (_, deps) = rt.capture(|rt| {
        for _ in 0..3 {
            for state in &states {
                state.get(rt);
            }
        }
    });
    assert_eq!(
        deps.sources,
        states.iter().map(State::id).collect::<Vec<_>>()
    );
    rt.accept(&observer, deps.clone());
    assert_eq!(rt.stats().edges, states.len());
    for index in [0, 31, 32, 127] {
        states[index].set(&mut rt, 1);
        assert!(observer.is_dirty(&rt));
        rt.accept(&observer, deps.clone());
    }

    let children: Vec<_> = (0..128).map(|_| rt.observer()).collect();
    let mut deps = Dependencies::default();
    for _ in 0..3 {
        for child in &children {
            deps.include(child);
        }
    }
    rt.accept(&observer, deps);
    assert_eq!(rt.stats().edges, children.len());
    let child = &children[127];
    let (_, deps) = rt.capture(|rt| *states[127].get(rt));
    rt.accept(child, deps);
    states[127].set(&mut rt, 2);
    assert!(observer.is_dirty(&rt));
}

#[test]
fn wide_equal_derived_results_still_replace_conditional_dependencies() {
    let mut rt = Runtime::new();
    let choose = State::new(&mut rt, true);
    let a: Vec<_> = (0..256).map(|_| State::new(&mut rt, 0)).collect();
    let b: Vec<_> = (0..256).map(|_| State::new(&mut rt, 0)).collect();
    let value = Derived::distinct(&mut rt, {
        let choose = choose.clone();
        let a = a.clone();
        let b = b.clone();
        move |rt| {
            let selected = if *choose.get(rt) { &a } else { &b };
            selected.iter().map(|state| *state.get(rt)).sum::<i32>()
        }
    });
    let observer = rt.observer();
    let (_, deps) = rt.capture(|rt| *value.get(rt));
    rt.accept(&observer, deps);
    choose.set(&mut rt, false);
    assert!(!observer.is_dirty(&rt));
    let computations = rt.stats().computations;
    a[63].set(&mut rt, 9);
    assert_eq!(rt.stats().computations, computations);
    b[63].set(&mut rt, 7);
    assert!(observer.is_dirty(&rt));
    assert_eq!(*value.get(&rt), 7);
}

#[test]
fn nested_capture_success_and_unwind_preserve_the_outer_read_set() {
    let mut rt = Runtime::new();
    let a = State::new(&mut rt, 0);
    let b = State::new(&mut rt, 0);
    let c = State::new(&mut rt, 0);
    let outer = rt.observer();
    let inner = rt.observer();
    let (inner_reads, outer_reads) = rt.capture(|rt| {
        a.get(rt);
        let (_, reads) = rt.capture(|rt| *b.get(rt));
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            rt.capture(|rt| {
                b.get(rt);
                panic!("discard nested capture");
            });
        }));
        assert!(panic.is_err());
        c.get(rt);
        reads
    });
    rt.accept(&outer, outer_reads.clone());
    rt.accept(&inner, inner_reads);
    assert_eq!(rt.stats().edges, 3);
    b.set(&mut rt, 1);
    assert!(inner.is_dirty(&rt));
    assert!(!outer.is_dirty(&rt));
    a.set(&mut rt, 1);
    assert!(outer.is_dirty(&rt));
    rt.accept(&outer, outer_reads);
    c.set(&mut rt, 1);
    assert!(outer.is_dirty(&rt));
}

#[test]
fn unchanged_dependency_fast_path_still_rejects_invalid_replacements() {
    use std::panic::{catch_unwind, AssertUnwindSafe};
    let mut rt = Runtime::new();
    let state = State::new(&mut rt, 0);
    let observer = rt.observer();
    let (_, original) = rt.capture(|rt| *state.get(rt));
    rt.accept(&observer, original.clone());
    rt.accept(&observer, original.clone());

    let mut foreign_rt = Runtime::new();
    let foreign = State::new(&mut foreign_rt, 0);
    let (_, foreign_reads) = foreign_rt.capture(|rt| *foreign.get(rt));
    assert!(catch_unwind(AssertUnwindSafe(|| rt.accept(&observer, foreign_reads))).is_err());
    let mut self_reads = Dependencies::default();
    self_reads.include(&observer);
    assert!(catch_unwind(AssertUnwindSafe(|| rt.accept(&observer, self_reads))).is_err());
    state.set(&mut rt, 1);
    assert!(
        observer.is_dirty(&rt),
        "failed replacements must retain accepted edges"
    );

    drop(state);
    rt.collect();
    assert_eq!(rt.stats().edges, 0);
    assert!(catch_unwind(AssertUnwindSafe(|| rt.accept(&observer, original))).is_err());
}

#[test]
fn mutation_context_survives_batches_derived_values_and_repeated_dirtiness() {
    let mut rt = Runtime::new();
    let a = State::new(&mut rt, 0);
    let b = State::new(&mut rt, 0);
    let derived = Derived::new(&mut rt, {
        let a = a.clone();
        move |rt| *a.get(rt) * 2
    });
    let oa = rt.observer();
    let ob = rt.observer();
    let aggregate = rt.observer();
    let (_, deps) = rt.capture(|rt| *derived.get(rt));
    rt.accept(&oa, deps);
    let (_, deps) = rt.capture(|rt| *b.get(rt));
    rt.accept(&ob, deps);
    let mut deps = Dependencies::default();
    deps.include(&oa);
    rt.accept(&aggregate, deps);
    rt.batch(|rt| {
        rt.with_mutation_context("animated", |rt| a.set(rt, 1));
        b.set(rt, 1);
    });
    assert_eq!(
        rt.observer_context(&oa).unwrap().get::<&str>(),
        Some(&"animated")
    );
    assert_eq!(rt.observer_context(&ob).unwrap().get::<&str>(), None);
    rt.take_dirty_observers();
    assert_eq!(
        rt.observer_context(&oa).unwrap().get::<&str>(),
        Some(&"animated")
    );
    rt.with_mutation_context("replacement", |rt| a.set(rt, 2));
    assert_eq!(
        rt.observer_context(&aggregate).unwrap().get::<&str>(),
        Some(&"replacement")
    );
    a.set(&mut rt, 3);
    assert_eq!(rt.observer_context(&oa).unwrap().get::<&str>(), None);
    rt.mark_clean(&oa);
    assert!(rt.observer_context(&oa).is_none());
}

#[test]
fn nested_mutation_context_restores_after_unwind() {
    let mut rt = Runtime::new();
    let a = State::new(&mut rt, 0);
    let observer = rt.observer();
    let (_, deps) = rt.capture(|rt| *a.get(rt));
    rt.accept(&observer, deps);
    rt.with_mutation_context("outer", |rt| {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            rt.with_mutation_context("inner", |rt| {
                a.set(rt, 1);
                panic!("test");
            });
        }));
        a.set(rt, 2);
    });
    assert_eq!(
        rt.observer_context(&observer).unwrap().get::<&str>(),
        Some(&"outer")
    );
    a.set(&mut rt, 3);
    assert_eq!(rt.observer_context(&observer).unwrap().get::<&str>(), None);
}

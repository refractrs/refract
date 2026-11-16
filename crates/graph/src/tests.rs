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

fn identity_chain(rt: &mut Runtime, source: &State<i32>, depth: usize) -> Derived<i32> {
    let mut value = Derived::new(rt, {
        let source = source.clone();
        move |rt| *source.get(rt)
    });
    for _ in 1..depth {
        value = Derived::new(rt, move |rt| *value.get(rt));
    }
    value
}

#[test]
fn uneven_diamonds_settle_once_without_invalidating_equal_outputs() {
    for depth in [2, 3, 16] {
        let mut rt = Runtime::new();
        let source = State::new(&mut rt, 1);
        let short = identity_chain(&mut rt, &source, 1);
        let long = identity_chain(&mut rt, &source, depth);
        let difference = Derived::distinct(&mut rt, move |rt| *short.get(rt) - *long.get(rt));
        let downstream = Derived::new(&mut rt, move |rt| *difference.get(rt));
        let observers: Vec<_> = (0..1000)
            .map(|_| {
                let observer = rt.observer();
                let (_, deps) = rt.capture(|rt| *downstream.get(rt));
                rt.accept(&observer, deps);
                observer
            })
            .collect();
        let wakes = Rc::new(std::cell::Cell::new(0));
        rt.set_waker({
            let wakes = wakes.clone();
            move || wakes.set(wakes.get() + 1)
        });
        for value in 2..10 {
            let before = rt.stats().computations;
            rt.batch(|rt| {
                source.set(rt, value - 1);
                rt.batch(|rt| source.set(rt, value));
            });
            assert_eq!(*downstream.get(&rt), 0);
            assert_eq!(rt.stats().computations - before, depth as u64 + 2);
            assert!(observers.iter().all(|observer| !observer.is_dirty(&rt)));
            assert!(rt.take_dirty_observers().is_empty());
        }
        assert_eq!(rt.stats().invalidations, 0);
        assert_eq!(wakes.get(), 0);
    }
}

#[test]
fn derived_computations_never_receive_mixed_diamond_inputs() {
    let mut rt = Runtime::new();
    let source = State::new(&mut rt, 1);
    let short = identity_chain(&mut rt, &source, 1);
    let long = identity_chain(&mut rt, &source, 3);
    let quotient = Derived::new(&mut rt, move |rt| {
        // Both branches represent the same value. A stale long branch would
        // make this denominator zero when the source changes from 1 to 2.
        10 / (*long.get(rt) - *short.get(rt) + 1)
    });
    source.set(&mut rt, 2);
    assert_eq!(*quotient.get(&rt), 10);
    assert_eq!(rt.stats().computations, 5);
}

#[test]
fn newly_selected_dependencies_settle_and_preserve_mutation_context() {
    let mut rt = Runtime::new();
    let choose = State::new(&mut rt, false);
    let source = State::new(&mut rt, 1);
    let selected = Rc::new(std::cell::RefCell::new(None::<Derived<i32>>));
    // Allocate the reader before its conditional dependency to rule out
    // creation order as a substitute for dependency-aware settlement.
    let value = Derived::distinct(&mut rt, {
        let choose = choose.clone();
        let selected = selected.clone();
        move |rt| {
            if *choose.get(rt) {
                *selected.borrow().as_ref().unwrap().get(rt)
            } else {
                0
            }
        }
    });
    *selected.borrow_mut() = Some(identity_chain(&mut rt, &source, 16));
    let observer = rt.observer();
    let (_, deps) = rt.capture(|rt| *value.get(rt));
    rt.accept(&observer, deps);
    rt.batch(|rt| {
        rt.with_mutation_context("selection", |rt| choose.set(rt, true));
        rt.with_mutation_context("source", |rt| source.set(rt, 2));
    });
    assert_eq!(*value.get(&rt), 2);
    assert_eq!(rt.stats().computations, 17);
    assert_eq!(
        rt.observer_context(&observer).unwrap().get::<&str>(),
        Some(&"source")
    );
    rt.mark_clean(&observer);
    source.set(&mut rt, 3);
    assert_eq!(*value.get(&rt), 3);
    assert!(observer.is_dirty(&rt));
    assert_eq!(rt.observer_context(&observer).unwrap().get::<&str>(), None);
    choose.set(&mut rt, false);
    rt.mark_clean(&observer);
    source.set(&mut rt, 4);
    assert_eq!(*value.get(&rt), 0);
    assert!(!observer.is_dirty(&rt));
}

#[test]
fn conditional_dependencies_can_reverse_direction() {
    let mut rt = Runtime::new();
    let choose = State::new(&mut rt, true);
    let selector = Derived::new(&mut rt, {
        let choose = choose.clone();
        move |rt| *choose.get(rt)
    });
    let source = State::new(&mut rt, 1);
    let later = Rc::new(std::cell::RefCell::new(None::<Derived<i32>>));
    let a = Derived::new(&mut rt, {
        let choose = selector.clone();
        let source = source.clone();
        let later = later.clone();
        move |rt| {
            if *choose.get(rt) {
                *source.get(rt)
            } else {
                *later.borrow().as_ref().unwrap().get(rt)
            }
        }
    });
    let b = Derived::new(&mut rt, {
        let choose = selector.clone();
        let source = source.clone();
        let a = a.clone();
        move |rt| {
            if *choose.get(rt) {
                *a.get(rt)
            } else {
                *source.get(rt)
            }
        }
    });
    *later.borrow_mut() = Some(b.clone());
    for value in 2..10 {
        choose.set(&mut rt, value % 2 == 1);
        source.set(&mut rt, value);
        assert_eq!(*a.get(&rt), value);
        assert_eq!(*b.get(&rt), value);
    }
    later.borrow_mut().take();
}

#[test]
fn long_chains_settle_without_recursive_dependency_walks() {
    let mut rt = Runtime::new();
    let source = State::new(&mut rt, 1);
    let last = identity_chain(&mut rt, &source, 10_000);
    source.set(&mut rt, 2);
    assert_eq!(*last.get(&rt), 2);
    assert_eq!(rt.stats().computations, 10_000);
    drop((source, last));
    rt.collect();
    assert_eq!(rt.stats().attributes, 0);
}

#[test]
fn panicking_computation_discards_staged_values_and_can_retry() {
    let mut rt = Runtime::new();
    let source = State::new(&mut rt, 1);
    let upstream = identity_chain(&mut rt, &source, 3);
    let fail = Rc::new(std::cell::Cell::new(false));
    let value = Derived::new(&mut rt, {
        let fail = fail.clone();
        let upstream = upstream.clone();
        move |rt| {
            let value = *upstream.get(rt);
            assert!(!fail.get(), "computation failed");
            value
        }
    });
    let observer = rt.observer();
    let (_, deps) = rt.capture(|rt| *value.get(rt));
    rt.accept(&observer, deps);
    fail.set(true);
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| source.set(&mut rt, 2))).is_err()
    );
    assert_eq!(*source.get(&rt), 2);
    assert_eq!(*upstream.get(&rt), 1);
    assert_eq!(*value.get(&rt), 1);
    assert!(!observer.is_dirty(&rt));
    fail.set(false);
    rt.batch(|_| {});
    assert_eq!(*upstream.get(&rt), 2);
    assert_eq!(*value.get(&rt), 2);
    assert!(observer.is_dirty(&rt));
}

#[test]
fn dynamic_cycles_report_the_dependency_and_allow_retry() {
    let mut rt = Runtime::new();
    let choose = State::new(&mut rt, false);
    let recursive = Rc::new(std::cell::RefCell::new(None::<Derived<i32>>));
    let value = Derived::new(&mut rt, {
        let choose = choose.clone();
        let recursive = recursive.clone();
        move |rt| {
            if *choose.get(rt) {
                *recursive.borrow().as_ref().unwrap().get(rt)
            } else {
                1
            }
        }
    });
    *recursive.borrow_mut() = Some(value.clone());
    let error =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| choose.set(&mut rt, true)))
            .unwrap_err();
    let message = error.downcast_ref::<String>().unwrap();
    assert!(message.contains("attribute graph dependency cycle at AttributeId("));
    choose.set(&mut rt, false);
    assert_eq!(*value.get(&rt), 1);
    recursive.borrow_mut().take();
}

#[test]
fn deep_dependencies_with_shared_inputs_do_not_recurse_through_computations() {
    let mut rt = Runtime::new();
    let source = State::new(&mut rt, 0);
    let shared = identity_chain(&mut rt, &source, 1);
    let mut last = shared.clone();
    for _ in 0..10_000 {
        let shared = shared.clone();
        last = Derived::new(&mut rt, move |rt| *shared.get(rt) + *last.get(rt));
    }
    source.set(&mut rt, 1);
    assert_eq!(*last.get(&rt), 10_001);
    assert_eq!(rt.stats().computations, 10_001);
    drop((last, shared, source));
    rt.collect();
    assert_eq!(rt.stats().attributes, 0);
}

#[test]
fn settlement_skips_clean_ancestor_subgraphs() {
    for depth in [10, 1000, 10_000] {
        let mut rt = Runtime::new();
        let stable = State::new(&mut rt, 7);
        let clean = identity_chain(&mut rt, &stable, depth);
        let source = State::new(&mut rt, 0);
        let changing = identity_chain(&mut rt, &source, 4);
        let joined = Derived::new(&mut rt, move |rt| *changing.get(rt) + *clean.get(rt));
        for value in 1..=3 {
            let validations = rt.validations;
            source.set(&mut rt, value);
            assert_eq!(*joined.get(&rt), value + 7);
            assert_eq!(rt.validations - validations, 5);
        }
        // The skipped branch must still participate when its own source changes.
        stable.set(&mut rt, 8);
        assert_eq!(*joined.get(&rt), 11);
    }
}

#[test]
fn equal_outputs_stop_validation_of_downstream_chains() {
    let mut rt = Runtime::new();
    let source = State::new(&mut rt, 0);
    let mut last = Derived::distinct(&mut rt, {
        let source = source.clone();
        move |rt| *source.get(rt) % 2
    });
    for _ in 0..10_000 {
        last = Derived::new(&mut rt, move |rt| *last.get(rt));
    }
    for value in [2, 4, 6] {
        let validations = rt.validations;
        source.set(&mut rt, value);
        assert_eq!(*last.get(&rt), 0);
        assert_eq!(rt.validations - validations, 1);
    }
    source.set(&mut rt, 7);
    assert_eq!(*last.get(&rt), 1);
}

#[test]
fn settlement_retry_restores_nodes_still_waiting_in_the_work_queue() {
    let mut rt = Runtime::new();
    let source = State::new(&mut rt, 0);
    let fail = Rc::new(std::cell::Cell::new(false));
    let first = Derived::new(&mut rt, {
        let source = source.clone();
        let fail = fail.clone();
        move |rt| {
            assert!(!fail.get(), "computation failed");
            *source.get(rt)
        }
    });
    let second = identity_chain(&mut rt, &source, 4);
    fail.set(true);
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| source.set(&mut rt, 1))).is_err()
    );
    fail.set(false);
    rt.batch(|_| {});
    assert_eq!(*first.get(&rt), 1);
    assert_eq!(*second.get(&rt), 1);
    assert_eq!(rt.stats().computations, 5);
}

#[test]
fn changed_dependency_levels_reach_existing_readers() {
    let mut rt = Runtime::new();
    let select = State::new(&mut rt, false);
    let source = State::new(&mut rt, 0);
    let later = Rc::new(std::cell::RefCell::new(None::<Derived<i32>>));
    let selected = Derived::distinct(&mut rt, {
        let select = select.clone();
        let later = later.clone();
        move |rt| {
            if *select.get(rt) {
                *later.borrow().as_ref().unwrap().get(rt)
            } else {
                0
            }
        }
    });
    let first_reader = Derived::new(&mut rt, {
        let selected = selected.clone();
        move |rt| *selected.get(rt)
    });
    let last_reader = Derived::new(&mut rt, move |rt| *first_reader.get(rt));
    *later.borrow_mut() = Some(identity_chain(&mut rt, &source, 64));
    // The equal result changes topology without scheduling either reader.
    select.set(&mut rt, true);
    let before = rt.stats().computations;
    source.set(&mut rt, 1);
    assert_eq!(*last_reader.get(&rt), 1);
    assert_eq!(rt.stats().computations - before, 67);
    // Retire nodes while their level updates are still pending.
    *later.borrow_mut() = Some(identity_chain(&mut rt, &source, 128));
    select.set(&mut rt, true);
    drop((selected, last_reader));
    later.borrow_mut().take();
    rt.collect();
    source.set(&mut rt, 2);
    assert_eq!(rt.stats().attributes, 2);
}

#[test]
fn mixed_depth_conditional_branches_remain_consistent_across_batches() {
    let mut rt = Runtime::new();
    let source = State::new(&mut rt, 0);
    let choose = State::new(&mut rt, false);
    let mut nodes = vec![identity_chain(&mut rt, &source, 1)];
    let mut seed = 0x1234_5678u64;
    for index in 1..64 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let short = nodes[(seed as usize) % index].clone();
        let long = nodes[index - 1].clone();
        nodes.push(Derived::distinct(&mut rt, {
            let choose = choose.clone();
            move |rt| {
                if *choose.get(rt) {
                    *short.get(rt)
                } else {
                    *long.get(rt)
                }
            }
        }));
    }
    let difference = Derived::distinct(&mut rt, {
        let last = nodes.last().unwrap().clone();
        let source = source.clone();
        move |rt| *last.get(rt) - *source.get(rt)
    });
    let observer = rt.observer();
    let (_, deps) = rt.capture(|rt| *difference.get(rt));
    rt.accept(&observer, deps);
    for value in 1..=32 {
        let computations = rt.stats().computations;
        rt.batch(|rt| {
            choose.set(rt, value % 2 == 0);
            source.set(rt, value);
        });
        assert!(nodes.iter().all(|node| *node.get(&rt) == value));
        assert_eq!(*difference.get(&rt), 0);
        assert!(!observer.is_dirty(&rt));
        assert_eq!(rt.stats().computations - computations, 65);
    }
}

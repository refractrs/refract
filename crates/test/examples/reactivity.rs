//! Repeatable reactivity experiments, including construction that UpdateStats cannot see.
//!
//! Run: cargo run -p refract-test --example reactivity --release --offline
//! Check behavior: cargo test -p refract-test --example reactivity --offline
//! Scaling cases: cargo run -p refract-test --example reactivity --release --offline -- --scaling
//! Timings cover mutation, prepare, and commit; exclude initial mount and TestHost::apply,
//! which clones its entire element map. These are core microbenchmarks, not frame timings.
//!
//! The sibling cases compare leaf reads, parent reads, per-row memoization, and
//! eager/deferred memoized branches. Only deferred construction or leaf-local
//! reads avoid reconstructing all unchanged rows. Memo props must include any
//! changing nonreactive inputs; () is appropriate for this static branch only.
//!
//! Selection cases compare mapped bindings, retained distinct projections, and
//! independent state cells. OneRow changes one item in a collection: distinct
//! projections suppress other row bodies but still recompute each projection.
//! Split cells avoid that fan-out. All collection cases preallocate both storage
//! forms so only the update path changes; mounting and memory are not compared.
//!
//! Counters describe work, not complexity proofs: visited excludes memo checks
//! and child matching. A wide parent still does O(width) matching with MemoEach.
//!
//! --scaling adds deep wrapper chains (the width column is their depth), one
//! body reading many independent attributes, and wider memoized sibling sets.
//! DeepSharedAncestors updates eight leaves under the same wrapper chain.

use refract_core::{prelude::*, App, UpdateStats};
use refract_test::TestHost;
use std::{cell::Cell, rc::Rc, time::Instant};

#[derive(Clone, Default)]
struct Counts {
    constructors: Rc<Cell<usize>>,
    bodies: Rc<Cell<usize>>,
    projections: Rc<Cell<usize>>,
}

impl Counts {
    fn reset(&self) {
        self.constructors.set(0);
        self.bodies.set(0);
        self.projections.set(0);
    }
}

fn bump(count: &Cell<usize>) {
    count.set(count.get() + 1);
}

#[derive(Clone, Copy, Debug)]
enum Siblings {
    LeafRead,
    ParentRead,
    MemoEach,
    MemoBranchEager,
    MemoBranchDeferred,
}

fn rows(width: usize, counts: &Counts, memo_each: bool) -> impl View {
    v_stack().children((0..width).map(|_| {
        bump(&counts.constructors);
        let counts = counts.clone();
        let child = view_fn(move |_| {
            bump(&counts.bodies);
            Ok(text("stable"))
        });
        if memo_each {
            child.memo(()).into_any()
        } else {
            child.into_any()
        }
    }))
}

type Mutation = dyn Fn(&mut Runtime, usize);

struct Fixture {
    app: App,
    host: TestHost,
    counts: Counts,
    write: Box<Mutation>,
}

#[derive(Debug)]
struct Sample {
    constructors: usize,
    bodies: usize,
    projections: usize,
    stats: UpdateStats,
    core_us: f64,
}

impl Fixture {
    fn mounted(mut self) -> Self {
        self.host.flush(&mut self.app).unwrap();
        self.host.updates.clear();
        self
    }

    fn step(&mut self, value: usize) -> Sample {
        self.counts.reset();
        let start = Instant::now();
        self.app.mutate(|rt| (self.write)(rt, value));
        let pending = self.app.prepare().unwrap();
        let mut elapsed = start.elapsed();
        // Do not reuse App::stats() when no update occurs: it reports the last commit.
        let stats = pending.as_ref().map(|p| p.update.stats).unwrap_or_default();
        if let Some(pending) = pending {
            self.host.apply(&pending.update).unwrap();
            let start = Instant::now();
            self.app.commit(pending).unwrap();
            elapsed += start.elapsed();
        }
        self.host.updates.clear();
        Sample {
            constructors: self.counts.constructors.get(),
            bodies: self.counts.bodies.get(),
            projections: self.counts.projections.get(),
            stats,
            core_us: elapsed.as_secs_f64() * 1e6,
        }
    }
}

fn siblings(width: usize, strategy: Siblings) -> Fixture {
    let counts = Counts::default();
    let source = Rc::new(std::cell::RefCell::new(None::<Binding<usize>>));
    let app = App::new(view_fn({
        let source = source.clone();
        let counts = counts.clone();
        move |cx| {
            let value = cx.state("value", || 0usize);
            *source.borrow_mut() = Some(value.clone());
            if !matches!(strategy, Siblings::LeafRead) {
                let _ = cx.read(&value);
            }
            let leaf = view_fn(move |cx| Ok(text(cx.read(&value).to_string())));
            let branch = match strategy {
                Siblings::LeafRead | Siblings::ParentRead => rows(width, &counts, false).into_any(),
                Siblings::MemoEach => rows(width, &counts, true).into_any(),
                Siblings::MemoBranchEager => rows(width, &counts, false).memo(()).into_any(),
                Siblings::MemoBranchDeferred => {
                    let counts = counts.clone();
                    view_fn(move |_| Ok(rows(width, &counts, false)))
                        .memo(())
                        .into_any()
                }
            };
            Ok(v_stack().child(leaf).child(branch))
        }
    }));
    let host = TestHost::default();
    Fixture {
        app,
        host,
        counts,
        write: Box::new(move |rt, value| source.borrow().as_ref().unwrap().set(rt, value)),
    }
    .mounted()
}

#[derive(Clone, Copy, Debug)]
enum Selection {
    Mapped,
    Distinct,
    Split,
}

fn selection(width: usize, strategy: Selection, change_selected: bool) -> Fixture {
    let counts = Counts::default();
    let source = Rc::new(std::cell::RefCell::new(None::<Binding<(usize, usize)>>));
    let split = Rc::new(std::cell::RefCell::new(None::<Binding<usize>>));
    let app = App::new(view_fn({
        let counts = counts.clone();
        let source = source.clone();
        let split = split.clone();
        move |cx| {
            let pair = cx.state("pair", || (0usize, 0usize));
            let first = cx.state("first", || 0usize);
            let second = cx.state("second", || 0usize);
            *source.borrow_mut() = Some(pair.clone());
            *split.borrow_mut() = Some(if change_selected {
                first.clone()
            } else {
                second
            });
            Ok(v_stack().children((0..width).map(|_| {
                let pair = pair.clone();
                let first = first.clone();
                let counts = counts.clone();
                let mapped = pair.map(
                    {
                        let counts = counts.clone();
                        move |p| {
                            bump(&counts.projections);
                            p.0
                        }
                    },
                    |p, v| p.0 = v,
                );
                view_fn(move |cx| {
                    bump(&counts.bodies);
                    let value = match strategy {
                        Selection::Mapped => cx.read(&mapped),
                        Selection::Distinct => {
                            let selected = cx.remember("selected", |rt| {
                                let pair = pair.clone();
                                let counts = counts.clone();
                                Derived::distinct(rt, move |rt| {
                                    bump(&counts.projections);
                                    pair.with(rt, |p| p.0)
                                })
                            });
                            cx.read_derived(&selected)
                        }
                        Selection::Split => cx.read(&first),
                    };
                    Ok(text(value.to_string()))
                })
            })))
        }
    }));
    Fixture {
        app,
        host: TestHost::default(),
        counts,
        write: Box::new(move |rt, value| match strategy {
            Selection::Split => split.borrow().as_ref().unwrap().set(rt, value),
            _ => source.borrow().as_ref().unwrap().update(rt, |pair| {
                if change_selected {
                    pair.0 = value
                } else {
                    pair.1 = value
                }
            }),
        }),
    }
    .mounted()
}

fn measure(label: &str, width: usize, mut fixture: Fixture) {
    for value in 1..=10 {
        fixture.step(value);
    }
    let samples: Vec<_> = (11..=110).map(|value| fixture.step(value)).collect();
    let mut times: Vec<_> = samples.iter().map(|sample| sample.core_us).collect();
    times.sort_by(f64::total_cmp);
    let s = &samples[0];
    println!(
        "{label},{width},{},{},{},{},{},{},{:.2},{:.2}",
        s.constructors,
        s.bodies,
        s.projections,
        s.stats.evaluated,
        s.stats.visited,
        s.stats.patches,
        times[50],
        times[95]
    );
}

// Each row now selects a different item; one mutation changes exactly one row.
fn collection(width: usize, strategy: Selection) -> Fixture {
    let counts = Counts::default();
    let source = Rc::new(std::cell::RefCell::new(None::<Binding<Vec<usize>>>));
    let split = Rc::new(std::cell::RefCell::new(Vec::<Binding<usize>>::new()));
    let app = App::new(view_fn({
        let source = source.clone();
        let split = split.clone();
        let counts = counts.clone();
        move |cx| {
            let items = cx.state("items", || vec![0usize; width]);
            *source.borrow_mut() = Some(items.clone());
            let cells: Vec<_> = (0..width)
                .map(|index| cx.state(&format!("item-{index}"), || 0usize))
                .collect();
            *split.borrow_mut() = cells.clone();
            Ok(
                v_stack().children(cells.into_iter().enumerate().map(|(index, cell)| {
                    let counts = counts.clone();
                    let mapped = items.map(
                        {
                            let counts = counts.clone();
                            move |items| {
                                bump(&counts.projections);
                                items[index]
                            }
                        },
                        move |items, value| items[index] = value,
                    );
                    view_fn(move |cx| {
                        bump(&counts.bodies);
                        let value = match strategy {
                            Selection::Mapped => cx.read(&mapped),
                            Selection::Distinct => {
                                let selected = cx.remember("selected", |rt| {
                                    let mapped = mapped.clone();
                                    Derived::distinct(rt, move |rt| mapped.get(rt))
                                });
                                cx.read_derived(&selected)
                            }
                            Selection::Split => cx.read(&cell),
                        };
                        Ok(text(value.to_string()))
                    })
                })),
            )
        }
    }));
    Fixture {
        app,
        host: TestHost::default(),
        counts,
        write: Box::new(move |rt, value| {
            let index = value % width;
            match strategy {
                Selection::Split => split.borrow()[index].set(rt, value),
                _ => source
                    .borrow()
                    .as_ref()
                    .unwrap()
                    .update(rt, |items| items[index] = value),
            }
        }),
    }
    .mounted()
}

fn deep_tree(depth: usize, leaves: usize) -> Fixture {
    let counts = Counts::default();
    let source = Rc::new(std::cell::RefCell::new(None::<Binding<usize>>));
    let app = App::new(view_fn({
        let source = source.clone();
        let counts = counts.clone();
        move |cx| {
            let value = cx.state("value", || 0usize);
            *source.borrow_mut() = Some(value.clone());
            let mut branch = v_stack()
                .children((0..leaves).map(|_| {
                    let value = value.clone();
                    let counts = counts.clone();
                    view_fn(move |cx| {
                        bump(&counts.bodies);
                        Ok(text(cx.read(&value).to_string()))
                    })
                }))
                .into_any();
            for _ in 0..depth {
                let child = branch;
                branch = view_fn(move |_| Ok(child.clone())).into_any();
            }
            Ok(branch)
        }
    }));
    Fixture {
        app,
        host: TestHost::default(),
        counts,
        write: Box::new(move |rt, value| source.borrow().as_ref().unwrap().set(rt, value)),
    }
    .mounted()
}

fn many_dependencies(width: usize) -> Fixture {
    let counts = Counts::default();
    let source = Rc::new(std::cell::RefCell::new(None::<State<usize>>));
    let app = App::new(view_fn({
        let counts = counts.clone();
        let source = source.clone();
        move |cx| {
            let values = cx.remember("values", |rt| {
                (0..width)
                    .map(|_| State::new(rt, 0usize))
                    .collect::<Vec<_>>()
            });
            *source.borrow_mut() = Some(values[0].clone());
            bump(&counts.bodies);
            let sum: usize = values.iter().map(|value| *cx.read_state_ref(value)).sum();
            Ok(text(sum.to_string()))
        }
    }));
    Fixture {
        app,
        host: TestHost::default(),
        counts,
        write: Box::new(move |rt, value| source.borrow().as_ref().unwrap().set(rt, value)),
    }
    .mounted()
}

fn main() {
    println!("scenario,width,row_constructors,row_bodies,projections,evaluated,visited,patches,core_p50_us,core_p95_us");
    if std::env::args().any(|arg| arg == "--scaling") {
        for depth in [16, 64, 256] {
            measure("DeepLeaf", depth, deep_tree(depth, 1));
            measure("DeepSharedAncestors", depth, deep_tree(depth, 8));
        }
        for width in [10, 100, 1000, 10000] {
            measure("ManyDependencies", width, many_dependencies(width));
            measure("WideMemo", width, siblings(width, Siblings::MemoEach));
        }
        return;
    }
    for width in [10, 100, 1000] {
        for strategy in [
            Siblings::LeafRead,
            Siblings::ParentRead,
            Siblings::MemoEach,
            Siblings::MemoBranchEager,
            Siblings::MemoBranchDeferred,
        ] {
            measure(&format!("{strategy:?}"), width, siblings(width, strategy));
        }
        for strategy in [Selection::Mapped, Selection::Distinct, Selection::Split] {
            measure(
                &format!("{strategy:?}Unrelated"),
                width,
                selection(width, strategy, false),
            );
            measure(
                &format!("{strategy:?}Selected"),
                width,
                selection(width, strategy, true),
            );
            measure(
                &format!("{strategy:?}OneRow"),
                width,
                collection(width, strategy),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deep_updates_reach_every_dirty_leaf_without_running_ancestors() {
        for leaves in [1, 8] {
            let mut fixture = deep_tree(64, leaves);
            for value in 1..=3 {
                let sample = fixture.step(value);
                assert_eq!(sample.bodies, leaves);
                assert_eq!(sample.stats.evaluated, leaves * 2);
                assert_eq!(sample.stats.patches, leaves);
                assert!(fixture
                    .host
                    .text()
                    .split_whitespace()
                    .all(|text| text == value.to_string()));
            }
        }
    }

    #[test]
    fn wide_read_sets_keep_all_subscriptions() {
        let mut fixture = many_dependencies(1000);
        for value in 1..=3 {
            let sample = fixture.step(value);
            assert_eq!(sample.bodies, 1);
            assert_eq!(sample.stats.patches, 1);
            assert_eq!(fixture.host.text(), value.to_string());
        }
    }

    #[test]
    fn construction_and_evaluation_are_separate_costs() {
        for (strategy, constructors, bodies) in [
            (Siblings::LeafRead, 0, 0),
            (Siblings::ParentRead, 1000, 1000),
            (Siblings::MemoEach, 1000, 0),
            (Siblings::MemoBranchEager, 1000, 0),
            (Siblings::MemoBranchDeferred, 0, 0),
        ] {
            let mut fixture = siblings(1000, strategy);
            let initial_ids: std::collections::HashSet<_> =
                fixture.host.elements.keys().copied().collect();
            let sample = fixture.step(1);
            assert_eq!(sample.constructors, constructors, "{strategy:?}");
            assert_eq!(sample.bodies, bodies, "{strategy:?}");
            assert_eq!(sample.stats.patches, 1, "{strategy:?}");
            assert_eq!(fixture.host.text().split_whitespace().next(), Some("1"));
            assert_eq!(
                fixture
                    .host
                    .elements
                    .keys()
                    .copied()
                    .collect::<std::collections::HashSet<_>>(),
                initial_ids
            );
        }
    }

    #[test]
    fn selectors_suppress_bodies_but_still_compute_unrelated_changes() {
        for (strategy, bodies, projections) in [
            (Selection::Mapped, 1000, 1000),
            (Selection::Distinct, 0, 1000),
            (Selection::Split, 0, 0),
        ] {
            let mut fixture = selection(1000, strategy, false);
            let sample = fixture.step(1);
            assert_eq!(sample.bodies, bodies, "{strategy:?}");
            assert_eq!(sample.projections, projections, "{strategy:?}");
            assert_eq!(sample.stats.patches, 0);
            assert!(fixture.host.text().split_whitespace().all(|s| s == "0"));
        }
    }

    #[test]
    fn all_selection_strategies_publish_selected_changes() {
        for strategy in [Selection::Mapped, Selection::Distinct, Selection::Split] {
            let mut fixture = selection(100, strategy, true);
            let sample = fixture.step(1);
            assert_eq!(sample.bodies, 100);
            assert_eq!(sample.stats.patches, 100);
            assert!(fixture.host.text().split_whitespace().all(|s| s == "1"));
        }
    }

    #[test]
    fn one_row_changes_without_replacing_rows_or_losing_other_values() {
        for (strategy, bodies, projections) in [
            (Selection::Mapped, 1000, 1000),
            (Selection::Distinct, 1, 1000),
            (Selection::Split, 1, 0),
        ] {
            let mut fixture = collection(1000, strategy);
            for value in 1..=3 {
                let sample = fixture.step(value);
                assert_eq!(sample.bodies, bodies, "{strategy:?}");
                assert_eq!(sample.projections, projections, "{strategy:?}");
                assert_eq!(sample.stats.patches, 1);
                assert_eq!(sample.stats.created, 0);
                assert_eq!(sample.stats.removed, 0);
                let values: Vec<usize> = fixture
                    .host
                    .text()
                    .split_whitespace()
                    .map(|value| value.parse().unwrap())
                    .collect();
                for (index, item) in values.into_iter().enumerate() {
                    assert_eq!(item, if index <= value { index } else { 0 });
                }
            }
        }
    }
}

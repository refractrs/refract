//! Dependency-free propagation benchmarks; timings exclude setup and observer cleanup.
//! The observer_capture case also times accepting a freshly captured read set.
use refract_graph::{Derived, Observer, Runtime, State};
use std::{hint::black_box, time::Instant};

type Mutation = dyn Fn(&mut Runtime, i64);

struct Fixture {
    rt: Runtime,
    write: Box<Mutation>,
    observers: Vec<Observer>,
    values: Vec<Derived<i64>>,
    expected: fn(i64) -> i64,
}
fn chain(rt: &mut Runtime, source: &State<i64>, depth: usize) -> Derived<i64> {
    let mut value = Derived::new(rt, {
        let source = source.clone();
        move |rt| *source.get(rt)
    });
    for _ in 1..depth {
        value = Derived::new(rt, move |rt| *value.get(rt));
    }
    value
}
fn observe(rt: &mut Runtime, value: &Derived<i64>) -> Observer {
    let observer = rt.observer();
    let (_, deps) = rt.capture(|rt| *value.get(rt));
    rt.accept(&observer, deps);
    observer
}
fn fixture(case: &str, width: usize) -> Fixture {
    let mut rt = Runtime::new();
    let source = State::new(&mut rt, 0i64);
    let mut values = Vec::new();
    let mut observers = Vec::new();
    let mut expected: fn(i64) -> i64 = |v| v;
    match case {
        "chain" => values.push(chain(&mut rt, &source, width)),
        "fanout" => {
            for _ in 0..width {
                values.push(chain(&mut rt, &source, 1));
            }
        }
        "state_observers" => {
            for _ in 0..width {
                let observer = rt.observer();
                let (_, deps) = rt.capture(|rt| *source.get(rt));
                rt.accept(&observer, deps);
                observers.push(observer);
            }
        }
        "distinct_cutoff" => {
            let mut value = Derived::distinct(&mut rt, {
                let source = source.clone();
                move |rt| *source.get(rt) % 2
            });
            for _ in 0..width {
                value = Derived::new(&mut rt, move |rt| *value.get(rt));
            }
            values.push(value);
            expected = |_| 0;
        }
        "clean_branch_join" => {
            let stable = State::new(&mut rt, 7);
            let clean = chain(&mut rt, &stable, width);
            values.push(Derived::new(&mut rt, {
                let source = source.clone();
                move |rt| *source.get(rt) + *clean.get(rt)
            }));
            expected = |v| v + 7;
        }
        "diamond_observers" | "diamond_derived" | "diamond_changed" => {
            let short = chain(&mut rt, &source, 1);
            let long = chain(&mut rt, &source, 3);
            let changed = case == "diamond_changed";
            let joined = Derived::distinct(&mut rt, move |rt| {
                if changed {
                    *short.get(rt) + *long.get(rt)
                } else {
                    *short.get(rt) - *long.get(rt)
                }
            });
            expected = if changed { |v| v * 2 } else { |_| 0 };
            if case == "diamond_derived" {
                for _ in 0..width {
                    let joined = joined.clone();
                    let value = Derived::new(&mut rt, move |rt| *joined.get(rt));
                    observers.push(observe(&mut rt, &value));
                    values.push(value);
                }
            } else {
                for _ in 0..width {
                    observers.push(observe(&mut rt, &joined));
                }
                values.push(joined);
            }
        }
        "conditional" => {
            let short = chain(&mut rt, &source, 1);
            let long = chain(&mut rt, &source, width);
            values.push(Derived::distinct(&mut rt, {
                let source = source.clone();
                move |rt| {
                    if *source.get(rt) % 4 == 0 {
                        *short.get(rt)
                    } else {
                        *long.get(rt)
                    }
                }
            }));
        }
        "conditional_wide" => {
            let left: Vec<_> = (0..width).map(|_| State::new(&mut rt, 0i64)).collect();
            let right: Vec<_> = (0..width).map(|_| State::new(&mut rt, 0i64)).collect();
            values.push(Derived::new(&mut rt, {
                let source = source.clone();
                move |rt| {
                    let value = *source.get(rt);
                    let states = if value % 4 == 0 { &left } else { &right };
                    value + states.iter().map(|state| *state.get(rt)).sum::<i64>()
                }
            }));
        }
        "observer_capture" => {
            let sources: Vec<_> = (0..width).map(|_| State::new(&mut rt, 0i64)).collect();
            let observer = rt.observer();
            observers.push(observer.clone());
            return Fixture {
                rt,
                observers,
                values,
                expected,
                write: Box::new(move |rt, value| {
                    source.set(rt, value);
                    let (_, reads) = rt.capture(|rt| {
                        black_box(source.get(rt));
                        for source in &sources {
                            black_box(source.get(rt));
                        }
                    });
                    rt.accept(&observer, reads);
                }),
            };
        }
        "batched_roots" => {
            let sources: Vec<_> = (0..width).map(|_| State::new(&mut rt, 0i64)).collect();
            values.push(Derived::new(&mut rt, {
                let sources = sources.clone();
                move |rt| sources.iter().map(|s| *s.get(rt)).sum::<i64>() / sources.len() as i64
            }));
            observers.push(observe(&mut rt, &values[0]));
            return Fixture {
                rt,
                observers,
                values,
                expected,
                write: Box::new(move |rt, value| {
                    rt.batch(|rt| {
                        for source in &sources {
                            source.set(rt, value);
                        }
                    })
                }),
            };
        }
        _ => unreachable!(),
    }
    if observers.is_empty() && !values.is_empty() {
        observers.push(observe(&mut rt, values.last().unwrap()));
    }
    Fixture {
        rt,
        observers,
        values,
        expected,
        write: Box::new(move |rt, value| source.set(rt, value)),
    }
}
fn measure(case: &str, width: usize, iterations: i64) {
    assert!(
        width > 0 && iterations > 0,
        "width and iterations must be positive"
    );
    let mut fixture = fixture(case, width);
    let mut elapsed = 0u128;
    for n in 1..=32 {
        (fixture.write)(&mut fixture.rt, n * 2);
        for observer in &fixture.observers {
            fixture.rt.mark_clean(observer);
        }
    }
    let before = fixture.rt.stats();
    for n in 33..33 + iterations {
        let value = black_box(n * 2);
        let start = Instant::now();
        (fixture.write)(&mut fixture.rt, value);
        elapsed += start.elapsed().as_nanos();
        for derived in &fixture.values {
            assert_eq!(*derived.get(&fixture.rt), (fixture.expected)(value));
        }
        for observer in &fixture.observers {
            fixture.rt.mark_clean(observer);
        }
    }
    let after = fixture.rt.stats();
    println!(
        "{case},{width},{iterations},{:.3},{:.3},{:.3},{},{}",
        elapsed as f64 / iterations as f64 / 1000.0,
        (after.computations - before.computations) as f64 / iterations as f64,
        (after.invalidations - before.invalidations) as f64 / iterations as f64,
        after.attributes,
        after.edges
    );
}
fn main() {
    let args: Vec<_> = std::env::args()
        .skip(1)
        .filter(|arg| arg != "--bench")
        .collect();
    if let [case, width, iterations] = args.as_slice() {
        measure(case, width.parse().unwrap(), iterations.parse().unwrap());
        return;
    }
    assert!(
        args.is_empty(),
        "usage: propagation [SCENARIO WIDTH ITERATIONS]"
    );
    println!("scenario,width,iterations,us_per_update,computations_per_update,invalidations_per_update,attributes,edges");
    for width in [10, 1000] {
        for case in ["chain", "fanout", "clean_branch_join", "conditional"] {
            measure(case, width, 512);
        }
    }
    for width in [1000, 10000] {
        measure("distinct_cutoff", width, 512);
    }
    for case in [
        "state_observers",
        "diamond_observers",
        "diamond_derived",
        "diamond_changed",
    ] {
        measure(case, 1000, 512);
    }
    for width in [128, 1024] {
        measure("batched_roots", width, 512);
        measure("conditional_wide", width, 512);
        measure("observer_capture", width, 512);
    }
}

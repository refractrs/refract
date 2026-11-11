use refract_core::{prelude::*, App};
use refract_test::TestHost;
use std::{cell::RefCell, rc::Rc};

#[test]
fn dirty_branches_join_marked_ancestors_after_discarded_preparation() {
    let saved = Rc::new(RefCell::new(Vec::<Binding<usize>>::new()));
    let mut app = App::new(view_fn({
        let saved = saved.clone();
        move |cx| {
            let values: Vec<_> = (0..16)
                .map(|index| cx.state(&index.to_string(), || 0usize))
                .collect();
            *saved.borrow_mut() = values.clone();
            Ok(v_stack().children(values.chunks(4).map(|values| {
                let leaves = v_stack()
                    .children(values.iter().map(|value| {
                        let value = value.clone();
                        view_fn(move |cx| Ok(text(cx.read(&value).to_string())))
                    }))
                    .into_any();
                view_fn(move |_| Ok(leaves.clone()))
            })))
        }
    }));
    let mut host = TestHost::default();
    host.flush(&mut app).unwrap();
    let values = saved.borrow().clone();
    let mut expected = vec![0usize; 16];
    // Different dirty observer iteration orders must build the same set of paths.
    for round in 1..=10 {
        app.mutate(|rt| {
            for index in [0, 5, 15] {
                values[index].set(rt, round);
                expected[index] = round;
            }
        });
        drop(app.prepare().unwrap().unwrap());
        // Add another branch while the previous paths are still recorded.
        app.mutate(|rt| values[10].set(rt, round));
        expected[10] = round;
        host.flush(&mut app).unwrap();
        let actual: Vec<usize> = host
            .text()
            .split_whitespace()
            .map(|value| value.parse().unwrap())
            .collect();
        assert_eq!(actual, expected);
        assert_eq!(app.stats().created, 0);
        assert_eq!(app.stats().removed, 0);
        assert!(app.prepare().unwrap().is_none());
    }
}

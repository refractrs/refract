use refract_core::{prelude::*, App, Description};
use refract_test::TestHost;

fn rows(specs: &[(Option<&str>, usize)]) -> impl View {
    v_stack().children(specs.iter().map(|(key, initial)| {
        let initial = *initial;
        let row = view_fn(move |cx| {
            let state = cx.state("value", || initial);
            let value = cx.read(&state);
            Ok(text(format!("{initial}:{value}"))
                .on_press(move |cx| state.update(cx.runtime(), |value| *value += 1))
                .label(initial.to_string()))
        });
        match key {
            Some(key) => row.key(*key).into_any(),
            None => row.into_any(),
        }
    }))
}

#[test]
fn ordered_matching_preserves_keyed_and_positional_state_through_suffix_edits() {
    let mut app = App::new(rows(&[
        (Some("a"), 10),
        (None, 20),
        (Some("c"), 30),
        (None, 40),
    ]));
    let mut host = TestHost::default();
    host.flush(&mut app).unwrap();
    for initial in [10, 20, 30, 40] {
        host.press(&mut app, &initial.to_string()).unwrap();
    }
    let a = host.button("10");
    let cases = [
        (
            vec![(Some("a"), 100), (None, 200), (Some("c"), 300), (None, 400)],
            "100:11 200:21 300:31 400:41",
        ),
        (
            vec![(Some("a"), 100), (Some("c"), 300), (None, 200), (None, 400)],
            "100:11 300:31 200:200 400:41",
        ),
        (
            vec![
                (Some("z"), 900),
                (Some("a"), 100),
                (Some("c"), 300),
                (None, 200),
                (None, 400),
            ],
            "900:900 100:11 300:31 200:41 400:400",
        ),
        (vec![(Some("z"), 900), (Some("a"), 100)], "900:900 100:11"),
        (
            vec![
                (Some("z"), 900),
                (Some("a"), 100),
                (Some("c"), 300),
                (None, 700),
            ],
            "900:900 100:11 300:300 700:700",
        ),
    ];
    for (specs, expected) in cases {
        app.set_view(rows(&specs));
        host.flush(&mut app).unwrap();
        assert_eq!(host.text(), expected);
        assert_eq!(host.button("100"), a);
    }

    let before = host.text();
    let revision = app.revision();
    app.set_view(rows(&[
        (Some("z"), 900),
        (Some("a"), 100),
        (Some("z"), 999),
    ]));
    assert!(host
        .flush(&mut app)
        .unwrap_err()
        .to_string()
        .contains("duplicate sibling key"));
    assert_eq!(host.text(), before);
    assert_eq!(app.revision(), revision);
    app.set_view(rows(&[(Some("z"), 900), (Some("a"), 100)]));
    host.flush(&mut app).unwrap();
    assert_eq!(host.text(), "900:900 100:11");
}

// External primitives can implement only describe(); metadata must retain its
// compatibility fallback, including host identity changes such as activation.
struct ExternalPrimitive {
    value: &'static str,
    interactive: bool,
}

impl View for ExternalPrimitive {
    fn describe(&self) -> Option<Description> {
        let primitive = text(self.value);
        if self.interactive {
            primitive.on_press(|_| {}).describe()
        } else {
            primitive.describe()
        }
    }
}

#[test]
fn custom_descriptions_keep_default_metadata_and_host_identity_rules() {
    let mut app = App::new(ExternalPrimitive {
        value: "one",
        interactive: false,
    });
    let mut host = TestHost::default();
    host.flush(&mut app).unwrap();
    let original = host.children[&None][0];
    app.set_view(ExternalPrimitive {
        value: "two",
        interactive: false,
    });
    host.flush(&mut app).unwrap();
    assert_eq!(host.children[&None], [original]);
    assert_eq!(host.text(), "two");
    app.set_view(ExternalPrimitive {
        value: "three",
        interactive: true,
    });
    host.flush(&mut app).unwrap();
    assert_ne!(host.children[&None], [original]);
    assert!(!host.elements.contains_key(&original));
    assert_eq!(host.text(), "three");
}

#[test]
fn state_declaration_validation_checks_names_and_types_without_requiring_order() {
    fn screen(mode: u8) -> impl View {
        view_fn(move |cx| {
            if mode == 5 {
                let second = cx.state("second", || 200);
                let first = cx.state("first", || 100);
                return Ok(text(format!("{}:{}", cx.read(&first), cx.read(&second))));
            }
            let first = if mode == 4 {
                cx.state("first", || String::from("wrong type"));
                0
            } else {
                let first = cx.state("first", || 1);
                cx.read(&first)
            };
            let second = if mode == 2 {
                0
            } else {
                let second = cx.state(if mode == 1 { "renamed" } else { "second" }, || 2);
                cx.read(&second)
            };
            if mode == 3 {
                cx.state("extra", || 3);
            }
            Ok(text(format!("{first}:{second}")))
        })
    }
    let mut app = App::new(screen(0));
    let mut host = TestHost::default();
    host.flush(&mut app).unwrap();
    let original = host.children[&None][0];
    let revision = app.revision();
    for mode in 1..=4 {
        app.set_view(screen(mode));
        assert!(
            host.flush(&mut app).is_err(),
            "invalid declaration mode {mode}"
        );
        assert_eq!(app.revision(), revision);
        assert_eq!(host.text(), "1:2");
    }
    app.set_view(screen(5));
    host.flush(&mut app).unwrap();
    assert_eq!(host.children[&None], [original]);
    assert_eq!(host.text(), "1:2");
}

use refract_core::{prelude::*, App, ElementId, ElementKind, Event, Patch};
use refract_test::TestHost;

fn form(multiline: bool) -> impl View {
    view_fn(move |cx| {
        let value = cx.state("value", || "initial".to_owned());
        Ok(field(value)
            .multiline(multiline)
            .key("editor")
            .padding(8.0))
    })
}

fn input_id(host: &TestHost) -> ElementId {
    *host
        .elements
        .iter()
        .find(|(_, e)| e.kind == ElementKind::TextInput)
        .unwrap()
        .0
}

#[test]
fn mode_changes_replace_only_the_control_and_preserve_its_binding() {
    let mut app = App::new(form(false));
    let mut host = TestHost::default();
    host.flush(&mut app).unwrap();
    let wrapper = host.children[&None][0];
    let mut id = input_id(&host);

    assert!(app.dispatch(id, Event::Input("edited".into())));
    host.flush(&mut app).unwrap();
    assert_eq!(input_id(&host), id, "ordinary edits retain the control");

    for multiline in [true, false] {
        app.set_view(form(multiline));
        // A discarded preparation must not retire the accepted control.
        drop(app.prepare().unwrap());
        assert!(app.elements().iter().any(|(current, _)| *current == id));
        host.flush(&mut app).unwrap();
        let next = input_id(&host);
        assert_ne!(next, id);
        assert_eq!(host.children[&None], [wrapper]);
        assert_eq!(host.elements[&next].multiline, multiline);
        assert_eq!(host.elements[&next].value, "edited");
        let patches = &host.updates.last().unwrap().patches;
        assert!(patches
            .iter()
            .any(|p| matches!(p, Patch::Remove(old) if *old == id)));
        assert!(patches
            .iter()
            .any(|p| matches!(p, Patch::Create(new, _) if *new == next)));
        assert!(!app.dispatch(id, Event::Input("stale composition".into())));
        assert!(!host.flush(&mut app).unwrap());
        id = next;
    }
}

#[test]
fn mode_is_part_of_root_element_identity_too() {
    let input = |multiline| {
        view().modify(false, move |e| {
            e.kind = ElementKind::TextInput;
            e.multiline = multiline;
        })
    };
    let mut app = App::new(input(false));
    let mut host = TestHost::default();
    host.flush(&mut app).unwrap();
    let before = input_id(&host);
    app.set_view(input(true));
    host.flush(&mut app).unwrap();
    assert_ne!(input_id(&host), before);
}

#[test]
fn multiline_is_only_valid_for_inputs_and_both_modes_are_interactive() {
    let mut invalid = App::new(view().modify(false, |e| e.multiline = true));
    assert!(invalid
        .prepare()
        .err()
        .unwrap()
        .0
        .contains("multiline requires a text input"));

    for multiline in [false, true] {
        let mut nested = App::new(
            h_stack()
                .alignment(Alignment::Center)
                .justify(Alignment::Center)
                .child(form(multiline))
                .on_press(|_| {}),
        );
        assert!(nested.prepare().err().unwrap().0.contains("interactive"));
    }
}

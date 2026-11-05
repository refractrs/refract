use super::*;
use refract::{App, ElementKind, Event, Patch};
use refract_test::TestHost;
use std::cell::{Cell, RefCell};

#[derive(Default)]
struct Timers {
    callbacks: RefCell<Vec<Tick>>,
    active: Rc<Cell<usize>>,
}

fn mounted() -> (App, TestHost, Rc<Timers>) {
    let timers = Rc::new(Timers::default());
    let mut app = App::new(application({
        let timers = timers.clone();
        move |_, tick| {
            timers.callbacks.borrow_mut().push(tick);
            timers.active.set(timers.active.get() + 1);
            let active = timers.active.clone();
            Box::new(move || active.set(active.get() - 1))
        }
    }));
    let mut host = TestHost::default();
    host.flush(&mut app).unwrap();
    (app, host, timers)
}

fn input(app: &mut App, host: &mut TestHost, label: &str, value: &str) {
    let id = *host
        .elements
        .iter()
        .find(|(_, e)| e.label.as_deref() == Some(label))
        .unwrap()
        .0;
    app.dispatch(id, Event::Input(value.into()));
    host.flush(app).unwrap();
}

fn tick(app: &mut App, host: &mut TestHost, timers: &Timers, index: usize) {
    let callback = timers.callbacks.borrow()[index].clone();
    app.mutate(|rt| callback(rt));
    host.flush(app).unwrap();
}

#[test]
fn chunks_update_one_text_element_without_rebuilding_the_shell() {
    let (mut app, mut host, timers) = mounted();
    input(&mut app, &mut host, "Message Refract", "Explain Rust state");
    host.press(&mut app, "Send message").unwrap();
    assert_eq!(timers.active.get(), 1);
    let ids: std::collections::HashSet<_> = host.elements.keys().copied().collect();
    tick(&mut app, &mut host, &timers, 0);
    let update = host.updates.last().unwrap();
    assert!(
        matches!(update.patches.as_slice(), [Patch::Update(_, e)] if e.kind == ElementKind::Text && e.text.starts_with("In this example"))
    );
    assert_eq!(
        update.stats.evaluated, 2,
        "only message_body and text evaluate"
    );
    assert_eq!(
        host.elements
            .keys()
            .copied()
            .collect::<std::collections::HashSet<_>>(),
        ids
    );
    for _ in 0..40 {
        tick(&mut app, &mut host, &timers, 0);
    }
    assert_eq!(timers.active.get(), 0, "completion removes timer");
    assert!(host.text().contains("application logic stays in Rust."));
}

#[test]
fn drafts_survive_switching_and_background_streams_remain_owned() {
    let (mut app, mut host, timers) = mounted();
    input(
        &mut app,
        &mut host,
        "Message Refract",
        "my unfinished draft",
    );
    host.press(&mut app, "Open A weekend away").unwrap();
    host.press(&mut app, "Find a change of scene").unwrap();
    assert_eq!(timers.active.get(), 1);
    host.press(&mut app, "Open A calmer kind of workspace")
        .unwrap();
    let editor = host
        .elements
        .values()
        .find(|e| e.kind == ElementKind::TextInput && e.multiline)
        .unwrap();
    assert_eq!(editor.value, "my unfinished draft");
    let revision = host.revision;
    tick(&mut app, &mut host, &timers, 0);
    assert_eq!(
        host.revision, revision,
        "hidden message body has no mounted reader"
    );
    host.press(&mut app, "Open Help me plan a slow weekend away")
        .unwrap();
    assert!(host.text().contains("Let’s leave a little"));
    assert_eq!(timers.active.get(), 1);
    host.press(&mut app, "Stop response").unwrap();
    assert_eq!(timers.active.get(), 0);
}

#[test]
fn stop_and_delete_reject_late_chunks_and_unmount_cleans_up() {
    let (mut app, mut host, timers) = mounted();
    input(&mut app, &mut host, "Message Refract", "one");
    host.press(&mut app, "Send message").unwrap();
    host.press(&mut app, "Stop response").unwrap();
    let text = host.text();
    tick(&mut app, &mut host, &timers, 0);
    assert_eq!(host.text(), text);
    input(&mut app, &mut host, "Message Refract", "two");
    host.press(&mut app, "Send message").unwrap();
    tick(&mut app, &mut host, &timers, 0);
    assert!(!host.text().contains("You asked: “one”"));
    host.press(&mut app, "Delete chat").unwrap();
    assert_eq!(timers.active.get(), 0);
    let text = host.text();
    tick(&mut app, &mut host, &timers, 1);
    assert_eq!(host.text(), text);
    host.press(&mut app, "Build something").unwrap();
    assert_eq!(timers.active.get(), 1);
    app.dispose();
    assert_eq!(timers.active.get(), 0);
}

#[test]
fn blank_submission_filtering_and_deleting_last_chat_preserve_invariants() {
    let mut rt = Runtime::default();
    let model = model::ChatModel::new(&mut rt);
    let first = model.conversations.get(&rt)[0].clone();
    first.draft.set(&mut rt, "  \n ".into());
    let count = first.messages.get(&rt).len();
    first.send(&mut rt);
    assert_eq!(first.messages.get(&rt).len(), count);
    for id in [1, 2, 3] {
        model.delete(&mut rt, model::ConversationId(id));
    }
    assert_eq!(model.conversations.get(&rt).len(), 1);
    assert_eq!(model.conversations.get(&rt)[0].id, *model.selected.get(&rt));

    let (mut app, mut host, _) = mounted();
    input(&mut app, &mut host, "Search conversations", "weekend");
    assert!(host
        .elements
        .values()
        .any(|e| e.label.as_deref() == Some("Open A weekend away")));
    assert!(!host
        .elements
        .values()
        .any(|e| e.label.as_deref() == Some("Open Designing with intention")));
}

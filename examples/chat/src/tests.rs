use super::*;
use refract::{App, ElementKind, Event, Patch};
use refract_test::TestHost;
use std::cell::{Cell, RefCell};

struct Worker {
    sender: MessageSender<service::StreamEvent>,
    pending: RefCell<std::collections::VecDeque<service::StreamEvent>>,
}

#[derive(Default)]
struct Workers {
    workers: RefCell<Vec<Worker>>,
    active: Rc<Cell<usize>>,
}

fn mounted() -> (App, TestHost, Rc<Workers>) {
    mounted_at("app://chat/")
}

fn mounted_at(url: &str) -> (App, TestHost, Rc<Workers>) {
    let (app, host, workers, _) = mounted_with_inputs_at(url);
    (app, host, workers)
}

fn mounted_with_inputs_at(url: &str) -> (App, TestHost, Rc<Workers>, InputHandles) {
    let workers = Rc::new(Workers::default());
    let (mut app, inputs) = App::with_inputs(|rt| {
        initialize_with_source(
            rt,
            Rc::new(ReplySource::new({
                let workers = workers.clone();
                move |prompt, sender| {
                    let chunks = service::chunks(&service::reply(&prompt));
                    let total = chunks.len();
                    workers.workers.borrow_mut().push(Worker {
                        sender,
                        pending: RefCell::new(
                            chunks
                                .into_iter()
                                .enumerate()
                                .map(|(i, text)| service::StreamEvent::Chunk {
                                    text,
                                    complete: i + 1 == total,
                                })
                                .collect(),
                        ),
                    });
                    workers.active.set(workers.active.get() + 1);
                    let active = workers.active.clone();
                    Ok(Box::new(move || active.set(active.get() - 1)))
                }
            })),
        )
    });
    let mut host = TestHost::default();
    host.install_surface(&mut app, "Chat", Some(url.parse().unwrap()));
    host.set_viewport(
        &mut app,
        Viewport {
            width: 1180.0,
            height: Some(880.0),
            scale_factor: 1.0,
        },
    )
    .unwrap();
    host.flush(&mut app).unwrap();
    (app, host, workers, inputs)
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

fn tick(app: &mut App, host: &mut TestHost, workers: &Workers, index: usize) {
    let list = workers.workers.borrow();
    let worker = &list[index];
    if let Some(event) = worker.pending.borrow_mut().pop_front() {
        let sender = worker.sender.clone();
        // Only owned payloads cross threads; graph state remains in the receiver.
        let result = std::thread::spawn(move || sender.try_send(event))
            .join()
            .unwrap();
        assert!(!matches!(result, Err(SendError::Full(_))));
    }
    host.flush(app).unwrap();
}

#[test]
fn chunks_update_one_text_element_without_rebuilding_the_shell() {
    let (mut app, mut host, workers) = mounted();
    input(&mut app, &mut host, "Message Refract", "Explain Rust state");
    host.press(&mut app, "Send message").unwrap();
    assert_eq!(workers.active.get(), 1);
    let ids: std::collections::HashSet<_> = host.elements.keys().copied().collect();
    tick(&mut app, &mut host, &workers, 0);
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
    for _ in 0..100 {
        tick(&mut app, &mut host, &workers, 0);
    }
    assert_eq!(
        workers.active.get(),
        0,
        "completion cancels producer and closes receiver"
    );
    assert!(host.text().contains("application logic stays in Rust."));
}

#[test]
fn drafts_survive_switching_and_background_streams_remain_owned() {
    let (mut app, mut host, workers) = mounted();
    input(
        &mut app,
        &mut host,
        "Message Refract",
        "my unfinished draft",
    );
    host.press(&mut app, "Open A weekend away").unwrap();
    host.press(&mut app, "Find a change of scene").unwrap();
    assert_eq!(workers.active.get(), 1);
    host.press(&mut app, "Open A calmer kind of workspace")
        .unwrap();
    let editor = host
        .elements
        .values()
        .find(|e| e.kind == ElementKind::TextInput && e.multiline)
        .unwrap();
    assert_eq!(editor.value, "my unfinished draft");
    let revision = host.revision;
    tick(&mut app, &mut host, &workers, 0);
    assert_eq!(
        host.revision, revision,
        "hidden message body has no mounted reader"
    );
    host.press(&mut app, "Open Help me plan a slow weekend away")
        .unwrap();
    assert!(host.text().contains("Let’s leave a little"));
    assert_eq!(workers.active.get(), 1);
    host.press(&mut app, "Stop response").unwrap();
    assert_eq!(workers.active.get(), 0);
}

#[test]
fn stop_and_delete_reject_late_chunks_and_unmount_cleans_up() {
    let (mut app, mut host, workers) = mounted();
    input(&mut app, &mut host, "Message Refract", "one");
    host.press(&mut app, "Send message").unwrap();
    host.press(&mut app, "Stop response").unwrap();
    let text = host.text();
    tick(&mut app, &mut host, &workers, 0);
    assert_eq!(host.text(), text);
    input(&mut app, &mut host, "Message Refract", "two");
    host.press(&mut app, "Send message").unwrap();
    tick(&mut app, &mut host, &workers, 0);
    assert!(!host.text().contains("You asked: “one”"));
    host.press(&mut app, "Delete Chat").unwrap();
    assert_eq!(workers.active.get(), 0);
    let text = host.text();
    tick(&mut app, &mut host, &workers, 1);
    assert_eq!(host.text(), text);
    host.press(&mut app, "Build something").unwrap();
    assert_eq!(workers.active.get(), 1);
    app.dispose();
    assert_eq!(workers.active.get(), 0);
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

#[test]
fn queued_completion_cannot_undo_stop_or_enter_a_replacement_request() {
    let (mut app, mut host, workers) = mounted();
    input(&mut app, &mut host, "Message Refract", "first request");
    host.press(&mut app, "Send message").unwrap();
    let old = workers.workers.borrow()[0].sender.clone();
    old.try_send(service::StreamEvent::Chunk {
        text: "STALE COMPLETION".into(),
        complete: true,
    })
    .unwrap();
    // Dispatch Stop before the host drains the already-queued result.
    host.press(&mut app, "Stop response").unwrap();
    assert!(host.text().contains("RESPONSE STOPPED"));
    assert!(!host.text().contains("STALE COMPLETION"));
    assert!(matches!(
        old.try_send(service::StreamEvent::Failed("late failure".into())),
        Err(SendError::Closed(_))
    ));
    input(&mut app, &mut host, "Message Refract", "second request");
    host.press(&mut app, "Send message").unwrap();
    tick(&mut app, &mut host, &workers, 1);
    assert!(!host.text().contains("STALE COMPLETION"));
    assert_eq!(workers.active.get(), 1);
}

#[test]
fn queued_chunks_are_ignored_on_delete_and_failures_release_the_composer() {
    let (mut app, mut host, workers) = mounted();
    input(&mut app, &mut host, "Message Refract", "delete me");
    host.press(&mut app, "Send message").unwrap();
    workers.workers.borrow()[0]
        .sender
        .try_send(service::StreamEvent::Chunk {
            text: "DELETED RESULT".into(),
            complete: true,
        })
        .unwrap();
    host.press(&mut app, "Delete Chat").unwrap();
    assert!(!host.text().contains("DELETED RESULT"));
    assert_eq!(workers.active.get(), 0);
    input(&mut app, &mut host, "Message Refract", "try again");
    host.press(&mut app, "Send message").unwrap();
    workers.workers.borrow()[1]
        .sender
        .try_send(service::StreamEvent::Failed("test failure".into()))
        .unwrap();
    host.flush(&mut app).unwrap();
    assert!(host.text().contains("RESPONSE FAILED"));
    assert!(host.text().contains("test failure"));
    assert_eq!(workers.active.get(), 0);
    input(&mut app, &mut host, "Message Refract", "retry");
    host.press(&mut app, "Send message").unwrap();
    assert_eq!(workers.active.get(), 1);
}

#[test]
fn failed_worker_start_is_rendered_without_leaving_a_live_request() {
    let mut app = App::with_inputs(|rt| {
        initialize_with_source(
            rt,
            Rc::new(ReplySource::new(|_, _| Err("cannot start worker".into()))),
        )
    })
    .0;
    let mut host = TestHost::default();
    host.install_surface(&mut app, "Chat", Some("app://chat/".parse().unwrap()));
    host.flush(&mut app).unwrap();
    input(&mut app, &mut host, "Message Refract", "hello");
    host.press(&mut app, "Send message").unwrap();
    host.flush(&mut app).unwrap();
    assert!(host.text().contains("cannot start worker"));
    assert!(host.text().contains("RESPONSE FAILED"));
    host.button("Send message");
}

#[test]
fn selection_updates_history_and_title_without_echoing_back_navigation() {
    let (mut app, mut host, _) = mounted();
    let surface = host.surface.as_ref().unwrap().surface();
    assert_eq!(
        surface.location.get(app.runtime()).unwrap().query(),
        Some("conversation=1")
    );
    host.press(&mut app, "Open A weekend away").unwrap();
    assert_eq!(
        surface.location.get(app.runtime()).unwrap().query(),
        Some("conversation=3")
    );
    assert_eq!(
        surface.title.get(app.runtime()),
        "A weekend away — Refract Chat"
    );
    host.go(&mut app, -1);
    host.flush(&mut app).unwrap();
    assert_eq!(
        surface.title.get(app.runtime()),
        "A calmer kind of workspace — Refract Chat"
    );
    host.go(&mut app, 1);
    host.flush(&mut app).unwrap();
    assert_eq!(
        surface.location.get(app.runtime()).unwrap().query(),
        Some("conversation=3")
    );
    assert_eq!(
        surface.title.get(app.runtime()),
        "A weekend away — Refract Chat"
    );
}

#[test]
fn narrow_viewport_collapses_sidebar_but_keeps_menu_accessible() {
    let (mut app, mut host, _) = mounted();
    host.set_viewport(
        &mut app,
        Viewport {
            width: 390.0,
            height: Some(800.0),
            scale_factor: 2.0,
        },
    )
    .unwrap();
    host.flush(&mut app).unwrap();
    assert!(!host
        .elements
        .values()
        .any(|element| element.label.as_deref() == Some("Search conversations")));
    host.press(&mut app, "Toggle sidebar").unwrap();
    assert!(host
        .elements
        .values()
        .any(|element| element.label.as_deref() == Some("Search conversations")));
    host.press(&mut app, "Open A weekend away").unwrap();
    assert!(!host
        .elements
        .values()
        .any(|element| element.label.as_deref() == Some("Search conversations")));
    assert!(host.text().contains("A weekend away"));
    host.set_viewport(
        &mut app,
        Viewport {
            width: 1180.0,
            height: Some(880.0),
            scale_factor: 1.0,
        },
    )
    .unwrap();
    host.flush(&mut app).unwrap();
    assert!(host
        .elements
        .values()
        .any(|element| element.label.as_deref() == Some("Search conversations")));
}

#[test]
fn deep_links_preserve_prefixes_and_unknown_conversations_replace_the_current_entry() {
    let (mut app, mut host, _) =
        mounted_at("https://example.com/apps/chat/?theme=dark&conversation=3#message");
    let surface = host.surface.as_ref().unwrap().surface();
    assert_eq!(
        surface.title.get(app.runtime()),
        "A weekend away — Refract Chat"
    );
    host.press(&mut app, "Open Designing with intention")
        .unwrap();
    let url = surface.location.get(app.runtime()).unwrap();
    assert_eq!(url.path(), "/apps/chat/");
    assert_eq!(url.fragment(), Some("message"));
    assert_eq!(url.query(), Some("theme=dark&conversation=2"));
    host.press(&mut app, "Delete Chat").unwrap();
    host.go(&mut app, -1);
    host.flush(&mut app).unwrap();
    assert_eq!(
        surface.title.get(app.runtime()),
        "A weekend away — Refract Chat"
    );

    let (app, mut host, _) = mounted_at("https://example.com/apps/chat/?conversation=999");
    let url = host
        .surface
        .as_ref()
        .unwrap()
        .surface()
        .location
        .get(app.runtime())
        .unwrap();
    assert_eq!(url.query(), Some("conversation=1"));
    assert!(
        host.history.go(-1).is_none(),
        "normalization must replace, not push"
    );
}

#[test]
fn host_theme_updates_preserve_drafts_nodes_and_active_streams() {
    let (mut app, mut host, workers, inputs) =
        mounted_with_inputs_at("https://refract.rs/examples/chat");
    input(&mut app, &mut host, "Message Refract", "Explain Rust state");
    host.press(&mut app, "Send message").unwrap();
    input(&mut app, &mut host, "Message Refract", "Keep this draft");
    let ids = host
        .elements
        .keys()
        .copied()
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(workers.active.get(), 1);

    app.mutate(|rt| inputs.dark.set_if_changed(rt, true));
    host.flush(&mut app).unwrap();
    assert_eq!(
        host.elements
            .keys()
            .copied()
            .collect::<std::collections::HashSet<_>>(),
        ids
    );
    assert!(host
        .elements
        .values()
        .any(|element| element.value == "Keep this draft"));
    assert!(host
        .elements
        .values()
        .any(|element| element.style.background
            == Some(crate::theme::Palette::for_dark(true).background)));
    assert_eq!(workers.active.get(), 1);
    let revision = app.revision();
    app.mutate(|rt| inputs.dark.set_if_changed(rt, true));
    host.flush(&mut app).unwrap();
    assert_eq!(app.revision(), revision);

    tick(&mut app, &mut host, &workers, 0);
    assert!(host.text().contains("In this example"));
    app.mutate(|rt| inputs.dark.set_if_changed(rt, false));
    host.flush(&mut app).unwrap();
    assert!(host
        .elements
        .values()
        .any(|element| element.value == "Keep this draft"));
    assert_eq!(workers.active.get(), 1);
}

#[test]
fn native_entry_initializes_theme_and_retains_drafts() {
    let mut app = App::new(application());
    let mut host = TestHost::default();
    host.install_surface(&mut app, "Chat", Some("app://chat/".parse().unwrap()));
    host.flush(&mut app).unwrap();
    assert!(host
        .elements
        .values()
        .any(|element| element.style.background == Some(BACKGROUND)));
    input(&mut app, &mut host, "Message Refract", "Native draft");
    host.press(&mut app, "Open A weekend away").unwrap();
    host.press(&mut app, "Open A calmer kind of workspace")
        .unwrap();
    assert!(host
        .elements
        .values()
        .any(|element| element.value == "Native draft"));
}

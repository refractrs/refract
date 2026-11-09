//! Effect-owned channels connect background producers to UI-local graph state.
use crate::model::{ChatModel, Conversation};
use refract::{prelude::*, Cleanup};
use std::rc::Rc;

#[derive(Debug)]
pub(crate) enum StreamEvent {
    Chunk {
        text: String,
        complete: bool,
    },
    // Browser scheduling failures; native startup errors are returned by launch.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    Failed(String),
}

type Launch = dyn Fn(String, MessageSender<StreamEvent>) -> Result<Cleanup, String>;

/// Injected so tests can control delivery without timers or network access.
pub(crate) struct ReplySource {
    launch: Rc<Launch>,
    active: std::cell::Cell<u32>,
}

impl ReplySource {
    pub(crate) fn new(
        launch: impl Fn(String, MessageSender<StreamEvent>) -> Result<Cleanup, String> + 'static,
    ) -> Self {
        Self {
            launch: Rc::new(launch),
            active: std::cell::Cell::new(0),
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn active_streams(&self) -> u32 {
        self.active.get()
    }
}

#[view]
pub(crate) fn streams(cx: &mut ViewContext<'_>) -> impl View {
    #[environment_object]
    let model: Rc<ChatModel>;
    let chats = cx.read_state(&model.conversations);
    // Owners are siblings of the visible pane: navigation never cancels a reply.
    fragment(chats.into_iter().map(|chat| {
        let key = chat.id.0.to_string();
        stream(chat).key(key)
    }))
}

#[view]
fn stream(cx: &mut ViewContext<'_>, chat: Rc<Conversation>) -> impl View {
    #[environment_object]
    let source: Rc<ReplySource>;
    let request = cx.read_state(&chat.request);
    let request_id = request.as_ref().map(|r| r.id);

    #[effect(id = "assistant-stream", deps = request_id, captures(request, chat, source))]
    let subscription = |rt| {
        let Some(request) = &request else {
            return Box::new(|| {}) as Cleanup;
        };
        let id = request.id;
        let receiver = chat.clone();
        let (sender, subscription) = rt.channel(move |rt, event| {
            // Commands invalidate request identity synchronously, before cleanup.
            // Already-queued chunks therefore cannot undo Stop or affect a new reply.
            match event {
                StreamEvent::Chunk { text, complete } => receiver.append(rt, id, &text, complete),
                StreamEvent::Failed(error) => receiver.fail(rt, id, error),
            }
        });
        match (source.launch)(request.prompt.clone(), sender) {
            Ok(cancel) => {
                source.active.set(source.active.get() + 1);
                let source = source.clone();
                Box::new(move || {
                    drop(subscription);
                    cancel();
                    source.active.set(source.active.get() - 1);
                }) as Cleanup
            }
            Err(error) => {
                chat.fail(rt, id, error);
                drop(subscription);
                Box::new(|| {}) as Cleanup
            }
        }
    };
    Empty
}

/// Split on UTF-8 character boundaries, preserving whitespace exactly. Roughly
/// 24 characters every 65 ms gives the local demo a readable streaming pace.
pub(super) fn chunks(value: &str) -> Vec<String> {
    value
        .chars()
        .collect::<Vec<_>>()
        .chunks(24)
        .map(|c| c.iter().collect())
        .collect()
}

pub(super) fn reply(prompt: &str) -> String {
    let lower = prompt.to_lowercase();
    if lower.contains("rust")
        || lower.contains("refract")
        || lower.contains("state")
        || lower.contains("dataflow")
    {
        "In this example, state has a clear owner and views subscribe only to what they use.\n\nThe root creates one ChatModel and shares it through the environment. Each conversation owns its draft, message list, and active request. Each message has its own body attribute.\n\nWhen a reply arrives, the service appends to that message’s body. The graph invalidates its text view; the sidebar, composer, and surrounding layout do not reevaluate.\n\nA committed effect owns a typed channel and a background task. The task sends chunks; only the UI-side receiver touches graph state. Completion, Stop, deletion, or unmount closes the receiver and cancels the task. Request IDs also reject results already queued before Stop.\n\nDesktop uses a worker thread; the browser awaits asynchronously. Both deliver owned messages through Refract channels. The application logic stays in Rust.".into()
    } else if lower.contains("weekend") || lower.contains("trip") || lower.contains("away") {
        "Let’s leave a little room in the itinerary.\n\nFriday evening\nArrive without a long to-do list. Find a neighborhood café, take a short walk, and settle in.\n\nSaturday\nChoose one anchor for the day: a hike, a museum, or a market. Leave the afternoon open for something you discover along the way.\n\nSunday\nStart slowly. Have breakfast somewhere you can linger, then take the scenic route home.\n\nA useful constraint: plan one thing each day and let the rest be optional. What kind of place did you have in mind?".into()
    } else if lower.contains("design") || lower.contains("workspace") || lower.contains("focus") {
        "Start with the feeling you want to create, then make a few small decisions that support it.\n\nReduce the number of competing signals\nGive the most useful thing the most space. Use contrast for emphasis and repetition for a sense of order.\n\nMake the next step obvious\nA good workspace should help you begin without making you think about the workspace itself. Keep tools close to the task they support.\n\nBuild in a pause\nA little empty space can be useful. It lets the eye rest and makes the important details easier to find.\n\nTry changing just one thing today. Notice whether it makes starting your work feel easier.".into()
    } else {
        let topic: String = prompt.chars().take(100).collect();
        format!("Let’s break that into something useful.\n\nYou asked: “{topic}”\n\n01   Name the outcome\nDescribe what a good result would look like in one sentence. Keep it concrete enough that you could recognize it.\n\n02   Choose a small first step\nFind one action you can take with what you already know. You can refine the plan as you learn.\n\n03   Make space to reflect\nAfter trying it, note what worked and what you would change. Progress is easier to see when you leave a record.\n\nThis is a scripted local reply, not an AI-generated answer. Try asking about Rust state, design, or a weekend away for other example responses.")
    }
}

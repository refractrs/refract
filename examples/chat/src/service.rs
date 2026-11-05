//! Deterministic local replies with lifecycle-owned streaming. Replace the reply
//! source here to add a network service; views and domain commands stay unchanged.

use crate::model::{ChatModel, Conversation};
use refract::{prelude::*, Cleanup, ScopeToken};
use std::{cell::Cell, rc::Rc};

pub type Tick = Rc<dyn Fn(&mut Runtime)>;
/// The adapter supplies only scheduling, never message contents.
pub struct Schedule(pub Rc<dyn Fn(ScopeToken, Tick) -> Cleanup>);

#[view]
pub(crate) fn streams(cx: &mut ViewContext<'_>) -> impl View {
    #[environment_object]
    let model: Rc<ChatModel>;
    let chats = cx.read_state(&model.conversations);
    // Stream owners are siblings of the visible pane, so changing selection or
    // filtering the sidebar does not unmount a running conversation's effect.
    fragment(chats.into_iter().map(|chat| {
        let key = chat.id.0.to_string();
        stream(chat).key(key)
    }))
}

#[view]
fn stream(cx: &mut ViewContext<'_>, chat: Rc<Conversation>) -> impl View {
    #[environment_object]
    let schedule: Rc<Schedule>;
    let request = cx.read_state(&chat.request);
    let request_id = request.as_ref().map(|r| r.id);
    let scope = cx.scope();

    #[effect(id = "assistant-stream", deps = request_id, captures(request, chat, schedule, scope))]
    let subscription = |_| {
        let Some(request) = &request else {
            return Box::new(|| {}) as Cleanup;
        };
        let chunks = chunks(&reply(&request.prompt));
        let index = Cell::new(0);
        let id = request.id;
        let chat = chat.clone();
        (schedule.0)(
            scope.clone(),
            Rc::new(move |rt| {
                let i = index.get();
                if let Some(chunk) = chunks.get(i) {
                    index.set(i + 1);
                    chat.append(rt, id, chunk, i + 1 == chunks.len());
                }
            }),
        )
    };
    Empty
}

/// Split on UTF-8 character boundaries, preserving whitespace exactly. Roughly
/// 24 characters every 65 ms gives the local demo a readable streaming pace.
fn chunks(value: &str) -> Vec<String> {
    value
        .chars()
        .collect::<Vec<_>>()
        .chunks(24)
        .map(|c| c.iter().collect())
        .collect()
}

fn reply(prompt: &str) -> String {
    let lower = prompt.to_lowercase();
    if lower.contains("rust")
        || lower.contains("refract")
        || lower.contains("state")
        || lower.contains("dataflow")
    {
        "In this example, state has a clear owner and views subscribe only to what they use.\n\nThe root creates one ChatModel and shares it through the environment. Each conversation owns its draft, message list, and active request. Each message has its own body attribute.\n\nWhen a reply arrives, the service appends to that message’s body. The graph invalidates its text view; the sidebar, composer, and surrounding layout do not reevaluate.\n\nA committed effect owns the timer. Completion, Stop, deletion, or unmount cleans it up. Request IDs also reject late callbacks.\n\nThe browser and desktop inject scheduling. The application logic stays in Rust.".into()
    } else if lower.contains("weekend") || lower.contains("trip") || lower.contains("away") {
        "Let’s leave a little room in the itinerary.\n\nFriday evening\nArrive without a long to-do list. Find a neighborhood café, take a short walk, and settle in.\n\nSaturday\nChoose one anchor for the day: a hike, a museum, or a mrefract. Leave the afternoon open for something you discover along the way.\n\nSunday\nStart slowly. Have breakfast somewhere you can linger, then take the scenic route home.\n\nA useful constraint: plan one thing each day and let the rest be optional. What kind of place did you have in mind?".into()
    } else if lower.contains("design") || lower.contains("workspace") || lower.contains("focus") {
        "Start with the feeling you want to create, then make a few small decisions that support it.\n\nReduce the number of competing signals\nGive the most useful thing the most space. Use contrast for emphasis and repetition for a sense of order.\n\nMake the next step obvious\nA good workspace should help you begin without making you think about the workspace itself. Keep tools close to the task they support.\n\nBuild in a pause\nA little empty space can be useful. It lets the eye rest and makes the important details easier to find.\n\nTry changing just one thing today. Notice whether it makes starting your work feel easier.".into()
    } else {
        let topic: String = prompt.chars().take(100).collect();
        format!("Let’s break that into something useful.\n\nYou asked: “{topic}”\n\n01   Name the outcome\nDescribe what a good result would look like in one sentence. Keep it concrete enough that you could recognize it.\n\n02   Choose a small first step\nFind one action you can take with what you already know. You can refine the plan as you learn.\n\n03   Make space to reflect\nAfter trying it, note what worked and what you would change. Progress is easier to see when you leave a record.\n\nThis is a scripted local reply, not an AI-generated answer. Try asking about Rust state, design, or a weekend away for other example responses.")
    }
}

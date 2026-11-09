//! Background execution adapters. Neither adapter receives graph state or UI handles.
#[cfg(target_arch = "wasm32")]
mod browser;
#[cfg(not(target_arch = "wasm32"))]
mod native;

#[cfg(target_arch = "wasm32")]
pub(crate) use browser::launch;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use native::launch;

use crate::service::{chunks, reply, StreamEvent};

fn events(prompt: &str) -> impl Iterator<Item = StreamEvent> {
    let chunks = chunks(&reply(prompt));
    let total = chunks.len();
    chunks
        .into_iter()
        .enumerate()
        .map(move |(i, text)| StreamEvent::Chunk {
            text,
            complete: i + 1 == total,
        })
}

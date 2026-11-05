//! A complete, local chat example shared by the web and desktop adapters.
//! Platform code supplies a cancellable clock. State, replies, and all views live
//! in Rust; no networking or API key is needed.

#[cfg(target_arch = "wasm32")]
mod browser;
#[cfg(all(target_arch = "wasm32", feature = "browser-tests"))]
mod browser_scroll_tests;
mod model;
mod service;
#[cfg(test)]
mod tests;
mod views;

use refract::{prelude::*, Cleanup, ScopeToken};
pub use service::{Schedule, Tick};
use std::rc::Rc;
pub use views::BACKGROUND;

pub fn application(schedule: impl Fn(ScopeToken, Tick) -> Cleanup + 'static) -> impl View {
    root(Rc::new(Schedule(Rc::new(schedule))))
}

#[view]
fn root(cx: &mut ViewContext<'_>, schedule: Rc<Schedule>) -> impl View {
    // This component owns the model without reading it. Changing a message
    // therefore never invalidates the root or its environment provider.
    let model = cx.remember("chat-model", model::ChatModel::new);
    fragment([views::shell().into_any(), service::streams().into_any()])
        .environment_object(model)
        .environment_object(schedule)
}

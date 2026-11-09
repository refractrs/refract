//! A local chat demonstrating effect-owned channels and cancellable async work.
//! Native workers and browser futures send owned chunks; only receivers touch UI state.
#[cfg(target_arch = "wasm32")]
mod browser;
mod model;
mod producer;
mod service;
mod surface;
#[cfg(test)]
mod tests;
mod views;

use refract::prelude::*;
use service::ReplySource;
use std::rc::Rc;
pub use views::BACKGROUND;

pub fn application() -> impl View {
    root(Rc::new(ReplySource::new(producer::launch)))
}

#[view]
fn root(cx: &mut ViewContext<'_>, source: Rc<ReplySource>) -> impl View {
    // This owner does not read message state. Streaming only invalidates leaves.
    let surface = cx.object::<Surface>()?;
    let model = cx.remember("chat-model", move |rt| {
        model::ChatModel::hosted(rt, surface)
    });
    fragment([
        views::shell().into_any(),
        service::streams().into_any(),
        surface::synchronize().into_any(),
    ])
    .environment_object(model)
    .environment_object(source)
}

//! A local chat demonstrating effect-owned channels and cancellable async work.
//! Native workers and browser futures send owned chunks; only receivers touch UI state.
#[cfg(target_arch = "wasm32")]
mod browser;
mod model;
mod producer;
mod service;
mod surface;
mod theme;
#[cfg(test)]
mod tests;
mod views;

use refract::prelude::*;
use service::ReplySource;
use std::rc::Rc;
pub use theme::{HostInputs, InputHandles};

/// The native launcher uses the same initializer with its default light theme.
pub fn application() -> impl View {
    view_fn(|cx| {
        let app = cx.remember("application", |rt| {
            let (view, inputs) = initialize(rt);
            (view.into_any(), inputs)
        });
        Ok::<_, ViewError>(app.0.clone())
    })
}

pub const BACKGROUND: Color = Color::rgb(251, 252, 254);

/// The shared initial input value is used by prerendering and hydration.
pub fn initialize(rt: &mut Runtime) -> (impl View, InputHandles) {
    initialize_with_source(rt, Rc::new(ReplySource::new(producer::launch)))
}

fn initialize_with_source(rt: &mut Runtime, source: Rc<ReplySource>) -> (impl View, InputHandles) {
    let dark = State::new(rt, false);
    let view = root(source).environment_object(Rc::new(HostInputs {
        dark: dark.read_only(),
    }));
    (view, InputHandles { dark })
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

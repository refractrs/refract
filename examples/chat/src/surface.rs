//! Keep navigation and document metadata separate from streaming message views.
use crate::model::ChatModel;
use refract::prelude::*;

pub fn synchronize() -> impl View {
    fragment([location().into_any(), title().into_any()])
}

#[view]
fn location(cx: &mut ViewContext<'_>) -> impl View {
    let surface = cx.object::<Surface>()?;
    let model = cx.object::<ChatModel>()?;
    let url = cx.read(&surface.location);
    cx.effect("location", url.clone(), move |rt| {
        model.apply_location(rt, url.clone());
        Box::new(|| {})
    });
    Empty
}

#[view]
fn title(cx: &mut ViewContext<'_>) -> impl View {
    let surface = cx.object::<Surface>()?;
    let model = cx.object::<ChatModel>()?;
    let selected = cx.read_state(&model.selected);
    let chats = cx.read_state(&model.conversations);
    let title = chats
        .iter()
        .find(|chat| chat.id == selected)
        .map(|chat| format!("{} — Refract Chat", cx.read_state(&chat.title)))
        .unwrap_or_else(|| "Refract Chat".into());
    cx.effect("title", title.clone(), move |rt| {
        if let Err(error) = surface.set_title(rt, title.clone()) {
            model.navigation_error.set(rt, Some(error.to_string()));
        }
        Box::new(|| {})
    });
    Empty
}

#[view]
pub fn navigation_status(cx: &mut ViewContext<'_>) -> impl View {
    let model = cx.object::<ChatModel>()?;
    let error = cx.read_state(&model.navigation_error);
    let request = cx.read_state(&model.navigation_request);
    let error = request
        .and_then(|request| match cx.read(&request.status) {
            RequestStatus::Failed(error) => Some(error.to_string()),
            _ => None,
        })
        .or(error);
    match error {
        Some(error) => text(format!("Navigation unavailable: {error}"))
            .status()
            .padding(8.0)
            .into_any(),
        None => Empty.into_any(),
    }
}

use super::events;
use crate::service::StreamEvent;
use futures_util::future::{AbortHandle, Abortable};
use refract::{Cleanup, MessageSender, SendError};
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{spawn_local, JsFuture};

pub(crate) fn launch(
    prompt: String,
    sender: MessageSender<StreamEvent>,
) -> Result<Cleanup, String> {
    let window = web_sys::window().ok_or("No browser window")?;
    let (cancel, registration) = AbortHandle::new_pair();
    spawn_local(async move {
        let work = async {
            for event in events(&prompt) {
                if let Err(error) = pause(&window, 65).await {
                    send(&window, &sender, StreamEvent::Failed(error)).await;
                    return;
                }
                if !send(&window, &sender, event).await {
                    return;
                }
            }
        };
        // Aborting drops the pending delay and clears its browser timeout.
        let _ = Abortable::new(work, registration).await;
    });
    Ok(Box::new(move || cancel.abort()))
}

async fn send(
    window: &web_sys::Window,
    sender: &MessageSender<StreamEvent>,
    mut event: StreamEvent,
) -> bool {
    loop {
        match sender.try_send(event) {
            Ok(()) => return true,
            Err(SendError::Closed(_)) => return false,
            Err(SendError::Full(pending)) => event = pending,
        }
        // Yield to the host so it can drain the inbox; don't drop terminal messages.
        if let Err(error) = pause(window, 10).await {
            web_sys::console::error_1(&error.into());
            return false;
        }
    }
}

async fn pause(window: &web_sys::Window, millis: i32) -> Result<(), String> {
    let mut timer = None;
    let promise = js_sys::Promise::new(&mut |resolve, reject| match window
        .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, millis)
    {
        Ok(id) => timer = Some(id),
        Err(error) => {
            let _ = reject.call1(&JsValue::UNDEFINED, &error);
        }
    });
    Delay {
        future: JsFuture::from(promise),
        window: window.clone(),
        timer,
    }
    .await
    .map(|_| ())
    .map_err(|error| format!("Timer failed: {error:?}"))
}

/// Keeps browser timer ownership attached to the future, including cancellation.
struct Delay {
    future: JsFuture,
    window: web_sys::Window,
    timer: Option<i32>,
}
impl Future for Delay {
    type Output = Result<JsValue, JsValue>;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.future).poll(cx)
    }
}
impl Drop for Delay {
    fn drop(&mut self) {
        if let Some(timer) = self.timer {
            self.window.clear_timeout_with_handle(timer);
        }
    }
}

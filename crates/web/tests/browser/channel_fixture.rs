//! Browser conformance fixture, driven by channels.html without a JS test dependency.
#![cfg(target_arch = "wasm32")]

use refract_core::{prelude::*, ViewError};
use refract_web::{Mount, MountOptions};
use std::{cell::RefCell, rc::Rc};
use wasm_bindgen::prelude::*;

type Senders = Rc<RefCell<Vec<MessageSender<String>>>>;

#[wasm_bindgen]
pub struct ChannelFixture {
    mount: Mount,
    request: Binding<u32>,
    senders: Senders,
}

#[wasm_bindgen]
impl ChannelFixture {
    pub fn send(&self, index: usize, value: String) -> Result<(), JsValue> {
        self.senders.borrow()[index]
            .try_send(value)
            .map_err(|error| JsValue::from_str(&error.to_string()))
    }

    pub fn replace(&self) -> Result<(), JsValue> {
        let request = self.request.clone();
        self.mount
            .handle()
            .update(move |rt| request.update(rt, |value| *value += 1))
    }

    pub fn revision(&self) -> u64 {
        self.mount.revision()
    }
    pub fn last_error(&self) -> Option<String> {
        self.mount.last_error()
    }
    pub fn dispose(&self) {
        self.mount.dispose();
    }
}

#[wasm_bindgen]
pub fn start_channel_fixture(host: web_sys::Element) -> Result<ChannelFixture, JsValue> {
    let request = Rc::new(RefCell::new(None));
    let senders: Senders = Rc::default();
    let view = view_fn({
        let request = request.clone();
        let senders = senders.clone();
        move |cx| {
            let trigger = cx.state("request", || 0_u32);
            *request.borrow_mut() = Some(trigger.clone());
            let value = cx.state("result", String::new);
            let dependency = cx.read(&trigger);
            let output = cx.read(&value);
            cx.effect("load", dependency, {
                let senders = senders.clone();
                move |rt| {
                    let value = value.clone();
                    let (sender, subscription) = rt.channel(move |rt, result| {
                        value.set_if_changed(rt, result);
                    });
                    // Proves the host's message waker is installed before the
                    // initial commit, including messages produced synchronously.
                    sender.try_send("effect started".into()).unwrap();
                    senders.borrow_mut().push(sender);
                    subscription.into_cleanup()
                }
            });
            Ok::<_, ViewError>(text(output))
        }
    });
    let mount = refract_web::mount(host, view, MountOptions::default())?;
    let request = request.borrow().clone().unwrap();
    Ok(ChannelFixture {
        mount,
        request,
        senders,
    })
}

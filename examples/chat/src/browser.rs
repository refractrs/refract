//! Browser mount with host theme inputs and Refract channel wakeups.
use crate::{initialize, initialize_with_source, producer, service::ReplySource, InputHandles};
use refract_web::{InputMount, MountOptions, RootSizing};
use std::rc::Rc;
use wasm_bindgen::prelude::*;

refract_web::export_prerender_with_inputs!(
    initialize,
    refract_web::PrerenderOptions {
        styles: ".refract-root { width: 100%; height: 100%; }".into(),
        ..Default::default()
    }
);

#[wasm_bindgen]
pub struct ChatApp {
    mount: InputMount<InputHandles>,
    source: Rc<ReplySource>,
}
#[wasm_bindgen]
impl ChatApp {
    pub fn set_dark(&self, dark: bool) -> Result<(), JsValue> {
        self.mount.update_inputs(move |inputs, rt| {
            inputs.dark.set_if_changed(rt, dark);
        })
    }

    pub fn hydrated(&self) -> bool {
        self.mount.mount().hydrated()
    }
    pub fn dispose(&self) {
        self.mount.mount().dispose();
    }
    pub fn revision(&self) -> u64 {
        self.mount.mount().revision()
    }
    pub fn dom_calls(&self) -> u64 {
        self.mount.mount().dom_calls()
    }
    pub fn active_streams(&self) -> u32 {
        self.source.active_streams()
    }
    pub fn last_error(&self) -> Option<String> {
        self.mount.mount().last_error()
    }
}

#[wasm_bindgen]
pub fn start(host: web_sys::Element) -> Result<ChatApp, JsValue> {
    let source = Rc::new(ReplySource::new(producer::launch));
    let mount = refract_web::hydrate_with_inputs(
        host,
        MountOptions {
            sizing: RootSizing::Fill,
            manage_document: true,
            ..Default::default()
        },
        |rt| initialize_with_source(rt, source.clone()),
    )?;
    Ok(ChatApp { mount, source })
}

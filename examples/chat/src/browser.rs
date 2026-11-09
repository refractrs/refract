//! Browser mount; channel wakeups are provided by Refract, without update handles.
use crate::{producer, root, service::ReplySource};
use refract_web::{Mount, MountOptions, RootSizing};
use std::rc::Rc;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct ChatApp {
    mount: Mount,
    source: Rc<ReplySource>,
}
#[wasm_bindgen]
impl ChatApp {
    pub fn dispose(&self) {
        self.mount.dispose();
    }
    pub fn revision(&self) -> u64 {
        self.mount.revision()
    }
    pub fn dom_calls(&self) -> u64 {
        self.mount.dom_calls()
    }
    pub fn active_streams(&self) -> u32 {
        self.source.active_streams()
    }
    pub fn last_error(&self) -> Option<String> {
        self.mount.last_error()
    }
}

#[wasm_bindgen]
pub fn start(host: web_sys::Element) -> Result<ChatApp, JsValue> {
    let source = Rc::new(ReplySource::new(producer::launch));
    let mount = refract_web::mount(
        host,
        root(source.clone()),
        MountOptions {
            sizing: RootSizing::Fill,
            manage_document: true,
            ..Default::default()
        },
    )?;
    Ok(ChatApp { mount, source })
}

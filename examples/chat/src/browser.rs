//! Browser adapter wiring. The timer invokes the same Rust tick as desktop.
use crate::application;
use refract::Cleanup;
use refract_web::{Mount, MountOptions, RootSizing, UpdateHandle};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
use wasm_bindgen::{prelude::*, JsCast};

#[wasm_bindgen]
pub struct ChatApp {
    mount: Mount,
    timers: Rc<Cell<u32>>,
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
        self.timers.get()
    }
}

#[wasm_bindgen]
pub fn start(host: web_sys::Element) -> Result<ChatApp, JsValue> {
    let window =
        web_sys::window().ok_or_else(|| JsValue::from_str("chat requires a browser window"))?;
    let connection: Rc<RefCell<Option<UpdateHandle>>> = Rc::default();
    let timers = Rc::new(Cell::new(0_u32));
    let schedule = {
        let connection = connection.clone();
        let timers = timers.clone();
        move |scope: refract::ScopeToken, tick: crate::Tick| -> Cleanup {
            let connection = connection.clone();
            let callback = Closure::<dyn FnMut()>::new(move || {
                if !scope.is_alive() {
                    return;
                }
                if let Some(handle) = connection.borrow().as_ref() {
                    let tick = tick.clone();
                    let scope = scope.clone();
                    if let Err(error) = handle.update(move |rt| {
                        if scope.is_alive() {
                            tick(rt);
                        }
                    }) {
                        web_sys::console::error_1(&error);
                    }
                }
            });
            match window.set_interval_with_callback_and_timeout_and_arguments_0(
                callback.as_ref().unchecked_ref(),
                65,
            ) {
                Ok(id) => {
                    timers.set(timers.get() + 1);
                    let timers = timers.clone();
                    let window = window.clone();
                    Box::new(move || {
                        window.clear_interval_with_handle(id);
                        timers.set(timers.get() - 1);
                        drop(callback);
                    })
                }
                Err(error) => {
                    web_sys::console::error_1(&error);
                    Box::new(|| {})
                }
            }
        }
    };
    let mount = refract_web::mount(
        host,
        application(schedule),
        MountOptions {
            sizing: RootSizing::Fill,
            ..Default::default()
        },
    )?;
    *connection.borrow_mut() = Some(mount.handle());
    Ok(ChatApp { mount, timers })
}

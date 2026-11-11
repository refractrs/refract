//! Focus recovery in the real DOM, including notifications during host patches.
#![cfg(target_arch = "wasm32")]
use refract_core::prelude::*;
use refract_web::{Mount, MountOptions};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct FocusFixture(Mount);

#[wasm_bindgen]
impl FocusFixture {
    pub fn last_error(&self) -> Option<String> {
        self.0.last_error()
    }
}

#[wasm_bindgen]
pub fn start_focus_fixture(host: web_sys::Element) -> Result<FocusFixture, JsValue> {
    let app = view_fn(|cx| {
        let flags = cx.state("flags", || 0_u8);
        let log = cx.state("log", String::new);
        let bits = cx.read(&flags);
        let mut content = v_stack();
        for name in ["a", "b", "c"] {
            if bits & 8 != 0 || (name == "b" && bits & 1 != 0) {
                continue;
            }
            let log = log.clone();
            content = content.child(
                text(name)
                    .label(name)
                    .focusable(!(name == "a" && bits & 4 != 0))
                    .disabled(name == "c" && bits & 2 != 0)
                    .height(40.0)
                    .on_focus_change(move |cx, focused| {
                        log.update(cx.runtime(), |log| {
                            log.push_str(&format!("{name}:{focused};"))
                        });
                    })
                    .key(name),
            );
        }
        let mut root = v_stack().child(scroll_view(content).height(70.0).width(200.0));
        for (name, bits) in [
            ("remove-b", 1),
            ("disable-c", 3),
            ("skip-a", 7),
            ("clear", 8),
            ("reset", 0),
        ] {
            let flags = flags.clone();
            root = root.child(
                text(name)
                    .label(name)
                    .on_press(move |cx| flags.set(cx.runtime(), bits))
                    .focusable(false),
            );
        }
        Ok(root.child(text(cx.read(&log)).label("log")))
    });
    refract_web::mount(host, app, MountOptions::default()).map(FocusFixture)
}

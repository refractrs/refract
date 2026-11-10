//! Live filter updates, removal, and validation through the retained DOM bridge.
#![cfg(target_arch = "wasm32")]
use refract_core::*;
use refract_web::{Mount, MountOptions};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct EffectsFixture(Mount);

#[wasm_bindgen]
impl EffectsFixture {
    pub fn last_error(&self) -> Option<String> {
        self.0.last_error()
    }
    pub fn dispose(&self) {
        self.0.dispose();
    }
}

#[wasm_bindgen]
pub fn start_effects_fixture(host: web_sys::Element) -> Result<EffectsFixture, JsValue> {
    refract_web::mount(
        host,
        view_fn(|cx| {
            let step = cx.state("step", || 0);
            let (blur, backdrop) = match cx.read(&step) {
                0 => (Some(3.0), Some(12.0)),
                1 => (Some(6.0), Some(4.0)),
                2 => (Some(0.0), Some(0.0)),
                3 => (None, None),
                _ => (Some(-1.0), Some(f32::NAN)),
            };
            Ok(v_stack()
                .gap(24.0)
                .child(
                    text("Subtree blur")
                        .font_size(36.0)
                        .label("blurred")
                        .modify(false, move |e| e.style.blur = blur),
                )
                .child(
                    z_stack()
                        .child(
                            h_stack()
                                .child(view().width(160.0).height(120.0).background(Color::BLUE))
                                .child(view().width(160.0).height(120.0).background(Color::WHITE)),
                        )
                        .child(
                            text("Sharp foreground")
                                .font_size(24.0)
                                .padding(24.0)
                                .background(Color(255, 255, 255, 45))
                                .radius(18.0)
                                .label("glass")
                                .modify(false, move |e| e.style.backdrop_blur = backdrop),
                        ),
                )
                .child(
                    text("Next")
                        .label("next")
                        .on_press(move |cx| step.update(cx.runtime(), |n| *n += 1)),
                ))
        }),
        MountOptions::default(),
    )
    .map(EffectsFixture)
}

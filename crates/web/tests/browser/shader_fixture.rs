//! Shader lifecycle and pixel contract through the retained browser adapter.
#![cfg(target_arch = "wasm32")]
use refract_core::*;
use refract_web::{Mount, MountOptions};
use wasm_bindgen::prelude::*;

fn application() -> impl View {
    let program = Shader::wgsl("struct Uniforms { red: f32 }\nfn shade(frame: ShaderFrame) -> vec4<f32> { return vec4<f32>(uniforms.red, frame.uv.x, sample_image(frame.uv).b, 0.5); }").unwrap()
        .image(ShaderImage::rgba(1, 1, vec![0, 0, 51, 255]).unwrap())
        .fallback(Color::rgb(11, 22, 33));
    view_fn(move |cx| {
        let amount = cx.state("amount", || 0.8_f32);
        let visible = cx.state("visible", || true);
        let wide = cx.state("wide", || false);
        let read = amount.clone();
        let program = program.clone();
        let drawing = if cx.read(&visible) {
            shader_with(move |inputs| program.clone().uniform("red", inputs.read(&read)))
                .width(if cx.read(&wide) { 128.0 } else { 64.0 })
                .height(64.0)
                .label("shader")
                .transition(Transition::opacity())
                .into_any()
        } else {
            Empty.into_any()
        };
        Ok(v_stack()
            .gap(12.0)
            .child(drawing)
            .child(text("Update").on_press(move |cx| amount.set(cx.runtime(), 0.4)))
            .child(text("Resize").on_press(move |cx| wide.update(cx.runtime(), |v| *v = !*v)))
            .child(text("Toggle").on_press(move |cx| {
                with_animation(
                    cx.runtime(),
                    Animation::linear(std::time::Duration::from_millis(400)),
                    |rt| {
                        visible.update(rt, |v| *v = !*v);
                    },
                );
            })))
    })
}
#[wasm_bindgen]
pub struct ShaderFixture(Mount);
#[wasm_bindgen]
impl ShaderFixture {
    pub fn hydrated(&self) -> bool {
        self.0.hydrated()
    }
    pub fn dispose(&self) {
        self.0.dispose();
    }
    pub fn last_error(&self) -> Option<String> {
        self.0.last_error()
    }
}
#[wasm_bindgen]
pub fn shader_snapshot() -> Result<String, JsValue> {
    refract_web::prerender(application(), Default::default())
        .map_err(|e| JsValue::from_str(&e.to_string()))
}
#[wasm_bindgen]
pub fn start_shader_fixture(
    host: web_sys::Element,
    hydrate: bool,
) -> Result<ShaderFixture, JsValue> {
    if hydrate {
        refract_web::hydrate(host, application(), MountOptions::default()).map(ShaderFixture)
    } else {
        refract_web::mount(host, application(), MountOptions::default()).map(ShaderFixture)
    }
}

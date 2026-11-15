//! Browser-owned GPU surfaces. Drop releases resources even during initialization.
use refract_core::{DrawingCommand, Shader, Size};
use wasm_bindgen::prelude::*;

#[wasm_bindgen(module = "/src/shader.js")]
extern "C" {
    pub(crate) fn copy_shader_canvases(original: &web_sys::Element, clone: &web_sys::Element);
    type ShaderSurface;
    #[wasm_bindgen(constructor)]
    fn new(host: &web_sys::Element) -> ShaderSurface;
    #[wasm_bindgen(method)]
    fn set_program(this: &ShaderSurface, source: &str);
    #[wasm_bindgen(method)]
    fn set_image(this: &ShaderSurface, pixels: &[u8], width: u32, height: u32);
    #[wasm_bindgen(method)]
    fn update(
        this: &ShaderSurface,
        uniforms: &[u8],
        frame: &[u8],
        width: u32,
        height: u32,
        left: f32,
        top: f32,
        fallback: &str,
    );
    #[wasm_bindgen(method)]
    fn active(this: &ShaderSurface) -> bool;
    #[wasm_bindgen(method)]
    fn dispose(this: &ShaderSurface);
}

pub(crate) struct ShaderDom {
    surface: ShaderSurface,
    previous: Option<Shader>,
}
impl ShaderDom {
    pub fn new(host: &web_sys::Element) -> Self {
        Self {
            surface: ShaderSurface::new(host),
            previous: None,
        }
    }
    pub fn active(&self) -> bool {
        self.surface.active()
    }
    pub fn draw(
        &mut self,
        commands: &[DrawingCommand],
        size: Size,
        left: f32,
        top: f32,
    ) -> Result<(), JsValue> {
        let (shader, elapsed) =
            frame(commands).ok_or_else(|| JsValue::from_str("invalid shader drawing"))?;
        if self
            .previous
            .as_ref()
            .is_none_or(|old| old.source() != shader.source())
        {
            self.surface.set_program(shader.source());
        }
        if self
            .previous
            .as_ref()
            .is_none_or(|old| old.image_input() != shader.image_input())
        {
            if let Some(image) = shader.image_input() {
                self.surface
                    .set_image(image.pixels(), image.width(), image.height());
            } else {
                self.surface.set_image(&[], 1, 1);
            }
        }
        let scale = web_sys::window().unwrap().device_pixel_ratio();
        let width = (size.width * scale).ceil().clamp(1.0, 8192.0) as u32;
        let height = (size.height * scale).ceil().clamp(1.0, 8192.0) as u32;
        let color = shader.fallback_color();
        self.surface.update(
            &shader
                .uniform_bytes()
                .map_err(|e| JsValue::from_str(&e.to_string()))?,
            &Shader::frame_bytes(size, width, height, elapsed),
            width,
            height,
            left,
            top,
            &format!(
                "rgba({},{},{},{})",
                color.0,
                color.1,
                color.2,
                f64::from(color.3) / 255.0
            ),
        );
        self.previous = Some(shader.clone());
        Ok(())
    }
}
impl Drop for ShaderDom {
    fn drop(&mut self) {
        self.surface.dispose();
    }
}
fn frame(commands: &[DrawingCommand]) -> Option<(&Shader, std::time::Duration)> {
    match commands {
        [DrawingCommand::Group { commands, .. }] => frame(commands),
        [DrawingCommand::Shader {
            shader, elapsed, ..
        }] => Some((shader, *elapsed)),
        _ => None,
    }
}

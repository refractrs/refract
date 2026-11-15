#[cfg(not(target_arch = "wasm32"))]
fn main() -> refract_native::Result<()> {
    refract_native::run(
        refract_native::NativeOptions {
            title: "Shaders — Refract".into(),
            initial_url: None,
            width: 860.0,
            height: 740.0,
            background: refract_shader::BACKGROUND,
            diagnostics: false,
        },
        |_| refract_shader::application(),
    )
}
#[cfg(target_arch = "wasm32")]
fn main() {}

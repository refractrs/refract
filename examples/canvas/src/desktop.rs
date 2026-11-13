#[cfg(not(target_arch = "wasm32"))]
fn main() -> refract_native::Result<()> {
    refract_native::run(
        refract_native::NativeOptions {
            title: "Canvas — Refract".into(),
            initial_url: None,
            width: 860.0,
            height: 760.0,
            background: refract_canvas::BACKGROUND,
            diagnostics: false,
        },
        |_| refract_canvas::application(),
    )
}

#[cfg(target_arch = "wasm32")]
fn main() {}

//! Native launcher; the UI and state are shared with the browser example.
#[cfg(not(target_arch = "wasm32"))]
fn main() -> refract_native::Result<()> {
    refract_native::run(
        refract_native::NativeOptions {
            title: "Counter — Refract Native".into(),
            diagnostics: std::env::var_os("REFRACT_DIAGNOSTICS").is_some(),
            ..Default::default()
        },
        |_| refract_counter::application(),
    )
}

#[cfg(target_arch = "wasm32")]
fn main() {}

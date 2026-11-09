//! The shared application selects a native background producer.
#[cfg(not(target_arch = "wasm32"))]
fn main() -> refract_native::Result<()> {
    refract_native::run(
        refract_native::NativeOptions {
            title: "Chat".into(),
            width: 1180.0,
            height: 880.0,
            background: refract_chat::BACKGROUND,
            diagnostics: std::env::var_os("REFRACT_DIAGNOSTICS").is_some(),
        },
        |_| refract_chat::application(),
    )
}
#[cfg(target_arch = "wasm32")]
fn main() {}

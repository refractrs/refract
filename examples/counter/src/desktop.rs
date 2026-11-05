//! Native launcher; the UI and state are shared with the browser example.
#[cfg(not(target_arch = "wasm32"))]
fn main() -> refract_desktop::Result<()> {
    refract_desktop::run(
        refract_desktop::DesktopOptions {
            title: "Counter — Refract Desktop".into(),
            diagnostics: std::env::var_os("REFRACT_DIAGNOSTICS").is_some(),
            ..Default::default()
        },
        |_| refract_counter::application(),
    )
}

#[cfg(target_arch = "wasm32")]
fn main() {}

//! Native launcher. The same gallery and embedded photos are used by the WASM build.
#[cfg(not(target_arch = "wasm32"))]
fn main() -> refract_desktop::Result<()> {
    refract_desktop::run(
        refract_desktop::DesktopOptions {
            title: "Gallery — Refract Desktop".into(),
            width: 1120.0,
            // Leave room for the full collection at native font metrics.
            height: 860.0,
            background: refract_gallery::BACKGROUND,
            diagnostics: std::env::var_os("REFRACT_DIAGNOSTICS").is_some(),
        },
        |_| refract_gallery::application(),
    )
}

#[cfg(target_arch = "wasm32")]
fn main() {}

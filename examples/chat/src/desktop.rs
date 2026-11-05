//! Only the scheduler and window configuration are platform-specific.
#[cfg(not(target_arch = "wasm32"))]
fn main() -> refract_desktop::Result<()> {
    refract_desktop::run(
        refract_desktop::DesktopOptions {
            title: "Chat — Refract Desktop".into(),
            width: 1180.0,
            height: 880.0,
            background: refract_chat::BACKGROUND,
            diagnostics: std::env::var_os("REFRACT_DIAGNOSTICS").is_some(),
        },
        |desktop| {
            refract_chat::application(move |scope, tick| {
                let timer = desktop
                    .interval(std::time::Duration::from_millis(65), scope, move |rt| {
                        tick(rt)
                    })
                    .expect("valid timer during committed chat setup");
                Box::new(move || drop(timer))
            })
        },
    )
}
#[cfg(target_arch = "wasm32")]
fn main() {}

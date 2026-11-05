//! Native clock source. The root effect, context, and displays are shared with WASM.
#[cfg(not(target_arch = "wasm32"))]
fn main() -> refract_desktop::Result<()> {
    use std::time::Duration;

    fn now() -> refract_clock::Reading {
        let now = chrono::Local::now();
        refract_clock::Reading {
            time: now.format("%H:%M:%S").to_string(),
            date: now.format("%A, %B %-d, %Y").to_string(),
        }
    }

    refract_desktop::run(
        refract_desktop::DesktopOptions {
            title: "Clock — Refract Desktop".into(),
            height: 600.0,
            diagnostics: std::env::var_os("REFRACT_DIAGNOSTICS").is_some(),
            ..Default::default()
        },
        |desktop| {
            refract_clock::application(now(), move |reading, scope| {
                let timer = desktop
                    .interval(Duration::from_secs(1), scope, move |rt| {
                        reading.set_if_changed(rt, now());
                    })
                    .expect("valid timer during committed clock setup");
                Box::new(move || drop(timer))
            })
        },
    )
}

#[cfg(target_arch = "wasm32")]
fn main() {}

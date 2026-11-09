//! Native clock source. The root effect, context, and displays are shared with WASM.
#[cfg(not(target_arch = "wasm32"))]
fn main() -> refract_native::Result<()> {
    use std::time::Duration;

    fn now() -> refract_clock::Reading {
        let now = chrono::Local::now();
        refract_clock::Reading {
            time: now.format("%H:%M:%S").to_string(),
            date: now.format("%A, %B %-d, %Y").to_string(),
        }
    }

    refract_native::run(
        refract_native::NativeOptions {
            title: "Clock — Refract Native".into(),
            height: 600.0,
            diagnostics: std::env::var_os("REFRACT_DIAGNOSTICS").is_some(),
            ..Default::default()
        },
        |native| {
            refract_clock::application(now(), move |reading, scope| {
                let timer = native
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

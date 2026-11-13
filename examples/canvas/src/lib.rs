//! One canvas example shared by the browser and native hosts.

use refract::prelude::*;
use std::time::Duration;

mod prism;

pub const BACKGROUND: Color = Color::rgb(245, 248, 253);
const INK: Color = Color::rgb(37, 49, 73);
const MUTED: Color = Color::rgb(111, 128, 153);

fn control(
    label: &str,
    active: bool,
    action: impl Fn(&mut ActionContext<'_>) + 'static,
) -> impl View {
    text(label)
        .font_size(13.0)
        .padding(12.0)
        .radius(10.0)
        .background(if active {
            Color::rgb(225, 234, 250)
        } else {
            Color::WHITE
        })
        .foreground(INK)
        .on_press(action)
        .label(label)
}

#[view]
pub fn application(cx: &mut ViewContext<'_>) -> impl View {
    let tilt = cx.state("tilt", || Point::ZERO);
    let pulses = cx.state("pulses", || true);
    let glow = cx.state("glow", || 1.0f64);
    let running = cx.read(&pulses);
    let bright = cx.read(&glow) > 0.5;
    let read_tilt = tilt.clone();
    let read_glow = glow.clone();
    let pointer_tilt = tilt.clone();
    let toggle_pulses = pulses.clone();
    let toggle_glow = glow.clone();

    // Read tilt here so pointer updates invalidate the canvas preparation alone.
    let illustration = canvas_with(
        move |inputs| (inputs.read(&read_tilt), inputs.read(&read_glow)),
        |g, frame, (tilt, glow)| prism::draw(g, frame, *tilt, *glow),
    )
    .animatable_inputs()
    .schedule(if running {
        CanvasSchedule::animation()
    } else {
        CanvasSchedule::Paused
    })
    .view_box(Rect::new(0.0, 0.0, 540.0, 330.0))
    .width(720.0)
    .height(440.0)
    .label("Interactive glass prism with refracted light")
    .on_pointer(move |cx, event| {
        if event.kind != PointerKind::Mouse {
            return;
        }
        let target = match event.phase {
            PointerPhase::Move => {
                let (Some(p), Some(b)) = (cx.local_position(event.position), cx.local_bounds())
                else {
                    return;
                };
                if b.width <= 0.0 || b.height <= 0.0 {
                    return;
                }
                Point::new(
                    (p.x / b.width * 2.0 - 1.0).clamp(-1.0, 1.0),
                    (p.y / b.height * 2.0 - 1.0).clamp(-1.0, 1.0),
                )
            }
            PointerPhase::Leave | PointerPhase::Cancel => Point::ZERO,
            _ => return,
        };
        with_animation(
            cx.runtime(),
            Animation::spring_with(Duration::from_millis(350), 0.9),
            |rt| pointer_tilt.set(rt, target),
        );
    });

    v_stack()
        .gap(18.0)
        .child(
            text("CANVAS / REFRACT")
                .font_size(11.0)
                .letter_spacing(2.0)
                .foreground(MUTED),
        )
        .child(
            text("Drawing with light.")
                .font_size(34.0)
                .font_weight(600)
                .foreground(INK),
        )
        .child(
            text("Move your pointer over the prism. Geometry follows with a spring.")
                .font_size(14.0)
                .foreground(MUTED),
        )
        .child(illustration)
        .child(
            h_stack()
                .gap(10.0)
                .child(control(
                    if running {
                        "Pause pulses"
                    } else {
                        "Play pulses"
                    },
                    running,
                    move |cx| toggle_pulses.update(cx.runtime(), |v| *v = !*v),
                ))
                .child(control(
                    if bright { "Glow on" } else { "Glow off" },
                    bright,
                    move |cx| {
                        with_animation(
                            cx.runtime(),
                            Animation::ease_in_out(Duration::from_millis(250)),
                            |rt| toggle_glow.update(rt, |v| *v = if *v > 0.5 { 0.0 } else { 1.0 }),
                        );
                    },
                ))
                .child(control("Reset tilt", false, move |cx| {
                    with_animation(cx.runtime(), Animation::spring(), |rt| {
                        tilt.set(rt, Point::ZERO)
                    })
                })),
        )
        .child(
            text("Paths  ·  Gradients  ·  Alpha masks  ·  Blur  ·  Animated inputs")
                .font_size(12.0)
                .foreground(MUTED),
        )
        .padding(30.0)
        .width(800.0)
}

#[cfg(target_arch = "wasm32")]
mod browser {
    use super::*;
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen]
    pub struct CanvasApp(refract_web::Mount);

    #[wasm_bindgen]
    impl CanvasApp {
        pub fn dispose(&self) {
            self.0.dispose();
        }

        pub fn revision(&self) -> u64 {
            self.0.revision()
        }

        pub fn dom_calls(&self) -> u64 {
            self.0.dom_calls()
        }

        pub fn last_error(&self) -> Option<String> {
            self.0.last_error()
        }
    }

    #[wasm_bindgen]
    pub fn start(host: web_sys::Element) -> Result<CanvasApp, JsValue> {
        Ok(CanvasApp(refract_web::mount(
            host,
            application(),
            Default::default(),
        )?))
    }
}

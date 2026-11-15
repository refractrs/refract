//! The same application-owned WGSL program runs on desktop and in the browser.
use refract::prelude::*;
use std::{sync::LazyLock, time::Duration};

pub const BACKGROUND: Color = Color::rgb(11, 14, 24);
static PLASMA: LazyLock<Shader> = LazyLock::new(|| {
    let mut pixels = Vec::new();
    for y in 0..128 {
        for x in 0..128 {
            let color = if (x / 16 + y / 16) % 2 == 0 {
                [240, 180, 125, 255]
            } else {
                [43, 70, 115, 255]
            };
            pixels.extend_from_slice(&color);
        }
    }
    Shader::wgsl(include_str!("plasma.wgsl"))
        .expect("valid example shader")
        .image(ShaderImage::rgba(128, 128, pixels).unwrap())
        .fallback(Color::rgb(25, 35, 65))
});

fn control(label: &str, action: impl Fn(&mut ActionContext<'_>) + 'static) -> impl View {
    text(label)
        .font_size(13.0)
        .padding(12.0)
        .radius(9.0)
        .background(Color::rgb(27, 33, 49))
        .foreground(Color::rgb(215, 225, 245))
        .on_press(action)
        .label(label)
}

#[view]
pub fn application(cx: &mut ViewContext<'_>) -> impl View {
    let pointer = cx.state("pointer", || [0.5_f32, 0.5]);
    let intensity = cx.state("intensity", || 1.0_f32);
    let image_mix = cx.state("image", || 0.0_f32);
    let running = cx.state("running", || true);
    let active = cx.read(&running);
    let (p, i, m) = (pointer.clone(), intensity.clone(), image_mix.clone());
    let drawing = shader_with(move |inputs| {
        PLASMA
            .clone()
            .uniform("pointer", inputs.read(&p))
            .uniform("intensity", inputs.read(&i))
            .uniform("image_mix", inputs.read(&m))
    })
    .animatable_inputs()
    .schedule(if active {
        ShaderSchedule::Animation
    } else {
        ShaderSchedule::Paused
    })
    .fill_width()
    .height(420.0)
    .radius(20.0)
    .clipped()
    .label("Animated shader with interactive light and image distortion")
    .on_pointer(move |cx, event| {
        if event.phase != PointerPhase::Move {
            return;
        }
        if let (Some(p), Some(b)) = (cx.local_position(event.position), cx.local_bounds()) {
            if b.width > 0.0 && b.height > 0.0 {
                pointer.set(
                    cx.runtime(),
                    [(p.x / b.width) as f32, (p.y / b.height) as f32],
                );
            }
        }
    });
    v_stack()
        .gap(18.0)
        .child(
            text("SHADERS / REFRACT")
                .font_size(11.0)
                .letter_spacing(2.0)
                .foreground(Color::rgb(130, 153, 185)),
        )
        .child(
            text("Light, written in WGSL.")
                .font_size(34.0)
                .font_weight(600)
                .foreground(Color::WHITE),
        )
        .child(
            text("Move your pointer through the field. One shader, two platforms.")
                .font_size(14.0)
                .foreground(Color::rgb(150, 163, 185)),
        )
        .child(drawing)
        .child(
            h_stack()
                .gap(10.0)
                .child(control(if active { "Pause" } else { "Play" }, move |cx| {
                    running.update(cx.runtime(), |v| *v = !*v)
                }))
                .child(control("Change intensity", move |cx| {
                    with_animation(
                        cx.runtime(),
                        Animation::ease_in_out(Duration::from_millis(500)),
                        |rt| {
                            intensity.update(rt, |v| *v = if *v > 0.7 { 0.45 } else { 1.0 });
                        },
                    );
                }))
                .child(control("Mix image", move |cx| {
                    with_animation(
                        cx.runtime(),
                        Animation::ease_in_out(Duration::from_millis(700)),
                        |rt| {
                            image_mix.update(rt, |v| *v = 1.0 - *v);
                        },
                    );
                })),
        )
        .child(
            text("Custom source · Reactive uniforms · Image sampling · GPU rendering")
                .font_size(12.0)
                .foreground(Color::rgb(115, 134, 162)),
        )
        .padding(28.0)
        .width(800.0)
        .max_width(800.0)
}

#[cfg(target_arch = "wasm32")]
mod browser {
    use super::*;
    use wasm_bindgen::prelude::*;
    #[wasm_bindgen]
    pub struct ShaderApp(refract_web::Mount);
    #[wasm_bindgen]
    impl ShaderApp {
        pub fn dispose(&self) {
            self.0.dispose();
        }
        pub fn revision(&self) -> u64 {
            self.0.revision()
        }
        pub fn last_error(&self) -> Option<String> {
            self.0.last_error()
        }
    }
    #[wasm_bindgen]
    pub fn start(host: web_sys::Element) -> Result<ShaderApp, JsValue> {
        Ok(ShaderApp(refract_web::mount(
            host,
            application(),
            Default::default(),
        )?))
    }
}

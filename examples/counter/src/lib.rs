//! The entire counter UI is authored in Rust using Refract's public API.
use refract::prelude::*;

const INK: Color = Color::rgb(38, 46, 35);
const MUTED: Color = Color::rgb(111, 116, 104);
const GREEN: Color = Color::rgb(53, 76, 44);
const LINE: Color = Color::rgb(217, 220, 207);
const PAPER: Color = Color::rgb(250, 250, 244);
const SERIF: &str = "Georgia, 'Times New Roman', serif";
const MONO: &str = "'SFMono-Regular', Consolas, monospace";

#[view]
fn number(cx: &mut ViewContext<'_>, count: Binding<i32>) -> impl View {
    let value = cx.read(&count);
    text(value.to_string())
        .status()
        .font_family(SERIF)
        .font_size(112.0)
        .line_height(136.0)
        .text_align(Alignment::Center)
        .foreground(INK)
}

#[view]
fn counter(cx: &mut ViewContext<'_>) -> impl View {
    #[state]
    let count: i32 = 0;
    #[action(count)]
    let increment = |cx| count.update(cx.runtime(), |n| *n = n.saturating_add(1));
    #[action(count)]
    let decrement = |cx| count.update(cx.runtime(), |n| *n = n.saturating_sub(1));
    #[action(count)]
    let reset = |cx| count.set_if_changed(cx.runtime(), 0);

    v_stack()
        .gap(28.0)
        .child(
            v_stack()
                .gap(6.0)
                .child(
                    text("CURRENT COUNT")
                        .font_family(MONO)
                        .font_size(11.0)
                        .foreground(MUTED)
                        .text_align(Alignment::Center),
                )
                .child(number(count))
                .child(
                    text("Every click is a little progress.")
                        .font_size(13.0)
                        .foreground(MUTED)
                        .text_align(Alignment::Center),
                )
                .padding(Insets::symmetric(24.0, 36.0))
                .background(PAPER)
                .border(Border::all(1.0, LINE))
                .radius(18.0),
        )
        .child(
            h_stack()
                .gap(12.0)
                .child(
                    h_stack()
                        .alignment(Alignment::Center)
                        .justify(Alignment::Center)
                        .child(text("−").font_size(25.0))
                        .on_press(decrement)
                        .label("Decrease count")
                        .width(58.0)
                        .height(54.0)
                        .background(PAPER)
                        .border(Border::all(1.0, LINE))
                        .radius(10.0),
                )
                .child(
                    h_stack()
                        .alignment(Alignment::Center)
                        .justify(Alignment::Center)
                        .child(text("+   Increment").font_size(15.0).font_weight(600))
                        .on_press(increment)
                        .label("Increment")
                        .height(54.0)
                        .grow(1.0)
                        .background(GREEN)
                        .foreground(Color::WHITE)
                        .radius(10.0),
                ),
        )
        .child(
            h_stack()
                .alignment(Alignment::Center)
                .justify(Alignment::Center)
                .child(text("Reset to zero").font_size(13.0).foreground(MUTED))
                .on_press(reset)
                .label("Reset to zero")
                .height(32.0),
        )
}

pub fn application() -> impl View {
    v_stack().gap(42.0)
        .child(h_stack().alignment(Alignment::Center).justify(Alignment::SpaceBetween)
            .child(h_stack().gap(10.0).alignment(Alignment::Center)
                .child(text("r").font_family(SERIF).font_size(30.0).foreground(GREEN))
                .child(text("refract").font_size(18.0).font_weight(600).foreground(INK)))
            .child(text("EXAMPLE  /  001").font_family(MONO).font_size(10.0).foreground(MUTED)))
        .child(v_stack().gap(14.0)
            .child(text("Make it count.").heading(1).font_family(SERIF).font_size(48.0).line_height(56.0).foreground(INK))
            .child(text(if cfg!(target_arch = "wasm32") { "A tiny application. A reactive foundation.\nBuilt with Refract, running in your browser." } else { "A tiny application. A reactive foundation.\nBuilt with Refract, running on your desktop." }).font_size(15.0).line_height(25.0).foreground(MUTED)))
        .child(counter())
        .child(h_stack().justify(Alignment::SpaceBetween).alignment(Alignment::Center)
            .child(text(if cfg!(target_arch = "wasm32") { "●  Rust + WebAssembly" } else { "●  Rust + Native GPU" }).font_family(MONO).font_size(10.0).foreground(GREEN))
            .child(text("One state. No page reloads.").font_size(11.0).foreground(MUTED)))
        .max_width(480.0).fill_width().padding(28.0)
}

#[cfg(target_arch = "wasm32")]
mod browser {
    use super::*;
    use wasm_bindgen::prelude::*;
    #[wasm_bindgen]
    pub struct CounterApp {
        mount: refract_web::Mount,
    }
    #[wasm_bindgen]
    impl CounterApp {
        pub fn dispose(&self) {
            self.mount.dispose()
        }
        pub fn revision(&self) -> u64 {
            self.mount.revision()
        }
        pub fn dom_calls(&self) -> u64 {
            self.mount.dom_calls()
        }
        pub fn last_error(&self) -> Option<String> {
            self.mount.last_error()
        }
    }
    #[cfg(feature = "browser-tests")]
    #[wasm_bindgen]
    pub fn start_border_fixture(host: web_sys::Element) -> Result<CounterApp, JsValue> {
        let content = view_fn(|cx| {
            let stage = cx.state("stage", || 0);
            let current = cx.read(&stage);
            let border = match current {
                0 => Some(Border::all(1.0, Color::BLACK).top(2.0, Color::BLUE).left(8.0, Color::BLACK)),
                1 => Some(Border::all(3.0, Color::BLUE)),
                2 => None,
                _ => Some(Border::default().bottom(5.0, Color::BLACK)),
            };
            let radius = if current == 0 { CornerRadii::all(12.0).top_left(0.0).bottom_right(24.0) }
                else { CornerRadii::all(6.0) };
            Ok(view().width(120.0).height(80.0).radius(radius)
                .background(Color::WHITE).label("border fixture")
                .on_press(move |cx| stage.update(cx.runtime(), |n| *n += 1))
                .modify(false, move |element| element.style.border = border))
        });
        Ok(CounterApp { mount: refract_web::mount(host, content, refract_web::MountOptions::default())? })
    }

    #[cfg(feature = "browser-tests")]
    #[wasm_bindgen]
    pub fn start_interaction_fixture(host: web_sys::Element) -> Result<CounterApp, JsValue> {
        let content = view_fn(|cx| {
            let presses = cx.state("presses", || 0);
            let count = cx.read(&presses);
            macro_rules! target {
                ($view:expr) => {{
                    let presses = presses.clone();
                    $view.width(80.0).height(40.0).on_press(move |cx| {
                        presses.update(cx.runtime(), |n| *n += 1);
                    }).into_any()
                }};
            }
            let targets = [
                target!(view().child(text("Box")).label("box")),
                target!(text("Text").label("text")),
                target!(image("/favicon.ico").label("image")),
            ];
            let disabled = presses.clone();
            let first = presses.clone();
            let second = presses.clone();
            Ok(v_stack()
                .children(targets)
                .child(
                    text("Disabled")
                        .label("disabled")
                        .on_press(move |cx| {
                            disabled.update(cx.runtime(), |n| *n += 100);
                        })
                        .font_size(16.0)
                        .disabled(true),
                )
                .child(
                    text("Composed")
                        .label("composed")
                        .on_press(move |cx| {
                            first.update(cx.runtime(), |n| *n += 1);
                        })
                        .font_size(16.0)
                        .on_press(move |cx| {
                            second.update(cx.runtime(), |n| *n += 10);
                        }),
                )
                .child(text(count.to_string()).status()))
        });
        Ok(CounterApp {
            mount: refract_web::mount(host, content, refract_web::MountOptions::default())?,
        })
    }

    #[cfg(feature = "browser-tests")]
    #[wasm_bindgen]
    pub fn start_fixture(host: web_sys::Element) -> Result<CounterApp, JsValue> {
        let form = view_fn(|cx| {
            let name = cx.state("name", || String::from("hello"));
            let enabled = cx.state("enabled", || false);
            let multiline = cx.state("multiline", || false);
            let submissions = cx.state("submissions", || 0u32);
            let current = cx.read(&name);
            let checked = cx.read(&enabled);
            let mode = cx.read(&multiline);
            let submitted = cx.read(&submissions);
            let input_submit = submissions.clone();
            let external = name.clone();
            Ok(v_stack()
                .gap(8.0)
                .child(
                    field(name.clone())
                        .width(260.0)
                        .multiline(mode)
                        .label("Name")
                        .on_submit(move |cx| {
                            input_submit.update(cx.runtime(), |n| *n += 1);
                        }),
                )
                .child(
                    field(name)
                        .width(280.0)
                        .multiline(true)
                        .height(160.0)
                        .label("Notes")
                        .font_size(18.0)
                        .on_submit(move |cx| {
                            submissions.update(cx.runtime(), |n| *n += 1);
                        }),
                )
                .child(view_fn(move |cx| {
                    let checked = cx.read(&enabled);
                    let binding = enabled.clone();
                    Ok(h_stack()
                        .alignment(Alignment::Center)
                        .gap(6.0)
                        .child(text(if checked { "☑" } else { "☐" }))
                        .child(text("Enabled"))
                        .label("Enabled")
                        .on_press(move |cx| {
                            binding.update(cx.runtime(), |value| *value = !*value);
                        })
                        .modify(false, move |element| {
                            element.role = Some("checkbox".into());
                            element.checked = Some(checked);
                        }))
                }))
                .child(text(format!(
                    "Model: {current}; enabled: {checked}; submissions: {submitted}"
                )))
                .child(
                    h_stack()
                        .alignment(Alignment::Center)
                        .justify(Alignment::Center)
                        .child(text("Replace"))
                        .on_press(move |cx| external.set(cx.runtime(), "external".to_owned()))
                        .label("Replace"),
                )
                .child(
                    h_stack()
                        .alignment(Alignment::Center)
                        .justify(Alignment::Center)
                        .child(text("Change input mode"))
                        .on_press(move |cx| {
                            multiline.update(cx.runtime(), |value| *value = !*value);
                        })
                        .label("Change input mode"),
                ))
        });
        Ok(CounterApp {
            mount: refract_web::mount(host, form, refract_web::MountOptions::default())?,
        })
    }
    #[wasm_bindgen]
    pub fn start(host: web_sys::Element) -> Result<CounterApp, JsValue> {
        std::panic::set_hook(Box::new(|info| {
            web_sys::console::error_1(&JsValue::from_str(&info.to_string()))
        }));
        Ok(CounterApp {
            mount: refract_web::mount(host, application(), refract_web::MountOptions::default())?,
        })
    }
}

#[cfg(all(target_arch = "wasm32", feature = "browser-tests"))]
mod events_fixture;

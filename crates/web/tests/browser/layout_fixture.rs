//! Overlay alignment, live updates, and prerender adoption in a real browser.
#![cfg(target_arch = "wasm32")]
use refract_core::prelude::*;
use refract_web::{Mount, MountOptions, PrerenderOptions};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct LayoutFixture(Mount);

#[wasm_bindgen]
impl LayoutFixture {
    pub fn hydrated(&self) -> bool {
        self.0.hydrated()
    }

    pub fn last_error(&self) -> Option<String> {
        self.0.last_error()
    }

    pub fn dispose(&self) {
        self.0.dispose();
    }
}

fn application() -> impl View {
    view_fn(|cx| {
        let step = cx.state("step", || 0_usize);
        let alignments = [Alignment::Start, Alignment::Center, Alignment::End];
        let index = cx.read(&step) % 9;
        Ok(v_stack()
            .gap(20.0)
            .child(
                z_stack()
                    .alignment(alignments[index / 3], alignments[index % 3])
                    .width(200.0)
                    .height(100.0)
                    .label("aligned")
                    .child(
                        view()
                            .width(40.0)
                            .height(20.0)
                            .background(Color::BLUE)
                            .label("back")
                            .on_press(|_| {}),
                    )
                    .child(
                        view()
                            .width(40.0)
                            .height(20.0)
                            .background(Color::WHITE)
                            .label("front")
                            .on_press(|_| {}),
                    )
                    .modify(false, |element| {
                        element.style.padding = Insets::all(10.0);
                        element.style.border = Some(Border::all(2.0, Color::BLACK));
                    }),
            )
            .child(
                z_stack()
                    .width(200.0)
                    .height(100.0)
                    .label("default")
                    .child(view().background(Color::BLUE).label("stretched"))
                    .child(
                        view()
                            .width(40.0)
                            .height(20.0)
                            .background(Color::WHITE)
                            .label("sized"),
                    ),
            )
            .child(
                text("Next alignment")
                    .label("next")
                    .on_press(move |cx| step.update(cx.runtime(), |n| *n += 1)),
            ))
    })
}

#[wasm_bindgen]
pub fn start_layout_fixture(host: web_sys::Element) -> Result<LayoutFixture, JsValue> {
    let html = refract_web::prerender(application(), PrerenderOptions::default())
        .map_err(|error| JsValue::from_str(&error.to_string()))?;
    host.set_inner_html(&html);
    refract_web::hydrate(host, application(), MountOptions::default()).map(LayoutFixture)
}

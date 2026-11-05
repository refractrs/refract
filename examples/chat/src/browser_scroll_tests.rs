//! Optional browser conformance fixture. Uses the public Rust scroll API; JS
//! drives real DOM scrolling and inspects snapshots captured by Rust callbacks.
use refract::prelude::*;
use refract_web::{Mount, MountOptions, RootSizing};
use std::{cell::RefCell, rc::Rc};
use wasm_bindgen::prelude::*;

type Events = Rc<RefCell<Vec<(u8, ScrollEvent)>>>;

#[wasm_bindgen]
pub struct ScrollFixture {
    mount: Mount,
    events: Events,
    height: Binding<f32>,
    shown: Binding<bool>,
}

#[wasm_bindgen]
impl ScrollFixture {
    pub fn events(&self) -> String {
        let events: Vec<_> = self
            .events
            .borrow()
            .iter()
            .map(|(target, event)| {
                let m = event.metrics;
                format!(
                    "[{target},{},{},{},{},{},{},{},{}]",
                    m.offset.x,
                    m.offset.y,
                    event.delta.x,
                    event.delta.y,
                    m.viewport_size.width,
                    m.viewport_size.height,
                    m.content_size.width,
                    m.content_size.height
                )
            })
            .collect();
        format!("[{}]", events.join(","))
    }
    pub fn resize_content(&self, height: f32) -> Result<(), JsValue> {
        let binding = self.height.clone();
        self.mount
            .handle()
            .update(move |rt| binding.set(rt, height))
    }
    pub fn remove_scroll(&self) -> Result<(), JsValue> {
        let shown = self.shown.clone();
        self.mount.handle().update(move |rt| shown.set(rt, false))
    }
    pub fn revision(&self) -> u64 {
        self.mount.revision()
    }
    pub fn last_error(&self) -> Option<String> {
        self.mount.last_error()
    }
    pub fn dispose(&self) {
        self.mount.dispose();
    }
}

#[wasm_bindgen]
pub fn start_scroll_fixture(
    host: web_sys::Element,
    follow: bool,
    end: bool,
) -> Result<ScrollFixture, JsValue> {
    let events: Events = Rc::default();
    let state = Rc::new(RefCell::new(None));
    let view = view_fn({
        let events = events.clone();
        let state = state.clone();
        move |cx| {
            let height = cx.state("height", || 800.0);
            let shown = cx.state("shown", || true);
            let at_end = cx.state("at-end", || end);
            *state.borrow_mut() = Some((height.clone(), shown.clone()));
            let observer = at_end.clone();
            let outer_events = events.clone();
            let inner_events = events.clone();
            let scroll = view_fn(move |cx| {
                if !cx.read(&shown) {
                    return Ok(Empty.into_any());
                }
                let outer_events = outer_events.clone();
                let inner_events = inner_events.clone();
                let observer = observer.clone();
                let height = height.clone();
                Ok(scroll_view(view_fn(move |cx| {
                    let inner_events = inner_events.clone();
                    Ok(v_stack()
                        .child(
                            scroll_view(text("inner\n".repeat(60)).width(500.0))
                                .axis(Axis::Both)
                                .on_scroll(move |_, event| {
                                    inner_events.borrow_mut().push((1, *event))
                                })
                                .width(180.0)
                                .height(80.0)
                                .label("Inner scroll"),
                        )
                        .width(600.0)
                        .height(cx.read(&height)))
                }))
                .axis(Axis::Both)
                .initial_position(if end {
                    ScrollPosition::End
                } else {
                    ScrollPosition::Start
                })
                .follow(if follow {
                    ScrollFollow::EndWhenPinned
                } else {
                    ScrollFollow::None
                })
                .on_scroll(move |cx, event| {
                    outer_events.borrow_mut().push((0, *event));
                    observer
                        .set_if_changed(cx.runtime(), event.metrics.is_at_end(Axis::Vertical, 2.0));
                })
                .fill_width()
                .height(200.0)
                .label("Outer scroll")
                .into_any())
            });
            Ok(v_stack()
                .child(scroll)
                .child(view_fn(move |cx| {
                    Ok(text(if cx.read(&at_end) {
                        "Latest"
                    } else {
                        "Earlier"
                    })
                    .label("Scroll status"))
                }))
                .fill_width()
                .fill_height())
        }
    });
    let mount = refract_web::mount(
        host,
        view,
        MountOptions {
            sizing: RootSizing::Fill,
            ..Default::default()
        },
    )?;
    let (height, shown) = state.borrow_mut().take().unwrap();
    Ok(ScrollFixture {
        mount,
        events,
        height,
        shown,
    })
}

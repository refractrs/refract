//! Browser scroll sampling. Native scrolling remains browser-owned. A capture
//! listener sees non-bubbling scroll events; ResizeObserver also schedules a
//! sample for geometry changes (including image/font layout without app patches).
//! One cancellable animation-frame callback batches reads, policy writes, then
//! notifications. Rust callbacks run later on the normal application queue.

use super::*;
use refract_core::{Layout, ScrollMetrics, ScrollOffset, ScrollSize};

pub(super) struct ScrollObserver {
    observer: web_sys::ResizeObserver,
    _resize_callback: Closure<dyn FnMut(js_sys::Array, web_sys::ResizeObserver)>,
    frame_callback: Closure<dyn FnMut(f64)>,
    frame_id: Cell<Option<i32>>,
}

impl ScrollObserver {
    pub fn new(inner: &Rc<Inner>) -> Result<Self, JsValue> {
        let weak = Rc::downgrade(inner);
        let resize_callback = Closure::new(move |_: js_sys::Array, _: web_sys::ResizeObserver| {
            if let Some(inner) = weak.upgrade() {
                request(&inner);
            }
        });
        let observer = web_sys::ResizeObserver::new(resize_callback.as_ref().unchecked_ref())?;

        let weak = Rc::downgrade(inner);
        let frame_callback = Closure::new(move |_: f64| {
            let Some(inner) = weak.upgrade().filter(|inner| inner.active.get()) else {
                return;
            };
            if let Some(scroll) = inner.scroll.borrow().as_ref() {
                scroll.frame_id.set(None);
            }
            sample(&inner);
        });
        Ok(Self {
            observer,
            _resize_callback: resize_callback,
            frame_callback,
            frame_id: Cell::new(None),
        })
    }

    /// Reconnect only after structural/configuration patches. Observe descendants
    /// too: overflow can change even when a fixed-size direct child does not grow.
    pub fn reconnect(&self, dom: &Dom) -> Result<(), JsValue> {
        self.observer.disconnect();
        let mut observed = HashSet::new();
        for (id, entry) in &dom.entries {
            if !managed(&entry.element) {
                continue;
            }
            if observed.insert(*id) {
                self.observer.observe(&entry.node);
            }
            let descendants = entry.node.query_selector_all("[data-refract-id]")?;
            for index in 0..descendants.length() {
                let Some(node) = descendants
                    .item(index)
                    .and_then(|n| n.dyn_into::<web_sys::Element>().ok())
                else {
                    continue;
                };
                if let Some(id) = node
                    .get_attribute("data-refract-id")
                    .and_then(|id| id.parse::<u64>().ok())
                    .map(ElementId)
                {
                    if observed.insert(id) {
                        self.observer.observe(&node);
                    }
                }
            }
        }
        Ok(())
    }
}

impl Drop for ScrollObserver {
    fn drop(&mut self) {
        self.observer.disconnect();
        if let Some(id) = self.frame_id.take() {
            if let Some(window) = web_sys::window() {
                let _ = window.cancel_animation_frame(id);
            }
        }
    }
}

fn managed(element: &Element) -> bool {
    element.scroll.is_some()
}

pub(super) fn request(inner: &Rc<Inner>) {
    if !inner.active.get() {
        return;
    }
    let scroll = inner.scroll.borrow();
    let Some(scroll) = scroll.as_ref() else {
        return;
    };
    // Scroll events and resize notifications share one pending sample.
    if scroll.frame_id.get().is_some() {
        return;
    }
    if let Some(window) = web_sys::window() {
        match window.request_animation_frame(scroll.frame_callback.as_ref().unchecked_ref()) {
            Ok(id) => scroll.frame_id.set(Some(id)),
            Err(error) => *inner.error.borrow_mut() = Some(format!("scroll frame: {error:?}")),
        }
    }
}

fn offset(node: &HtmlElement, property: &str) -> f64 {
    // Stable web-sys bindings expose integer scrollTop/Left. Read the actual DOM
    // doubles so fractional movement isn't lost to integer conversion.
    js_sys::Reflect::get(node.as_ref(), &JsValue::from_str(property))
        .ok()
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0)
}

fn metrics(node: &HtmlElement) -> ScrollMetrics {
    let viewport = ScrollSize {
        width: node.client_width() as f64,
        height: node.client_height() as f64,
    };
    let mut metrics = ScrollMetrics {
        offset: ScrollOffset {
            x: offset(node, "scrollLeft"),
            y: offset(node, "scrollTop"),
        },
        viewport_size: viewport,
        content_size: ScrollSize {
            width: (node.scroll_width() as f64).max(viewport.width),
            height: (node.scroll_height() as f64).max(viewport.height),
        },
    };
    let max = metrics.max_offset();
    metrics.offset.x = metrics.offset.x.clamp(0.0, max.x);
    metrics.offset.y = metrics.offset.y.clamp(0.0, max.y);
    metrics
}

fn sample(inner: &Rc<Inner>) {
    let mut dom = inner.dom.borrow_mut();
    // Read all geometry before policy writes, avoiding a layout read/write loop.
    let samples: Vec<_> = dom
        .entries
        .iter()
        .filter(|(_, entry)| managed(&entry.element))
        .map(|(id, entry)| (*id, metrics(&entry.node)))
        .collect();

    let mut events = Vec::new();
    for (id, before) in samples {
        let entry = dom.entries.get_mut(&id).unwrap();
        let Layout::Scroll(axis) = entry.element.style.layout else {
            continue;
        };
        let options = entry.element.scroll.unwrap();
        let resolved = entry.scroll_tracker.resolve(options, axis, before);
        if resolved.offset != before.offset {
            // Numeric assignment is immediate; CSS smooth scrolling isn't enabled.
            let _ = js_sys::Reflect::set(
                entry.node.as_ref(),
                &JsValue::from_str("scrollLeft"),
                &JsValue::from_f64(resolved.offset.x),
            );
            let _ = js_sys::Reflect::set(
                entry.node.as_ref(),
                &JsValue::from_str("scrollTop"),
                &JsValue::from_f64(resolved.offset.y),
            );
        }
        // Browser may quantize/clamp the requested offset. Report what it accepted.
        let mut actual = resolved;
        actual.offset = ScrollOffset {
            x: offset(&entry.node, "scrollLeft").clamp(0.0, resolved.max_offset().x),
            y: offset(&entry.node, "scrollTop").clamp(0.0, resolved.max_offset().y),
        };
        entry.scroll_tracker.record(actual);
        if let Some(event) = entry.scroll_tracker.notification(actual, options.observe) {
            events.push(Queued::Event(id, UiEvent::Scroll(event)));
        }
    }

    drop(dom);
    if !events.is_empty() {
        inner.queue.borrow_mut().extend(events);
        schedule(inner);
    }
}

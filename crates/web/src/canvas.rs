//! Canvas resources, size/visibility observation, and presentation-only drawing.

use super::*;
use refract_core::{CanvasCache, CanvasSchedule, DrawingCommand, Size};

#[derive(Default)]
pub(super) struct CanvasDom {
    pub(super) cache: CanvasCache,
    pub(super) commands: Option<Rc<[DrawingCommand]>>,
    pub visible: bool,
    pub active: bool,
}

pub(super) struct CanvasObserver {
    resize: web_sys::ResizeObserver,
    _resize: Closure<dyn FnMut(js_sys::Array, web_sys::ResizeObserver)>,
    intersection: web_sys::IntersectionObserver,
    _intersection: Closure<dyn FnMut(js_sys::Array, web_sys::IntersectionObserver)>,
    media: web_sys::MediaQueryList,
    callback: Closure<dyn FnMut(web_sys::Event)>,
    document: Document,
}

impl CanvasObserver {
    pub fn new(inner: &Rc<Inner>) -> Result<Self, JsValue> {
        let weak = Rc::downgrade(inner);
        let resize = Closure::new(move |_: js_sys::Array, _: web_sys::ResizeObserver| {
            if let Some(inner) = weak.upgrade() {
                schedule(&inner);
            }
        });
        let observer = web_sys::ResizeObserver::new(resize.as_ref().unchecked_ref())?;
        let weak = Rc::downgrade(inner);
        let intersection = Closure::new(
            move |entries: js_sys::Array, _: web_sys::IntersectionObserver| {
                if let Some(inner) = weak.upgrade() {
                    {
                        let mut dom = inner.dom.borrow_mut();
                        for value in entries.iter() {
                            let entry =
                                value.unchecked_into::<web_sys::IntersectionObserverEntry>();
                            let id = entry
                                .target()
                                .get_attribute("data-refract-id")
                                .and_then(|v| v.parse().ok())
                                .map(ElementId);
                            if let Some(canvas) = id
                                .and_then(|id| dom.entries.get_mut(&id))
                                .map(|e| &mut e.canvas)
                            {
                                canvas.visible = entry.is_intersecting();
                                if !canvas.visible {
                                    canvas.cache.pause();
                                }
                            }
                        }
                    }
                    schedule(&inner);
                }
            },
        );
        let visibility = web_sys::IntersectionObserver::new(intersection.as_ref().unchecked_ref())?;
        let document = inner.dom.borrow().document.clone();
        let media = web_sys::window()
            .unwrap()
            .match_media("(prefers-reduced-motion: reduce)")?
            .unwrap();
        let weak = Rc::downgrade(inner);
        let callback = Closure::new(move |_: web_sys::Event| {
            if let Some(inner) = weak.upgrade() {
                for entry in inner.dom.borrow_mut().entries.values_mut() {
                    entry.canvas.cache.pause();
                }
                schedule(&inner);
            }
        });
        media.add_event_listener_with_callback("change", callback.as_ref().unchecked_ref())?;
        document.add_event_listener_with_callback(
            "visibilitychange",
            callback.as_ref().unchecked_ref(),
        )?;
        Ok(Self {
            resize: observer,
            _resize: resize,
            intersection: visibility,
            _intersection: intersection,
            media,
            callback,
            document,
        })
    }

    pub fn reconnect(&self, dom: &Dom) {
        self.resize.disconnect();
        self.intersection.disconnect();
        for entry in dom.entries.values().filter(|e| e.element.canvas.is_some()) {
            self.resize.observe(&entry.node);
            self.intersection.observe(&entry.node);
        }
    }
}

impl Drop for CanvasObserver {
    fn drop(&mut self) {
        self.resize.disconnect();
        self.intersection.disconnect();
        let _ = self
            .media
            .remove_event_listener_with_callback("change", self.callback.as_ref().unchecked_ref());
        let _ = self.document.remove_event_listener_with_callback(
            "visibilitychange",
            self.callback.as_ref().unchecked_ref(),
        );
    }
}

impl Dom {
    pub(super) fn draw_canvases(&mut self, now: std::time::Duration) -> Result<(), JsValue> {
        let hidden = self.document.hidden();
        let reduced = web_sys::window()
            .unwrap()
            .match_media("(prefers-reduced-motion: reduce)")?
            .is_some_and(|m| m.matches());
        self.canvas_active = false;
        for id in &self.canvas_ids {
            let Some(entry) = self.entries.get_mut(id) else {
                continue;
            };
            let Some(content) = self.presentation.element(*id, now).and_then(|e| e.canvas) else {
                continue;
            };
            let s = &entry.element.style;
            let size = Size::new(
                (f64::from(entry.node.client_width())
                    - f64::from(s.padding.left + s.padding.right))
                .max(0.0),
                (f64::from(entry.node.client_height())
                    - f64::from(s.padding.top + s.padding.bottom))
                .max(0.0),
            );
            let visible = !hidden && entry.canvas.visible && size.width > 0.0 && size.height > 0.0;
            entry.canvas.active =
                visible && !reduced && content.schedule == CanvasSchedule::Animation;
            self.canvas_active |= entry.canvas.active;
            if !visible {
                entry.canvas.cache.pause();
                continue;
            }
            let commands = entry
                .canvas
                .cache
                .render(&content, size, now, true, reduced)
                .map_err(|e| JsValue::from_str(&e.to_string()))?;
            if entry
                .canvas
                .commands
                .as_ref()
                .is_none_or(|old| !Rc::ptr_eq(old, &commands))
            {
                let mut tree =
                    crate::svg::scene(&commands, size, &format!("{}-{}", self.canvas_prefix, id.0));
                tree.position(s.padding.left, s.padding.top);
                self.calls += crate::svg::update(&entry.node, &tree)?;
                entry.canvas.commands = Some(commands);
            }
        }
        Ok(())
    }
}

//! Browser observations and document ownership. Callbacks only schedule sampling;
//! graph writes and platform requests run in the mount's normal flush.
use super::*;
use refract_core::{HistoryMode, SurfaceCommand, SurfaceError, Viewport};

thread_local! {
    static OWNERS: RefCell<Vec<Document>> = const { RefCell::new(Vec::new()) };
}

pub(super) struct DocumentLease(Option<Document>);

impl DocumentLease {
    pub fn claim(document: &Document, managed: bool) -> Result<Self, JsValue> {
        if !managed {
            return Ok(Self(None));
        }
        OWNERS.with(|owners| {
            let mut owners = owners.borrow_mut();
            if owners.contains(document) {
                return Err(JsValue::from_str(
                    "document already has a controlling Refract mount",
                ));
            }
            owners.push(document.clone());
            Ok(Self(Some(document.clone())))
        })
    }
}

impl Drop for DocumentLease {
    fn drop(&mut self) {
        if let Some(document) = &self.0 {
            OWNERS.with(|owners| owners.borrow_mut().retain(|owner| owner != document));
        }
    }
}

pub(super) struct SurfaceObserver {
    host: web_sys::Element,
    window: web_sys::Window,
    fill: bool,
    size: Rc<Cell<Option<(f64, f64)>>>,
    resize: web_sys::ResizeObserver,
    _resize_callback: Closure<dyn FnMut(js_sys::Array, web_sys::ResizeObserver)>,
    mutation: web_sys::MutationObserver,
    _mutation_callback: Closure<dyn FnMut(js_sys::Array, web_sys::MutationObserver)>,
    event_callback: Closure<dyn FnMut(web_sys::Event)>,
    resolution: RefCell<Option<web_sys::MediaQueryList>>,
    scale: Cell<f64>,
    _lease: DocumentLease,
}

const EVENTS: &[&str] = &[
    "resize",
    "popstate",
    "hashchange",
    "refract-location",
    "focus",
    "blur",
];

pub(super) fn request(inner: &Rc<Inner>) {
    inner.surface_dirty.set(true);
    schedule(inner);
}

impl SurfaceObserver {
    pub fn new(
        inner: &Rc<Inner>,
        host: web_sys::Element,
        fill: bool,
        lease: DocumentLease,
    ) -> Result<Self, JsValue> {
        let document = inner.dom.borrow().document.clone();
        let window = document
            .default_view()
            .ok_or_else(|| JsValue::from_str("document has no window"))?;
        let size = Rc::new(Cell::new(None));
        let weak = Rc::downgrade(inner);
        let measured = size.clone();
        let resize_callback =
            Closure::new(move |entries: js_sys::Array, _: web_sys::ResizeObserver| {
                if let Some(inner) = weak.upgrade() {
                    if let Ok(entry) = entries.get(0).dyn_into::<web_sys::ResizeObserverEntry>() {
                        let rect = entry.content_rect();
                        measured.set(Some((rect.width(), rect.height())));
                    }
                    request(&inner);
                }
            });
        let resize = web_sys::ResizeObserver::new(resize_callback.as_ref().unchecked_ref())?;
        let weak = Rc::downgrade(inner);
        let mutation_callback =
            Closure::new(move |_: js_sys::Array, _: web_sys::MutationObserver| {
                if let Some(inner) = weak.upgrade() {
                    request(&inner);
                }
            });
        let mutation = web_sys::MutationObserver::new(mutation_callback.as_ref().unchecked_ref())?;
        let weak = Rc::downgrade(inner);
        let event_callback = Closure::new(move |_: web_sys::Event| {
            if let Some(inner) = weak.upgrade() {
                request(&inner);
            }
        });
        let observer = Self {
            host,
            window,
            fill,
            size,
            resize,
            _resize_callback: resize_callback,
            mutation,
            _mutation_callback: mutation_callback,
            event_callback,
            resolution: RefCell::new(None),
            scale: Cell::new(0.0),
            _lease: lease,
        };
        // Construct the owner before attaching resources so partial setup also cleans up.
        observer.resize.observe(&observer.host);
        if let Some(head) = document.head() {
            let options = web_sys::MutationObserverInit::new();
            options.set_child_list(true);
            options.set_character_data(true);
            options.set_subtree(true);
            observer.mutation.observe_with_options(&head, &options)?;
        }
        for event in EVENTS {
            observer.window.add_event_listener_with_callback(
                event,
                observer.event_callback.as_ref().unchecked_ref(),
            )?;
        }
        observer.watch_resolution()?;
        Ok(observer)
    }

    fn watch_resolution(&self) -> Result<(), JsValue> {
        let scale = self.window.device_pixel_ratio();
        if self.scale.replace(scale) == scale {
            return Ok(());
        }
        if let Some(old) = self.resolution.borrow_mut().take() {
            old.remove_event_listener_with_callback(
                "change",
                self.event_callback.as_ref().unchecked_ref(),
            )?;
        }
        if let Some(query) = self
            .window
            .match_media(&format!("(resolution: {scale}dppx)"))?
        {
            query.add_event_listener_with_callback(
                "change",
                self.event_callback.as_ref().unchecked_ref(),
            )?;
            *self.resolution.borrow_mut() = Some(query);
        }
        Ok(())
    }

    pub fn sample(&self, inner: &Inner, rt: &mut Runtime) -> Result<(), JsValue> {
        self.watch_resolution()?;
        let active = inner.dom.borrow().document.has_focus()?;
        inner.surface_driver.publish_active(rt, active);
        inner
            .dom
            .borrow()
            .root
            .set_attribute("data-refract-active", if active { "true" } else { "false" })?;
        let (width, height) = match self.size.get() {
            Some(size) => size,
            None => {
                // client dimensions include padding; the observer reports content dimensions.
                let style = self.window.get_computed_style(&self.host)?;
                let pixels = |name| {
                    style
                        .as_ref()
                        .and_then(|style| style.get_property_value(name).ok())
                        .and_then(|value| value.trim_end_matches("px").parse::<f64>().ok())
                        .unwrap_or(0.0)
                };
                (
                    (f64::from(self.host.client_width())
                        - pixels("padding-left")
                        - pixels("padding-right"))
                    .max(0.0),
                    (f64::from(self.host.client_height())
                        - pixels("padding-top")
                        - pixels("padding-bottom"))
                    .max(0.0),
                )
            }
        };
        let location = refract_core::Url::parse(&self.window.location().href()?)
            .map_err(|error| JsValue::from_str(&error.to_string()))?;
        let title = inner.dom.borrow().document.title();
        rt.batch(|rt| {
            let _ = inner.surface_driver.publish_viewport(
                rt,
                Viewport {
                    width,
                    height: self.fill.then_some(height),
                    scale_factor: self.scale.get(),
                },
            );
            inner.surface_driver.publish_title(rt, title);
            inner.surface_driver.publish_location(rt, Some(location));
        });
        Ok(())
    }

    pub fn apply(&self, inner: &Inner, rt: &mut Runtime) {
        for request in inner.surface_driver.take_requests(rt) {
            let result = self.execute(inner, rt, &request.command);
            request.complete(rt, result);
        }
    }

    fn execute(
        &self,
        inner: &Inner,
        rt: &mut Runtime,
        command: &SurfaceCommand,
    ) -> Result<(), SurfaceError> {
        match command {
            SurfaceCommand::SetTitle(title) => {
                let document = inner.dom.borrow().document.clone();
                if document.title() != *title {
                    document.set_title(title);
                }
                inner.surface_driver.publish_title(rt, document.title());
            }
            SurfaceCommand::Navigate { reference, mode } => {
                let href = self.window.location().href().map_err(host_error)?;
                let current = refract_core::Url::parse(&href)
                    .map_err(|error| SurfaceError::InvalidUrl(error.to_string()))?;
                let url = refract_core::resolve_location(Some(&current), reference)?;
                let history = self.window.history().map_err(host_error)?;
                // Preserve foreign state when replacing; new entries belong to this navigation.
                match mode {
                    HistoryMode::Push => {
                        history.push_state_with_url(&JsValue::NULL, "", Some(url.as_str()))
                    }
                    HistoryMode::Replace => history.replace_state_with_url(
                        &history.state().map_err(host_error)?,
                        "",
                        Some(url.as_str()),
                    ),
                }
                .map_err(host_error)?;
                let url =
                    refract_core::Url::parse(&self.window.location().href().map_err(host_error)?)
                        .map_err(|error| SurfaceError::InvalidUrl(error.to_string()))?;
                inner.surface_driver.publish_location(rt, Some(url));
                // History writes do not emit popstate. Notify other observing mounts.
                if let Ok(event) = web_sys::Event::new("refract-location") {
                    let _ = self.window.dispatch_event(&event);
                }
            }
        }
        Ok(())
    }
}

fn host_error(error: JsValue) -> SurfaceError {
    SurfaceError::Host(format!("{error:?}"))
}

impl Drop for SurfaceObserver {
    fn drop(&mut self) {
        self.resize.disconnect();
        self.mutation.disconnect();
        for event in EVENTS {
            let _ = self.window.remove_event_listener_with_callback(
                event,
                self.event_callback.as_ref().unchecked_ref(),
            );
        }
        if let Some(query) = self.resolution.borrow_mut().take() {
            let _ = query.remove_event_listener_with_callback(
                "change",
                self.event_callback.as_ref().unchecked_ref(),
            );
        }
    }
}

//! Mount lifecycle, event delegation, and retained DOM patching.
//!
//! Each mount owns a portable app and a browser registry. Events and queued
//! mutations update the app, then the adapter applies and acknowledges its patches.
//! Core state is committed only after the corresponding DOM update succeeds.

// Scroll sampling shares private mount state but lives in a separate source file.
#[path = "events.rs"]
mod events;
#[path = "scroll.rs"]
mod scroll;
#[path = "surface.rs"]
mod surface;

use js_sys::Function;
use refract_core::{
    App, Element, ElementId, ElementKind, Event as UiEvent, Interaction, Patch, Runtime,
    ScopeToken, Update, View,
};
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    rc::{Rc, Weak},
};
use wasm_bindgen::{prelude::*, JsCast};
use web_sys::{
    Document, HtmlElement, HtmlInputElement, HtmlTextAreaElement, Node, ShadowRootInit,
    ShadowRootMode,
};

/// Choose whether the root sizes to its content or fills a definite host size.
#[derive(Clone, Copy, Default)]
pub enum RootSizing {
    /// Let the mounted tree determine the root's height.
    #[default]
    Content,
    /// Fill a host whose width and height are already nonzero.
    Fill,
}

/// Configure the mount boundary, root sizing, and stylesheet policy.
#[derive(Clone)]
pub struct MountOptions {
    /// Attach an open shadow root to isolate the application's styles.
    pub shadow: bool,
    pub sizing: RootSizing,
    /// Claim document title/history control. Only one mount per document may own it.
    pub manage_document: bool,
    /// CSP nonce applied to the mount's injected base stylesheet.
    pub style_nonce: Option<String>,
}

impl Default for MountOptions {
    fn default() -> Self {
        Self {
            shadow: true,
            sizing: RootSizing::Content,
            manage_document: false,
            style_nonce: None,
        }
    }
}

/// Last applied properties and browser-owned resources for one retained element.
struct Entry {
    node: HtmlElement,
    element: Element,
    styles: BTreeMap<&'static str, String>,
    composing: bool,
    // Keep model writes separate from the browser's provisional IME text.
    pending_value: Option<String>,
    image_url: Option<crate::image::ImageUrl>,
    scroll_tracker: refract_core::ScrollTracker,
}

/// Host registry and revision used to validate and apply core patches.
struct Dom {
    document: Document,
    root: HtmlElement,
    stylesheet: web_sys::Element,
    entries: HashMap<ElementId, Entry>,
    revision: u64,
    calls: u64,
    adopting: bool,
    hydrated: bool,
    presentation: refract_core::PresentationState,
}

/// Own the JS callback until its root listener has been detached.
struct Listener {
    root: web_sys::EventTarget,
    name: &'static str,
    callback: Closure<dyn FnMut(web_sys::Event)>,
    capture: bool,
}

impl Drop for Listener {
    fn drop(&mut self) {
        let _ = self.root.remove_event_listener_with_callback_and_bool(
            self.name,
            self.callback.as_ref().unchecked_ref(),
            self.capture,
        );
    }
}

type Mutation = Box<dyn FnOnce(&mut Runtime)>;

/// Model mutations and deferred input share one FIFO queue.
enum Queued {
    Mutation(Option<ScopeToken>, Mutation),
    Event(ElementId, UiEvent),
    Focus(Option<ElementId>),
}

/// Shared mount state. Browser callbacks hold weak references to avoid cycles.
struct Inner {
    app: RefCell<Option<App>>,
    queue: RefCell<VecDeque<Queued>>,
    dom: RefCell<Dom>,
    listeners: RefCell<Vec<Listener>>,
    captures: RefCell<HashMap<u64, ElementId>>,
    suppressed_clicks: RefCell<HashMap<u64, Rc<()>>>,
    scroll: RefCell<Option<scroll::ScrollObserver>>,
    surface_driver: refract_core::SurfaceDriver,
    surface: RefCell<Option<surface::SurfaceObserver>>,
    surface_dirty: Cell<bool>,
    scheduled: Cell<bool>,
    // A failed DOM update stops work without marking resource cleanup complete.
    active: Cell<bool>,
    disposed: Cell<bool>,
    error: RefCell<Option<String>>,
    microtasks: Cell<u8>,
    animation_frame: Cell<Option<i32>>,
    animation_callback: RefCell<Option<Closure<dyn FnMut(f64)>>>,
}

/// Own this handle for the mount's lifetime. Dropping it releases browser resources.
pub struct Mount(Rc<Inner>);

/// A weak handle for submitting model updates from browser callbacks.
#[derive(Clone)]
pub struct UpdateHandle {
    inner: Weak<Inner>,
    scope: Option<ScopeToken>,
}

impl UpdateHandle {
    /// Admit a mutation to the owning UI queue. Success means admitted, not painted.
    pub fn update(&self, f: impl FnOnce(&mut Runtime) + 'static) -> Result<(), JsValue> {
        let inner = self
            .inner
            .upgrade()
            .filter(|m| m.active.get())
            .ok_or_else(|| JsValue::from_str("mount disposed"))?;

        if self.scope.as_ref().is_some_and(|s| !s.is_alive()) {
            return Err(JsValue::from_str("view scope disposed"));
        }

        if inner.queue.borrow().len() >= 1024 {
            return Err(JsValue::from_str("Refract update queue is full"));
        }

        inner
            .queue
            .borrow_mut()
            .push_back(Queued::Mutation(self.scope.clone(), Box::new(f)));
        schedule(&inner);
        Ok(())
    }
}

impl Mount {
    /// Whether startup reused a matching prerendered tree instead of mounting anew.
    pub fn hydrated(&self) -> bool {
        self.0.dom.borrow().hydrated
    }
    /// Read metadata or submit requests through the normal update handle.
    pub fn surface(&self) -> Rc<refract_core::Surface> {
        self.0.surface_driver.surface()
    }

    /// Resample after an embedding router calls pushState/replaceState itself.
    pub fn refresh_location(&self) {
        surface::request(&self.0);
    }

    /// Submit work for as long as this mount remains active.
    pub fn handle(&self) -> UpdateHandle {
        UpdateHandle {
            inner: Rc::downgrade(&self.0),
            scope: None,
        }
    }

    /// Drop queued work if its owning view unmounts before it runs.
    pub fn scoped_handle(&self, scope: ScopeToken) -> UpdateHandle {
        UpdateHandle {
            inner: Rc::downgrade(&self.0),
            scope: Some(scope),
        }
    }

    /// Latest update revision applied to the DOM.
    pub fn revision(&self) -> u64 {
        self.0.dom.borrow().revision
    }

    /// Cumulative patch-operation count for diagnostics, not an exact browser API count.
    pub fn dom_calls(&self) -> u64 {
        self.0.dom.borrow().calls
    }

    /// Most recent error recorded by an asynchronous flush or scroll request.
    pub fn last_error(&self) -> Option<String> {
        self.0.error.borrow().clone()
    }

    /// Release listeners, observers, app state, and DOM resources. Safe to repeat.
    pub fn dispose(&self) {
        dispose(&self.0)
    }
}

impl Drop for Mount {
    fn drop(&mut self) {
        self.dispose()
    }
}

fn dispose(inner: &Inner) {
    if inner.disposed.replace(true) {
        return;
    }

    // Stop incoming work before dropping callbacks and the application they use.
    inner.active.set(false);
    if let Some(id) = inner.animation_frame.take() {
        if let Some(window) = web_sys::window() { let _ = window.cancel_animation_frame(id); }
    }
    inner.animation_callback.borrow_mut().take();
    inner.queue.borrow_mut().clear();
    inner.listeners.borrow_mut().clear();
    inner.scroll.borrow_mut().take();
    inner.surface.borrow_mut().take();
    if let Some(app) = inner.app.borrow_mut().as_mut() {
        inner.surface_driver.close(app.runtime_mut());
    }
    inner.app.borrow_mut().take();

    let mut dom = inner.dom.borrow_mut();
    dom.root.remove();
    dom.stylesheet.remove();
    dom.entries.clear();
    dom.presentation.clear();
}

/// Mount a view in a browser element and apply its initial tree synchronously.
pub fn mount(
    host: web_sys::Element,
    view: impl View,
    options: MountOptions,
) -> Result<Mount, JsValue> {
    mount_inner(host, view, options, false)
}

/// Adopt matching prerendered HTML, or mount normally when no snapshot exists.
/// A mismatching initial tree is replaced before effects start. Initial inputs
/// must match the build export; event handlers and reactive state are recreated.
pub fn hydrate(
    host: web_sys::Element,
    view: impl View,
    options: MountOptions,
) -> Result<Mount, JsValue> {
    mount_inner(host, view, options, true)
}

fn direct_child(parent: &Node, attribute: &str) -> Option<web_sys::Element> {
    let children = parent.child_nodes();
    (0..children.length())
        .filter_map(|index| children.item(index))
        .filter_map(|node| node.dyn_into::<web_sys::Element>().ok())
        .find(|element| element.has_attribute(attribute))
}

fn mount_inner(
    host: web_sys::Element,
    view: impl View,
    options: MountOptions,
    hydrate: bool,
) -> Result<Mount, JsValue> {
    let document = host
        .owner_document()
        .ok_or_else(|| JsValue::from_str("host has no document"))?;

    let lease = surface::DocumentLease::claim(&document, options.manage_document)?;

    if matches!(options.sizing, RootSizing::Fill)
        && (host.client_width() == 0 || host.client_height() == 0)
    {
        return Err(JsValue::from_str(
            "fill mount requires a host with nonzero definite dimensions",
        ));
    }

    // Own a root and stylesheet inside the chosen host boundary, leaving any
    // other host content alone when the mount is disposed.
    let parent: Node = if options.shadow {
        if let Some(shadow) = host.shadow_root() {
            if !hydrate
                || (shadow.has_child_nodes()
                    && direct_child(shadow.as_ref(), "data-refract-prerender").is_none())
            {
                return Err(JsValue::from_str(
                    "host already has a shadow root; use a new host element",
                ));
            }
            shadow.into()
        } else {
            let template = if hydrate {
                direct_child(host.as_ref(), "data-refract-prerender")
                    .and_then(|element| element.dyn_into::<web_sys::HtmlTemplateElement>().ok())
            } else {
                None
            };
            let shadow = host.attach_shadow(&ShadowRootInit::new(ShadowRootMode::Open))?;
            // innerHTML/client navigation does not activate declarative shadow
            // roots. Materialize the supplied template without parsing strings.
            if let Some(template) = template {
                shadow.append_child(&template.content())?;
                template.remove();
            }
            shadow.into()
        }
    } else {
        host.clone().into()
    };

    let existing = if hydrate {
        direct_child(&parent, "data-refract-prerender")
    } else {
        None
    };
    let adopting = existing.is_some();
    if existing
        .as_ref()
        .is_some_and(|root| root.get_attribute("data-refract-prerender").as_deref() != Some("1"))
    {
        return Err(JsValue::from_str("unsupported Refract prerender format"));
    }
    let root: HtmlElement = match existing {
        Some(root) => root,
        None => document.create_element("div")?,
    }
    .dyn_into()?;
    root.set_class_name("refract-root");
    if matches!(options.sizing, RootSizing::Fill) {
        root.style().set_property("width", "100%")?;
        root.style().set_property("height", "100%")?;
    }

    let existing_style = if adopting {
        direct_child(&parent, "data-refract-style")
    } else {
        None
    };
    let stylesheet = match existing_style {
        Some(style) if style.tag_name() == "STYLE" => style,
        _ => {
            let style = document.create_element("style")?;
            style.set_text_content(Some(BASE_STYLE));
            parent.append_child(&style)?;
            style
        }
    };
    if let Some(nonce) = options.style_nonce {
        stylesheet.set_attribute("nonce", &nonce)?;
    }

    if !adopting {
        parent.append_child(&root)?;
    }

    let mut app = App::new(view);
    let surface_driver = refract_core::SurfaceDriver::install(
        &mut app,
        document.title(),
        None,
        refract_core::SurfaceCapabilities {
            title: options.manage_document,
            navigation: options.manage_document,
        },
    );
    let inner = Rc::new(Inner {
        app: RefCell::new(Some(app)),
        surface_driver,
        surface: RefCell::new(None),
        surface_dirty: Cell::new(true),
        queue: RefCell::new(VecDeque::new()),
        dom: RefCell::new(Dom {
            document,
            root: root.clone(),
            stylesheet,
            entries: HashMap::new(),
            revision: 0,
            calls: 0,
            adopting,
            hydrated: false,
            presentation: Default::default(),
        }),
        listeners: RefCell::new(vec![]),
        captures: RefCell::new(HashMap::new()),
        suppressed_clicks: RefCell::new(HashMap::new()),
        scroll: RefCell::new(None),
        scheduled: Cell::new(false),
        active: Cell::new(true),
        disposed: Cell::new(false),
        error: RefCell::new(None),
        microtasks: Cell::new(0),
        animation_frame: Cell::new(None),
        animation_callback: RefCell::new(None),
    });

    // Graph writes request a flush without keeping the mount alive on their own.
    let weak = Rc::downgrade(&inner);
    inner
        .app
        .borrow_mut()
        .as_mut()
        .unwrap()
        .runtime_mut()
        .set_waker(move || {
            if let Some(inner) = weak.upgrade() {
                schedule(&inner)
            }
        });

    let weak = Rc::downgrade(&inner);
    inner
        .app
        .borrow_mut()
        .as_mut()
        .unwrap()
        .runtime_mut()
        .set_message_waker(move || {
            if let Some(inner) = weak.upgrade() {
                schedule(&inner);
            }
        });

    // Observe Tab before it enters the mount, including after pointer use here.
    // Root-only key listeners miss navigation from elsewhere in the document.
    let weak = Rc::downgrade(&inner);
    let callback = Closure::new(move |event: web_sys::Event| {
        if event
            .dyn_ref::<web_sys::KeyboardEvent>()
            .is_some_and(|key| {
                key.key() == "Tab"
                    && !key.ctrl_key()
                    && !key.meta_key()
                    && !key.alt_key()
                    && !key.is_composing()
            })
        {
            if let Some(inner) = weak.upgrade() {
                let _ = inner.dom.borrow().root.set_attribute("data-refract-input", "keyboard");
            }
        }
    });
    let target: web_sys::EventTarget = inner.dom.borrow().document.clone().into();
    target.add_event_listener_with_callback_and_bool("keydown", callback.as_ref().unchecked_ref(), true)?;
    inner.listeners.borrow_mut().push(Listener {
        root: target,
        name: "keydown",
        callback,
        capture: true,
    });

    // Delegate input from the root. Scroll uses capture because native scroll
    // events do not bubble, and its geometry reads are batched separately.
    for name in [
        "click",
        "input",
        "compositionstart",
        "keydown",
        "keyup",
        "focusin",
        "focusout",
        "pointerdown",
        "pointermove",
        "pointerup",
        "pointercancel",
        "pointerenter",
        "pointerleave",
        "lostpointercapture",
        "compositionend",
        "scroll",
    ] {
        let weak = Rc::downgrade(&inner);
        let callback = Closure::new(move |event: web_sys::Event| {
            if let Some(inner) = weak.upgrade() {
                if event.type_() == "scroll" {
                    scroll::request(&inner);
                } else {
                    dispatch(&inner, &event)
                }
            }
        });

        if let Err(error) = root.add_event_listener_with_callback_and_bool(
            name,
            callback.as_ref().unchecked_ref(),
            matches!(name, "scroll" | "pointerenter" | "pointerleave"),
        ) {
            dispose(&inner);
            return Err(error);
        }

        inner.listeners.borrow_mut().push(Listener {
            root: root.clone().into(),
            name,
            callback,
            capture: matches!(name, "scroll" | "pointerenter" | "pointerleave"),
        });
    }

    match scroll::ScrollObserver::new(&inner) {
        Ok(scroll) => *inner.scroll.borrow_mut() = Some(scroll),
        Err(error) => {
            dispose(&inner);
            return Err(error);
        }
    }

    match surface::SurfaceObserver::new(
        &inner,
        host,
        matches!(options.sizing, RootSizing::Fill),
        lease,
    ) {
        Ok(observer) => *inner.surface.borrow_mut() = Some(observer),
        Err(error) => {
            dispose(&inner);
            return Err(error);
        }
    }

    // Return a usable initial tree, or clean up the partially initialized mount.
    if let Err(error) = flush(&inner) {
        dispose(&inner);
        return Err(error);
    }

    Ok(Mount(inner))
}

/// Coalesce pending writes into one flush; callbacks may outlive a disposed mount.
fn animation_now() -> std::time::Duration {
    std::time::Duration::from_secs_f64(web_sys::window().unwrap().performance().unwrap().now() / 1000.0)
}

fn request_animation(inner: &Rc<Inner>) -> Result<(), JsValue> {
    if !inner.active.get() || inner.animation_frame.get().is_some() || !inner.dom.borrow().presentation.is_animating() { return Ok(()); }
    if inner.animation_callback.borrow().is_none() {
        let weak = Rc::downgrade(inner);
        let callback = Closure::wrap(Box::new(move |timestamp: f64| {
            let Some(inner) = weak.upgrade() else { return; };
            inner.animation_frame.set(None);
            if !inner.active.get() { return; }
            let result = (|| -> Result<(), JsValue> {
                let mut dom = inner.dom.borrow_mut();
                let samples = dom.presentation.advance(std::time::Duration::from_secs_f64(timestamp / 1000.0));
                for (id, element) in samples {
                    if let Some(entry) = dom.entries.get_mut(&id) {
                        let css = crate::style::css(&element);
                        let style = entry.node.style();
                        let mut calls = 0;
                        for key in entry.styles.keys() { if !css.contains_key(key) { style.remove_property(key)?; calls += 1; } }
                        for (key,value) in &css { if entry.styles.get(key) != Some(value) { style.set_property(key,value)?; calls += 1; } }
                        entry.styles = css;
                        dom.calls += calls;
                    }
                }
                Ok(())
            })();
            if let Err(error) = result.and_then(|_| request_animation(&inner)) {
                inner.active.set(false);
                *inner.error.borrow_mut() = Some(format!("{error:?}"));
                web_sys::console::error_1(&error);
            }
            scroll::request(&inner);
        }) as Box<dyn FnMut(f64)>);
        *inner.animation_callback.borrow_mut() = Some(callback);
    }
    let callback = inner.animation_callback.borrow();
    let id = web_sys::window().unwrap().request_animation_frame(callback.as_ref().unwrap().as_ref().unchecked_ref())?;
    inner.animation_frame.set(Some(id));
    Ok(())
}

fn schedule(inner: &Rc<Inner>) {
    if !inner.active.get() || inner.scheduled.replace(true) {
        return;
    }

    let weak = Rc::downgrade(inner);
    let callback = Closure::once_into_js(move || {
        if let Some(inner) = weak.upgrade() {
            inner.scheduled.set(false);
            if !inner.active.get() {
                return;
            }

            if let Err(error) = flush(&inner) {
                web_sys::console::error_1(&error);
                *inner.error.borrow_mut() = Some(format!("{error:?}"));
            }
        }
    });

    let callback: &Function = callback.unchecked_ref();
    let window = web_sys::window().expect("browser window");

    // Yield to the browser when follow-up updates keep extending the microtask chain.
    let count = inner.microtasks.get();
    if count >= 16 {
        inner.microtasks.set(0);
        let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(callback, 0);
    } else {
        inner.microtasks.set(count + 1);
        window.queue_microtask(callback);
    }
}

/// Drain admitted work, apply the resulting host patches, then commit core state.
fn flush(inner: &Rc<Inner>) -> Result<(), JsValue> {
    let mut app = inner
        .app
        .try_borrow_mut()
        .map_err(|_| JsValue::from_str("reentrant commit"))?;
    let Some(app) = app.as_mut() else {
        return Ok(());
    };

    if inner.surface_dirty.replace(false) {
        if let Some(surface) = inner.surface.borrow().as_ref() {
            surface.sample(inner, app.runtime_mut())?;
        }
    }

    // Bound each drain so a sustained queue can still make progress through commit.
    for _ in 0..128 {
        let command = inner.queue.borrow_mut().pop_front();
        match command {
            Some(Queued::Mutation(scope, action)) => {
                // The owning view may have unmounted since this work was queued.
                if scope.is_none_or(|scope| scope.is_alive()) {
                    app.mutate(action);
                }
            }
            Some(Queued::Event(id, event)) => {
                app.dispatch(id, event);
            }
            Some(Queued::Focus(target)) => app.observe_focus(target),
            None => break,
        }
    }

    if !inner.queue.borrow().is_empty() {
        schedule(inner);
    }

    app.runtime_mut().drain_messages();
    let pending = app
        .prepare()
        .map_err(|e| JsValue::from_str(&e.to_string()))?;
    let committed = pending.is_some();
    if let Some(pending) = pending {
        inner.dom.borrow().validate(&pending.update)?;

        // Property-only updates keep the existing observer subscriptions unless
        // they change which scroll behavior needs to be tracked.
        let reconnect_scroll = pending.update.patches.iter().any(|patch| match patch {
            Patch::Create(..) | Patch::Remove(..) | Patch::Children(..) => true,
            Patch::Update(id, element) => {
                inner.dom.borrow().entries[id].element.scroll != element.scroll
            }
        });

        // Release the mutable DOM borrow before reconnecting observers.
        let result = inner.dom.borrow_mut().apply(&pending.update);
        let result = result.and_then(|()| {
            events::reconcile_capture(inner);
            if reconnect_scroll {
                if let Some(scroll) = inner.scroll.borrow().as_ref() {
                    scroll.reconnect(&inner.dom.borrow())?;
                }
            }

            scroll::request(inner);
            Ok(())
        });

        match result {
            Ok(()) => app
                .commit(pending)
                .map_err(|e| JsValue::from_str(&e.to_string()))?,
            Err(error) => {
                // Once DOM mutation begins the previous tree is not a safe diff baseline.
                inner.active.set(false);
                inner.listeners.borrow_mut().clear();
                inner.surface.borrow_mut().take();
                inner.surface_driver.close(app.runtime_mut());
                app.dispose();

                let dom = inner.dom.borrow();
                dom.root.set_text_content(Some(
                    "Refract could not update this application. Remount to recover.",
                ));
                return Err(error);
            }
        }
    }

    let request = app
        .take_focus_request()
        .map_err(|e| JsValue::from_str(&e.to_string()))?;
    if let Some(target) = request {
        // Keep the DOM borrow during the native call to suppress reentrant pairs.
        // Confirm the actual result once the call returns.
        inner.dom.borrow_mut().request_focus(target)?;
    }
    if committed || request.is_some() {
        let focused = inner.dom.borrow().focused_id();
        app.confirm_focus(focused);
    }

    if let Some(surface) = inner.surface.borrow().as_ref() {
        surface.apply(inner, app.runtime_mut());
    }
    request_animation(inner)?;
    Ok(())
}

/// Translate browser input into semantic events for the nearest eligible owner.
fn dispatch(inner: &Rc<Inner>, event: &web_sys::Event) {
    if !inner.active.get() {
        return;
    }

    if !inner.scheduled.get() {
        inner.microtasks.set(0);
    }
    if events::dispatch(inner, event) {
        return;
    }
    let event_type = event.type_();

    // Walk the composed path so delegation works inside the shadow root and
    // clicks on child content can reach their owning activation target.
    let path = event.composed_path();
    for target in path.iter() {
        let Ok(node) = target.dyn_into::<HtmlElement>() else {
            continue;
        };

        let Some(id) = node
            .get_attribute("data-refract-id")
            .and_then(|v| v.parse::<u64>().ok())
            .map(ElementId)
        else {
            continue;
        };

        let message = {
            let mut dom = match inner.dom.try_borrow_mut() {
                Ok(dom) => dom,
                Err(_) => return,
            };

            let Some(entry) = dom.entries.get_mut(&id) else {
                continue;
            };

            // IDs alone are not enough: an event may come from another mount
            // or a node that no longer matches this retained entry.
            if entry.node != node || entry.element.disabled {
                continue;
            }

            match event_type.as_str() {
                "click" if entry.element.interaction == Some(Interaction::Activate) => {
                    Some(UiEvent::Press)
                }
                "keydown" if !entry.composing => {
                    event.dyn_ref::<web_sys::KeyboardEvent>().and_then(|key| {
                        if entry.element.interaction == Some(Interaction::Activate)
                            && !key.is_composing()
                            && matches!(key.key().as_str(), "Enter" | " ")
                        {
                            key.prevent_default();
                            return (!key.repeat()).then_some(UiEvent::Press);
                        }

                        // Multiline Enter edits text; Command/Ctrl+Enter submits.
                        let submit = key.key() == "Enter"
                            && !key.is_composing()
                            && entry.element.kind == ElementKind::TextInput
                            && (!entry.element.multiline || key.meta_key() || key.ctrl_key());
                        if submit {
                            key.prevent_default();
                            (!key.repeat()).then_some(UiEvent::Submit)
                        } else {
                            None
                        }
                    })
                }
                "compositionstart" => {
                    entry.composing = true;
                    entry.pending_value = None;
                    None
                }
                "compositionend" => {
                    entry.composing = false;
                    // A model write during composition wins over provisional
                    // browser text; otherwise publish the completed IME edit.
                    let value = entry
                        .pending_value
                        .take()
                        .unwrap_or_else(|| input_value(&node));
                    set_value(&node, &value);
                    Some(UiEvent::Input(value))
                }
                "input" if entry.element.kind == ElementKind::TextInput && !entry.composing => {
                    Some(UiEvent::Input(input_value(&node)))
                }
                _ => None,
            }
        };

        // Dispatch immediately when possible, preserving FIFO order if work is queued.
        if let Some(message) = message {
            if inner.queue.borrow().is_empty() {
                if let Ok(mut app) = inner.app.try_borrow_mut() {
                    if let Some(app) = app.as_mut() {
                        if app.dispatch(id, message) {
                            break;
                        }
                    }
                    continue;
                }
            }

            if inner.queue.borrow().len() < 1024 {
                inner
                    .queue
                    .borrow_mut()
                    .push_back(Queued::Event(id, message));
                schedule(inner);
            } else {
                web_sys::console::error_1(&JsValue::from_str("Refract event queue is full"));
            }

            break;
        }
    }
}

fn input_value(node: &HtmlElement) -> String {
    if let Some(input) = node.dyn_ref::<HtmlInputElement>() {
        input.value()
    } else if let Some(input) = node.dyn_ref::<HtmlTextAreaElement>() {
        input.value()
    } else {
        String::new()
    }
}

/// Preserve selection when applying model text; equal values leave the editor alone.
fn set_value(node: &HtmlElement, value: &str) {
    // Browser selection offsets count UTF-16 code units, not Rust UTF-8 bytes.
    let length = value.encode_utf16().count() as u32;
    if let Some(input) = node.dyn_ref::<HtmlInputElement>() {
        if input.value() != value {
            let selection = (
                input.selection_start().ok().flatten(),
                input.selection_end().ok().flatten(),
            );
            input.set_value(value);
            if let (Some(start), Some(end)) = selection {
                let _ = input.set_selection_range(start.min(length), end.min(length));
            }
        }
    } else if let Some(input) = node.dyn_ref::<HtmlTextAreaElement>() {
        if input.value() != value {
            let selection = (
                input.selection_start().ok().flatten(),
                input.selection_end().ok().flatten(),
            );
            input.set_value(value);
            if let (Some(start), Some(end)) = selection {
                let _ = input.set_selection_range(start.min(length), end.min(length));
            }
        }
    }
}

impl Dom {
    /// Check IDs, properties and ordered element children before taking ownership.
    /// No app effects have run and no snapshot nodes have been changed yet.
    fn adopt(&mut self, update: &Update) -> Result<bool, JsValue> {
        let nodes = self.root.query_selector_all("[data-refract-id]")?;
        let mut available = HashMap::new();
        for index in 0..nodes.length() {
            let Some(node) = nodes
                .item(index)
                .and_then(|n| n.dyn_into::<HtmlElement>().ok())
            else {
                return Ok(false);
            };
            let Some(id) = node
                .get_attribute("data-refract-id")
                .and_then(|id| id.parse::<u64>().ok())
            else {
                return Ok(false);
            };
            if available.insert(ElementId(id), node).is_some() {
                return Ok(false);
            }
        }
        let mut entries = HashMap::new();
        for patch in &update.patches {
            if let Patch::Create(id, element) = patch {
                let Some(node) = available.remove(id) else {
                    return Ok(false);
                };
                if !node
                    .tag_name()
                    .eq_ignore_ascii_case(crate::html::tag(element))
                {
                    return Ok(false);
                }
                let attributes = crate::html::attributes(*id, element);
                if node.get_attribute_names().length() as usize != attributes.len() {
                    return Ok(false);
                }
                for (key, value) in attributes {
                    if node.get_attribute(key).as_deref() != Some(value.as_str()) {
                        return Ok(false);
                    }
                }
                if element.kind == ElementKind::Text
                    && node.text_content().as_deref() != Some(element.text.as_str())
                {
                    return Ok(false);
                }
                if element.kind == ElementKind::TextInput && input_value(&node) != element.value {
                    // V1 has no event replay: a modified form value cannot silently
                    // become a different initial Rust model. Remount consistently.
                    return Ok(false);
                }
                entries.insert(
                    *id,
                    Entry {
                        node,
                        element: element.clone(),
                        styles: crate::style::css(element),
                        composing: false,
                        pending_value: None,
                        image_url: None,
                        scroll_tracker: Default::default(),
                    },
                );
            }
        }
        if !available.is_empty() {
            return Ok(false);
        }
        let children: HashMap<_, _> = update
            .patches
            .iter()
            .filter_map(|patch| {
                if let Patch::Children(parent, children) = patch {
                    Some((*parent, children.as_slice()))
                } else {
                    None
                }
            })
            .collect();
        for (parent, node) in std::iter::once((None, &self.root))
            .chain(entries.iter().map(|(id, entry)| (Some(*id), &entry.node)))
        {
            let expected = children.get(&parent).copied().unwrap_or_default();
            let actual = node.child_nodes();
            let text_leaf = parent.is_some_and(|id| {
                let element = &entries[&id].element;
                element.kind == ElementKind::Text
                    || (element.kind == ElementKind::TextInput && element.multiline)
            });
            if text_leaf && expected.is_empty() {
                // Text must be literal text, not an unexpected element with the same contents.
                if (0..actual.length())
                    .filter_map(|index| actual.item(index))
                    .any(|node| node.node_type() != Node::TEXT_NODE)
                {
                    return Ok(false);
                }
            } else if actual.length() as usize != expected.len()
                || expected.iter().enumerate().any(|(index, id)| {
                    !actual
                        .item(index as u32)
                        .is_some_and(|node| node.is_same_node(Some(entries[id].node.as_ref())))
                })
            {
                return Ok(false);
            }
        }
        self.entries = entries;
        Ok(true)
    }

    /// Reject stale revisions and invalid patch targets before mutating the DOM.
    fn validate(&self, update: &Update) -> Result<(), JsValue> {
        if update.base_revision != self.revision {
            return Err(JsValue::from_str("DOM revision mismatch"));
        }

        // Validate only patch targets. Copying the retained registry here would
        // turn every single-leaf update into O(total mounted elements) work.
        let mut created = HashSet::new();
        for patch in &update.patches {
            match patch {
                Patch::Create(id, _) => {
                    if self.entries.contains_key(id) || !created.insert(*id) {
                        return Err(JsValue::from_str("duplicate DOM identity"));
                    }
                }
                Patch::Update(id, _) | Patch::Remove(id) => {
                    if !self.entries.contains_key(id) && !created.contains(id) {
                        return Err(JsValue::from_str("unknown DOM target"));
                    }
                }
                Patch::Children(parent, list) => {
                    let exists =
                        |id: &ElementId| self.entries.contains_key(id) || created.contains(id);
                    if parent.is_some_and(|id| !exists(&id)) || list.iter().any(|id| !exists(id)) {
                        return Err(JsValue::from_str("unknown DOM child"));
                    }
                }
            }
        }
        Ok(())
    }

    fn owned_id(&self, node: &web_sys::Element) -> Option<ElementId> {
        let id = ElementId(node.get_attribute("data-refract-id")?.parse().ok()?);
        self.entries
            .get(&id)
            .filter(|entry| entry.node.is_same_node(Some(node)))
            .map(|_| id)
    }

    fn focused_id(&self) -> Option<ElementId> {
        self.active_element()
            .as_ref()
            .and_then(|node| self.owned_id(node))
    }

    fn request_focus(&mut self, target: Option<ElementId>) -> Result<(), JsValue> {
        if let Some(id) = target {
            if let Some(entry) = self.entries.get(&id).filter(|e| !e.element.disabled) {
                entry.node.focus()?;
            }
        } else if let Some(id) = self.focused_id() {
            self.entries[&id].node.blur()?;
        }
        Ok(())
    }

    fn active_element(&self) -> Option<web_sys::Element> {
        self.root.get_root_node().dyn_into::<web_sys::ShadowRoot>().ok()
            .and_then(|root| root.active_element())
            .or_else(|| self.document.active_element())
    }

    /// Apply a validated batch while retaining nodes and focus wherever possible.
    fn apply(&mut self, update: &Update) -> Result<(), JsValue> {
        let now = animation_now();
        self.presentation.apply(update, now);
        if std::mem::take(&mut self.adopting) {
            let adopted = self.adopt(update)?;
            self.root.remove_attribute("data-refract-prerender")?;
            if adopted {
                self.hydrated = true;
                self.revision = update.revision;
                return Ok(());
            }
            // Validate the whole snapshot before adopting any of it. A changed
            // initial view gets one clean mount, without duplicate DOM or effects.
            self.root.set_text_content(None);
        }
        // Document.activeElement identifies the host when focus is in a shadow
        // tree, so consult that tree first to find the actual focused control.
        let active = self.active_element();

        let active_id = active.as_ref().and_then(|node| {
            let id = ElementId(node.get_attribute("data-refract-id")?.parse().ok()?);
            self.entries.get(&id).filter(|entry| entry.node.is_same_node(Some(node))).map(|_| id)
        });
        let recover = active_id.filter(|focused| update.patches.iter().any(|patch| match patch {
            Patch::Remove(id) => id == focused,
            Patch::Update(id, element) => id == focused && (element.disabled
                || (self.entries[id].element.is_focusable() && !element.is_focusable())),
            _ => false,
        }));
        let candidates = if let Some(focused) = recover {
            // DOM order is tab order for Refract's zero/-one tab indices.
            let nodes = self.root.query_selector_all("[data-refract-id]")?;
            let order: Vec<_> = (0..nodes.length()).filter_map(|i| {
                let node = nodes.item(i)?.dyn_into::<HtmlElement>().ok()?;
                let id = ElementId(node.get_attribute("data-refract-id")?.parse().ok()?);
                self.entries.get(&id).filter(|entry| entry.node == node).map(|_| id)
            }).collect();
            order.iter().position(|id| *id == focused).map(|index| {
                order[index + 1..].iter().chain(order[..index].iter().rev()).copied().collect::<Vec<_>>()
            }).unwrap_or_default()
        } else {
            Vec::new()
        };

        for patch in &update.patches {
            match patch {
                Patch::Create(id, element) => {
                    let tag = crate::html::tag(element);

                    let node: HtmlElement = self.document.create_element(tag)?.dyn_into()?;
                    node.set_attribute("data-refract-id", &id.0.to_string())?;
                    node.set_class_name("refract-node");
                    let mut entry = Entry {
                        node,
                        element: element.clone(),
                        styles: BTreeMap::new(),
                        composing: false,
                        pending_value: None,
                        image_url: None,
                        scroll_tracker: Default::default(),
                    };
                    self.calls += apply_properties(&mut entry, element, true)?;
                    self.calls += 1;
                    self.entries.insert(*id, entry);
                }
                Patch::Update(id, element) => {
                    let displayed = self.presentation.element(*id, now).unwrap_or_else(|| element.clone());
                    let entry = self.entries.get_mut(id).unwrap();
                    self.calls += apply_properties(entry, &displayed, false)?;
                    entry.element = element.clone();
                }
                Patch::Children(parent, list) => {
                    let parent = parent
                        .and_then(|id| self.entries.get(&id))
                        .map(|e| e.node.clone())
                        .unwrap_or_else(|| self.root.clone());

                    // Keep matching nodes in place and move only nodes out of order.
                    let mut anchor = parent.first_child();
                    for id in list {
                        let child: &Node = self.entries[id].node.as_ref();
                        if anchor.as_ref().is_some_and(|n| n.is_same_node(Some(child))) {
                            anchor = child.next_sibling();
                        } else {
                            parent.insert_before(child, anchor.as_ref())?;
                            self.calls += 1;
                        }
                    }
                    // Children removed or moved elsewhere are handled by subsequent operations.
                }
                Patch::Remove(id) => {
                    if let Some(entry) = self.entries.remove(id) {
                        entry.node.remove();
                        self.calls += 1;
                    }
                }
            }
        }

        if let Some(previous) = recover {
            if let Some(element) = active.as_ref().and_then(|node| node.dyn_ref::<HtmlElement>()) {
                let _ = element.blur();
            }
            let _ = previous;
            if let Some(target) = candidates.into_iter().find(|id| {
                self.entries.get(id).is_some_and(|entry| entry.element.is_focusable())
            }) {
                let node = &self.entries[&target].node;
                node.focus()?;
            }
        } else if let Some(active) = active {
            // Moving a retained node may blur it. Restore only that same node;
            // unrelated updates must never steal focus from another mount.
            if self.root.contains(Some(&active)) && self.active_element().as_ref() != Some(&active) {
                if let Some(element) = active.dyn_ref::<HtmlElement>() {
                    let _ = element.focus();
                }
            }
        }

        self.revision = update.revision;
        Ok(())
    }
}

/// Diff against the last applied description and retain browser-owned editing state.
fn apply_properties(entry: &mut Entry, new: &Element, initial: bool) -> Result<u64, JsValue> {
    let mut calls = 0;
    let old = &entry.element;
    let node = &entry.node;

    // Update the existing text node where possible to preserve its identity.
    if new.kind == ElementKind::Text && (initial || new.text != old.text) {
        if let Some(text) = node
            .first_child()
            .filter(|node| node.node_type() == Node::TEXT_NODE)
        {
            text.set_node_value(Some(&new.text));
        } else {
            node.set_text_content(Some(&new.text));
        }
        calls += 1;
    }

    // Optional attributes must be removed when their semantic value disappears.
    for (key, value, previous) in [
        ("aria-label", new.label.as_deref(), old.label.as_deref()),
        ("role", new.role.as_deref(), old.role.as_deref()),
        (
            "placeholder",
            new.placeholder.as_deref(),
            old.placeholder.as_deref(),
        ),
    ] {
        if initial || value != previous {
            if let Some(value) = value {
                node.set_attribute(key, value)?;
            } else {
                node.remove_attribute(key)?;
            }
            calls += 1;
        }
    }

    if initial || new.source != old.source {
        let image_url = new
            .source
            .as_ref()
            .map(crate::image::ImageUrl::new)
            .transpose()?;
        if let Some(url) = &image_url {
            node.set_attribute("src", url.as_str())?;
        } else {
            node.remove_attribute("src")?;
        }
        // Release the previous Blob URL only after installing the new source.
        entry.image_url = image_url;
        calls += 1;
    }

    if initial || new.level != old.level {
        if let Some(level) = new.level {
            node.set_attribute("aria-level", &level.to_string())?;
        } else {
            node.remove_attribute("aria-level")?;
        }
        calls += 1;
    }

    if new.kind == ElementKind::Image && (initial || new.label != old.label) {
        node.set_attribute("alt", new.label.as_deref().unwrap_or_default())?;
        calls += 1;
    }

    // Native text controls and activatable elements use different disabled semantics.
    if new.kind == ElementKind::TextInput && (initial || new.disabled != old.disabled) {
        if new.disabled {
            node.set_attribute("disabled", "")?;
        } else {
            node.remove_attribute("disabled")?;
        }
        calls += 1;
    }

    if new.interaction == Some(Interaction::Activate) && (initial || new.disabled != old.disabled) {
        node.set_attribute("data-refract-activate", "")?;

        node.set_attribute("aria-disabled", if new.disabled { "true" } else { "false" })?;
        calls += 1;
    }

    if initial
        || new.focusable != old.focusable
        || new.disabled != old.disabled
        || new.focus_target != old.focus_target
    {
        if new.focus_target
            || new.focusable.is_some()
            || new.interaction == Some(Interaction::Activate)
            || new.kind == ElementKind::TextInput
        {
            node.set_tab_index(if new.is_focusable() { 0 } else { -1 });
        } else if old.focus_target || old.focusable.is_some() {
            node.remove_attribute("tabindex")?;
        }
        if new.disabled {
            let _ = node.blur();
        }
        calls += 1;
    }

    if initial || new.checked != old.checked {
        if let Some(checked) = new.checked {
            node.set_attribute("aria-checked", if checked { "true" } else { "false" })?;
            calls += 1;
        } else if old.checked.is_some() {
            node.remove_attribute("aria-checked")?;
            calls += 1;
        }
    }

    if new.kind == ElementKind::TextInput && (initial || new.value != old.value) {
        if entry.composing {
            // Defer model writes until compositionend so the IME can finish editing.
            entry.pending_value = Some(new.value.clone());
        } else if input_value(node) != new.value {
            set_value(node, &new.value);
            calls += 1;
        }
    }

    // Remove obsolete declarations as well as updating changed values, so style
    // resets do not leave properties from the previous description behind.
    let css = crate::style::css(new);
    let style = node.style();
    for key in entry.styles.keys() {
        if !css.contains_key(key) {
            style.remove_property(key)?;
            calls += 1;
        }
    }

    for (key, value) in &css {
        if entry.styles.get(key) != Some(value) {
            style.set_property(key, value)?;
            calls += 1;
        }
    }

    // Base CSS uses these markers for layout rules involving parent and child.
    if initial || new.style.layout != old.style.layout {
        node.set_attribute(
            "data-refract-layout",
            match new.style.layout {
                refract_core::Layout::Overlay => "overlay",
                refract_core::Layout::Row => "row",
                refract_core::Layout::Column => "column",
                _ => "box",
            },
        )?;
        calls += 1;
    }

    for (name, value, previous) in [
        (
            "data-fill-width",
            new.style.fill_width,
            old.style.fill_width,
        ),
        (
            "data-fill-height",
            new.style.fill_height,
            old.style.fill_height,
        ),
    ] {
        if initial || value != previous {
            if value {
                node.set_attribute(name, "")?;
            } else {
                node.remove_attribute(name)?;
            }
            calls += 1;
        }
    }

    // Publish the new diff baseline only after all property writes succeed.
    entry.styles = css;
    entry.element = new.clone();
    Ok(calls)
}

// Scoped defaults complement per-element inline styles, including child-fill
// behavior and interaction states that depend on CSS selectors.
const BASE_STYLE: &str = crate::html::BASE_STYLE;

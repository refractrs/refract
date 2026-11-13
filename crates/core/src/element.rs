//! Semantic host elements, identity, validation, and dispatched events.

use crate::{
    ElementId, Environment, EnvironmentKey, Event, EventResponse, ImageSource, Layout,
    PointerCapture, PointerPhase, Runtime, ScrollOptions, Style, ViewError,
};
use std::rc::Rc;

/// Semantic primitives that a host adapter can create.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ElementKind {
    Box,
    Text,
    TextInput,
    Image,
    Canvas,
}

/// Native interaction semantics, independent of visual primitive kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Interaction {
    Activate,
}

/// Properties that require a new host element when they change.
/// Keep this separate from the semantic kind: one text-input primitive can
/// require different native controls for single-line and multiline editing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ElementIdentity {
    kind: ElementKind,
    interaction: Option<Interaction>,
    multiline: bool,
}

/// A complete host-element description. Adapters diff properties on update.
#[derive(Clone, Debug, PartialEq)]
pub struct Element {
    pub kind: ElementKind,
    pub interaction: Option<Interaction>,
    pub style: Style,
    pub transition: Option<crate::Transition>,
    pub scroll: Option<ScrollOptions>,
    pub text: String,
    pub value: String,
    /// Allow line breaks in a `TextInput`. Defaults to false.
    /// Changing this remounts the host control, resetting native editing state.
    pub multiline: bool,
    pub checked: Option<bool>,
    pub disabled: bool,
    /// Override keyboard tab participation; fields and activatable elements default to true.
    pub focusable: Option<bool>,
    /// Host capability for a bound focus target; does not add a tab stop.
    #[doc(hidden)]
    pub focus_target: bool,
    pub label: Option<String>,
    pub role: Option<String>,
    pub level: Option<u8>,
    pub placeholder: Option<String>,
    pub source: Option<ImageSource>,
    pub canvas: Option<crate::CanvasContent>,
}

impl Element {
    pub(crate) fn new(kind: ElementKind) -> Self {
        Self {
            kind,
            interaction: None,
            style: Style::default(),
            transition: None,
            scroll: None,
            text: String::new(),
            value: String::new(),
            multiline: false,
            checked: None,
            disabled: false,
            focusable: None,
            focus_target: false,
            label: None,
            role: None,
            level: None,
            placeholder: None,
            source: None,
            canvas: None,
        }
    }

    pub fn is_focusable(&self) -> bool {
        !self.disabled
            && self.focusable.unwrap_or(
                self.kind == ElementKind::TextInput
                    || self.interaction == Some(Interaction::Activate),
            )
    }

    pub(crate) fn identity(&self) -> ElementIdentity {
        ElementIdentity {
            kind: self.kind,
            interaction: self.interaction,
            multiline: self.multiline,
        }
    }

    /// Validate portable styles and supported image URL schemes before host mutation.
    pub fn validate(&self) -> Result<(), ViewError> {
        if let Some(transition) = self.transition { transition.validate()?; }
        self.style.validate()?;
        if let Some(canvas) = &self.canvas {
            canvas.validate()?;
        }
        if self.canvas.is_some() != (self.kind == ElementKind::Canvas) {
            return Err(ViewError::new("canvas content requires a canvas element"));
        }
        if self.multiline && self.kind != ElementKind::TextInput {
            return Err(ViewError::new("multiline requires a text input"));
        }
        if self.interaction == Some(Interaction::Activate) && self.kind == ElementKind::TextInput {
            return Err(ViewError::new("activation requires a non-editable element"));
        }
        if self.scroll.is_some() && !matches!(self.style.layout, Layout::Scroll(_)) {
            return Err(ViewError::new("scroll options require a scroll layout"));
        }
        if let Some(url) = self.source.as_ref().and_then(ImageSource::url) {
            let trimmed = url.trim();
            let scheme = trimmed.split(['/', '?', '#']).next().unwrap_or_default();
            if scheme.contains(':')
                && !(trimmed.starts_with("https://")
                    || trimmed.starts_with("http://")
                    || trimmed.starts_with("blob:"))
            {
                return Err(ViewError::new("unsupported URL scheme"));
            }
        }
        Ok(())
    }
}

/// A retained callback invoked with the mounted view's environment.
#[derive(Clone)]
pub struct Handler {
    callback: Rc<dyn Fn(&mut ActionContext<'_>, &Event)>,
    pub(crate) focus: Option<crate::runtime::focus_state::FocusBinding>,
}

impl Handler {
    pub fn new(callback: impl Fn(&mut ActionContext<'_>, &Event) + 'static) -> Self {
        Self {
            callback: Rc::new(callback),
            focus: None,
        }
    }
}

impl std::ops::Deref for Handler {
    type Target = dyn Fn(&mut ActionContext<'_>, &Event);
    fn deref(&self) -> &Self::Target {
        &*self.callback
    }
}

/// Both concrete builders and general view modifiers use the same activation policy.
pub(crate) fn add_press_handler(
    element: &mut Element,
    handler: &mut Option<Handler>,
    action: impl Fn(&mut ActionContext<'_>) + 'static,
) {
    element.interaction = Some(Interaction::Activate);
    element.role.get_or_insert_with(|| "button".into());
    crate::view::modifiers::append_handler(
        handler,
        Handler::new(move |cx, event| {
            if matches!(event, Event::Press) {
                action(cx);
            }
        }),
    );
}

/// Host-provided border-box geometry at the last presented layout. The transform
/// maps local coordinates into the mounting surface; it includes scrolling.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ElementGeometry {
    pub bounds: crate::Rect,
    pub transform: crate::Affine2d,
}

/// Access to graph mutation and inherited values during event dispatch.
pub struct ActionContext<'a> {
    pub(crate) runtime: &'a mut Runtime,
    pub(crate) geometry: Option<ElementGeometry>,
    pub(crate) environment: &'a Environment,
    pub(crate) target: ElementId,
    pub(crate) current_target: ElementId,
    pub(crate) event: &'a Event,
    pub(crate) response: &'a mut EventResponse,
}

impl ActionContext<'_> {
    /// Convert a surface point into the current handler's local border box.
    /// Returns None before layout or for a non-invertible transform.
    pub fn local_position(&self, point: crate::EventPoint) -> Option<crate::Point> {
        let [a, b, c, d, e, f] = self.geometry?.transform.inverse()?.0;
        Some(crate::Point::new(
            a * point.x + c * point.y + e,
            b * point.x + d * point.y + f,
        ))
    }

    pub fn local_bounds(&self) -> Option<crate::Rect> {
        self.geometry.map(|g| g.bounds)
    }

    pub fn target(&self) -> ElementId {
        self.target
    }
    pub fn current_target(&self) -> ElementId {
        self.current_target
    }
    /// Suppress native behavior without stopping other callbacks or bubbling.
    pub fn prevent_default(&mut self) {
        self.response.default_prevented = true;
    }
    /// Stop ancestor callbacks; remaining callbacks on this element still run.
    pub fn stop_propagation(&mut self) {
        self.response.propagation_stopped = true;
    }
    pub fn default_prevented(&self) -> bool {
        self.response.default_prevented
    }
    /// Retain this pointer's events outside the element until release, up, or cancel.
    pub fn capture_pointer(&mut self) {
        if let Event::Pointer(event) = self.event {
            if matches!(event.phase, PointerPhase::Down | PointerPhase::Move) {
                self.response.pointer_capture = Some(PointerCapture {
                    pointer_id: event.id,
                    target: self.current_target,
                    capture: true,
                });
            }
        }
    }
    pub fn release_pointer(&mut self) {
        if let Event::Pointer(event) = self.event {
            self.response.pointer_capture = Some(PointerCapture {
                pointer_id: event.id,
                target: self.current_target,
                capture: false,
            });
        }
    }

    pub fn runtime(&mut self) -> &mut Runtime {
        self.runtime
    }

    pub fn environment<K: EnvironmentKey>(&self) -> K::Value {
        self.environment.read::<K>(self.runtime)
    }
}

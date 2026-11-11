//! Bound text input and its native control configuration.

use crate::{
    view::Described, ActionContext, Binding, Description, Element, ElementKind, Event, Handler,
    Style, View, ViewContext, ViewError,
};

/// A bound text control. These builders set native control properties directly.
pub struct Field {
    binding: Binding<String>,
    handler: Option<Handler>,
    element: Element,
}

impl crate::modifiers::ElementBuilder for Field {
    fn element_mut(&mut self) -> &mut crate::Element {
        &mut self.element
    }
    fn handler_mut(&mut self) -> &mut Option<Handler> {
        &mut self.handler
    }
}

impl Field {
    crate::modifiers::element_modifiers!(pub Self, crate::modifiers::configure);
    crate::modifiers::event_modifiers!(pub Self, crate::modifiers::configure_handler);
    crate::modifiers::focus_modifiers!(pub Self, crate::modifiers::configure_focus);
    crate::modifiers::text_modifiers!(pub Self, crate::modifiers::configure);

    /// Allow multiple lines. Defaults to false.
    ///
    /// Multiline fields insert a newline on Enter and submit on Command/Ctrl+Enter.
    /// Changing modes remounts the host control while retaining the bound value;
    /// focus, selection, undo history, and active composition are reset.
    pub fn multiline(mut self, value: bool) -> Self {
        self.element.multiline = value;
        self
    }

    /// Replace the style of the editable control.
    pub fn style(mut self, style: Style) -> Self {
        self.element.style = style;
        self
    }

    /// Submit without inserting a newline. IME composition never submits.
    pub fn on_submit(mut self, action: impl Fn(&mut ActionContext<'_>) + 'static) -> Self {
        crate::modifiers::append_handler(
            &mut self.handler,
            Handler::new(move |cx, event| {
                if matches!(event, Event::Submit) {
                    action(cx);
                }
            }),
        );
        self
    }

    /// Observe user edits after the binding updates; programmatic writes do not emit input.
    pub fn on_input(mut self, action: impl Fn(&mut ActionContext<'_>, &str) + 'static) -> Self {
        crate::modifiers::append_handler(
            &mut self.handler,
            Handler::new(move |cx, event| {
                if let Event::Input(value) = event {
                    action(cx, value);
                }
            }),
        );
        self
    }

    /// Set the hint displayed when the editable value is empty.
    pub fn placeholder(mut self, value: impl Into<String>) -> Self {
        self.element.placeholder = Some(value.into());
        self
    }
}

impl View for Field {
    fn body(&self, cx: &mut ViewContext<'_>) -> Result<impl View, ViewError> {
        let mut element = self.element.clone();
        // Reading during body evaluation subscribes this control to model changes.
        element.value = cx.read(&self.binding);

        // Native edits travel back through the same binding. Equal values do not
        // trigger another graph invalidation.
        let value = self.binding.clone();
        let callbacks = self.handler.clone();
        let focus = callbacks.as_ref().and_then(|h| h.focus.clone());
        let mut handler = Handler::new(move |cx, e| {
            if let Event::Input(v) = e {
                value.set_if_changed(cx.runtime(), v.clone())
            }
            if let Some(callbacks) = &callbacks {
                callbacks(cx, e);
            }
        });
        handler.focus = focus;
        Ok(Described(Description::Element(
            Box::new(element),
            Vec::new(),
            Some(handler),
        )))
    }
}

/// Edit a bound string. Use `.multiline(true)` to allow line breaks.
pub fn field(value: Binding<String>) -> Field {
    Field {
        binding: value,
        handler: None,
        element: Element::new(ElementKind::TextInput),
    }
}

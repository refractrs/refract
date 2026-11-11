//! Shared modifiers and their identity, style, and environment wrappers.

use crate::{
    environment::Provider, scroll_view, ActionContext, AnyView, Description, Element, ElementKind,
    EnvironmentKey, Insets, ScrollView, View,
};
use std::{any::Any, rc::Rc};

// One modifier definition serves concrete builders and the generic ViewExt fallback.
macro_rules! element_modifiers {
    ($visibility:vis $output:ty, $apply:path) => {
        $visibility fn width(self, value: f32) -> $output {
            $apply(self, move |e| e.style.width = Some(value))
        }

        $visibility fn height(self, value: f32) -> $output {
            $apply(self, move |e| e.style.height = Some(value))
        }

        $visibility fn min_width(self, value: f32) -> $output {
            $apply(self, move |e| e.style.min_width = Some(value))
        }

        $visibility fn max_width(self, value: f32) -> $output {
            $apply(self, move |e| e.style.max_width = Some(value))
        }

        $visibility fn min_height(self, value: f32) -> $output {
            $apply(self, move |e| e.style.min_height = Some(value))
        }

        $visibility fn max_height(self, value: f32) -> $output {
            $apply(self, move |e| e.style.max_height = Some(value))
        }

        $visibility fn fill_width(self) -> $output {
            $apply(self, move |e| e.style.fill_width = true)
        }

        $visibility fn fill_height(self) -> $output {
            $apply(self, move |e| e.style.fill_height = true)
        }

        $visibility fn grow(self, value: f32) -> $output {
            $apply(self, move |e| e.style.grow = value)
        }

        $visibility fn background(self, color: $crate::Color) -> $output {
            $apply(self, move |e| e.style.background = Some(color))
        }

        /// Layer a linear or radial gradient over the optional solid background color.
        $visibility fn background_gradient(self, gradient: impl Into<$crate::Gradient>) -> $output {
            let gradient = gradient.into();
            $apply(self, move |e| {
                e.style.background_gradient = Some(gradient.clone())
            })
        }

        $visibility fn foreground(self, color: $crate::Color) -> $output {
            $apply(self, move |e| e.style.foreground = Some(color))
        }

        /// Cast an outer rounded-box shadow without changing layout or hit testing.
        $visibility fn shadow(self, shadow: $crate::BoxShadow) -> $output {
            $apply(self, move |e| e.style.shadow = Some(shadow))
        }

        $visibility fn border(self, value: $crate::Border) -> $output {
            $apply(self, move |e| e.style.border = Some(value))
        }

        $visibility fn radius(self, value: impl Into<$crate::CornerRadii>) -> $output {
            let value = value.into();
            $apply(self, move |e| e.style.radius = value)
        }

        $visibility fn opacity(self, value: f32) -> $output {
            $apply(self, move |e| e.style.opacity = Some(value))
        }

        /// Blur this element and its descendants as one image. The amount is
        /// Gaussian standard deviation in logical pixels; zero disables the effect.
        /// Does not change layout or hit testing. Ancestor clips constrain the result.
        $visibility fn blur(self, amount: f32) -> $output {
            $apply(self, move |e| e.style.blur = Some(amount))
        }

        /// Blur previously painted content behind this element, inside its rounded
        /// border box. Its own background and descendants remain sharp. Use a
        /// translucent background to reveal the effect. Zero disables the effect.
        /// The amount is Gaussian standard deviation in logical pixels.
        $visibility fn backdrop_blur(self, amount: f32) -> $output {
            $apply(self, move |e| e.style.backdrop_blur = Some(amount))
        }

        $visibility fn clipped(self) -> $output {
            $apply(self, move |e| e.style.clipped = true)
        }

        /// Disable events and keyboard focus on the current outer element.
        /// Apply alongside `on_press`, after any structural modifiers defining its target.
        $visibility fn disabled(self, value: bool) -> $output {
            $apply(self, move |e| e.disabled = value)
        }

        /// Customize the keyboard ring and its quieter pointer-editing variant.
        /// The indicator follows this element's corner radius without changing layout.
        $visibility fn focus_ring(self, ring: $crate::FocusRing) -> $output {
            $apply(self, move |e| e.style.focus_ring = Some(ring))
        }

        /// Highlight this element while it or a descendant has focus.
        /// Use `focus_ring(FocusRing::none())` on an inner field to delegate its cue.
        $visibility fn focus_within(self, ring: $crate::FocusRing) -> $output {
            $apply(self, move |e| e.style.focus_within = Some(ring))
        }

        /// Include/exclude this element in keyboard tab navigation.
        $visibility fn focusable(self, value: bool) -> $output {
            $apply(self, move |e| e.focusable = Some(value))
        }

        $visibility fn label(self, value: impl Into<String>) -> $output {
            let value = value.into();
            $apply(self, move |e| e.label = Some(value.clone()))
        }
    };
}
pub(crate) use element_modifiers;

macro_rules! event_modifiers {
    ($visibility:vis $output:ty, $apply:path) => {
        $visibility fn on_key(self, action: impl Fn(&mut $crate::ActionContext<'_>, &$crate::KeyEvent) + 'static) -> $output {
            $apply(self, $crate::Handler::new(move |cx, event| {
                if let $crate::Event::Key(event) = event { action(cx, event); }
            }))
        }
        $visibility fn on_pointer(self, action: impl Fn(&mut $crate::ActionContext<'_>, &$crate::PointerEvent) + 'static) -> $output {
            $apply(self, $crate::Handler::new(move |cx, event| {
                if let $crate::Event::Pointer(event) = event { action(cx, event); }
            }))
        }
        $visibility fn on_hover(self, action: impl Fn(&mut $crate::ActionContext<'_>, bool) + 'static) -> $output {
            $apply(self, $crate::Handler::new(move |cx, event| {
                if let $crate::Event::Hover(inside) = event { action(cx, *inside); }
            }))
        }
        /// Observe confirmed focus entering or leaving this entire subtree.
        $visibility fn on_focus_within_change(self, action: impl Fn(&mut $crate::ActionContext<'_>, bool) + 'static) -> $output {
            $apply(self, $crate::Handler::new(move |cx, event| {
                if let $crate::Event::FocusWithin(focused) = event { action(cx, *focused); }
            }))
        }
        $visibility fn on_focus_change(self, action: impl Fn(&mut $crate::ActionContext<'_>, bool) + 'static) -> $output {
            $apply(self, $crate::Handler::new(move |cx, event| {
                if let $crate::Event::Focus(focused) = event { action(cx, *focused); }
            }))
        }
    };
}
pub(crate) use event_modifiers;

macro_rules! focus_modifiers {
    ($visibility:vis $output:ty, $apply:path) => {
        /// Bind direct focus. Writes request focus after commit; host transitions
        /// update the binding. This does not add the element to the tab order.
        $visibility fn focused(self, binding: $crate::Binding<bool>) -> $output {
            $apply(self, $crate::focus::FocusBinding::boolean(binding))
        }
        /// Associate this element with one value in a shared focus selection.
        $visibility fn focused_value<T: Clone + PartialEq + 'static>(self, binding: $crate::Binding<Option<T>>, value: T) -> $output {
            $apply(self, $crate::focus::FocusBinding::value(binding, value))
        }
    };
}
pub(crate) use focus_modifiers;

pub(crate) fn configure_focus<T: ElementBuilder>(
    mut view: T,
    focus: crate::focus::FocusBinding,
) -> T {
    view.element_mut().focus_target = true;
    view.handler_mut()
        .get_or_insert_with(|| crate::Handler::new(|_, _| {}))
        .focus = Some(focus);
    view
}

fn modify_focus(view: impl View, focus: crate::focus::FocusBinding) -> Modified {
    let (mut element, children, mut handler) = match view.describe() {
        Some(Description::Element(element, children, handler)) => (element, children, handler),
        _ => (
            Box::new(Element::new(ElementKind::Box)),
            vec![view.into_any()],
            None,
        ),
    };
    element.focus_target = true;
    handler
        .get_or_insert_with(|| crate::Handler::new(|_, _| {}))
        .focus = Some(focus);
    Modified(Description::Element(element, children, handler))
}

pub(crate) fn append_handler(slot: &mut Option<crate::Handler>, action: crate::Handler) {
    let previous = slot.take();
    let focus = action
        .focus
        .clone()
        .or_else(|| previous.as_ref().and_then(|h| h.focus.clone()));
    let mut combined = crate::Handler::new(move |cx, event| {
        if let Some(previous) = &previous {
            previous(cx, event);
        }
        action(cx, event);
    });
    combined.focus = focus;
    *slot = Some(combined);
}

pub(crate) fn configure_handler<T: ElementBuilder>(mut view: T, action: crate::Handler) -> T {
    append_handler(view.handler_mut(), action);
    view
}

fn modify_handler(view: impl View, action: crate::Handler) -> Modified {
    if let Some(Description::Element(element, children, mut handler)) = view.describe() {
        append_handler(&mut handler, action);
        Modified(Description::Element(element, children, handler))
    } else {
        Modified(Description::Element(
            Box::new(Element::new(ElementKind::Box)),
            vec![view.into_any()],
            Some(action),
        ))
    }
}

// Typography belongs to text-bearing builders, not arbitrary views.
macro_rules! text_modifiers {
    ($visibility:vis $output:ty, $apply:path) => {
        $visibility fn font_size(self, value: f32) -> $output {
            $apply(self, move |e| e.style.font_size = Some(value))
        }

        $visibility fn font_weight(self, value: u16) -> $output {
            $apply(self, move |e| e.style.font_weight = Some(value))
        }

        /// Request an italic face, or a synthesized slant when unavailable.
        $visibility fn italic(self) -> $output {
            $apply(self, move |e| e.style.italic = Some(true))
        }

        /// Add spacing between letters in logical pixels. Negative values tighten text.
        $visibility fn letter_spacing(self, value: f32) -> $output {
            $apply(self, move |e| e.style.letter_spacing = Some(value))
        }

        /// Add spacing between words in logical pixels. Negative values tighten text.
        $visibility fn word_spacing(self, value: f32) -> $output {
            $apply(self, move |e| e.style.word_spacing = Some(value))
        }

        $visibility fn font_family(self, value: impl Into<String>) -> $output {
            let value = value.into();
            $apply(self, move |e| e.style.font_family = Some(value.clone()))
        }

        $visibility fn line_height(self, value: f32) -> $output {
            $apply(self, move |e| e.style.line_height = Some(value))
        }

        $visibility fn text_align(self, value: $crate::Alignment) -> $output {
            $apply(self, move |e| e.style.text_align = Some(value))
        }

    };
}
pub(crate) use text_modifiers;

pub(crate) trait ElementBuilder {
    fn element_mut(&mut self) -> &mut Element;
    fn handler_mut(&mut self) -> &mut Option<crate::Handler>;
}

pub(crate) fn configure<V: ElementBuilder>(mut view: V, f: impl FnOnce(&mut Element)) -> V {
    f(view.element_mut());
    view
}

fn modify_outer<V: View>(view: V, f: impl Fn(&mut Element) + 'static) -> Modified {
    view.modify(false, f)
}

// Key wrappers own an erased child to avoid cloning arbitrary user views.
/// A view with an explicit sibling-local key for preserving mounted identity.
pub struct KeyView {
    view: AnyView,
    key: String,
}

impl View for KeyView {
    fn metadata(&self) -> crate::ViewMetadata {
        crate::ViewMetadata::default()
    }

    fn describe(&self) -> Option<Description> {
        Some(Description::Group(vec![self.view.clone()]))
    }

    fn view_key(&self) -> Option<String> {
        Some(self.key.clone())
    }
}

/// A view description produced by applying a style or interaction modifier.
#[derive(Clone)]
pub struct Modified(Description);

impl View for Modified {
    fn metadata(&self) -> crate::ViewMetadata {
        self.0.metadata()
    }

    fn describe(&self) -> Option<Description> {
        Some(self.0.clone())
    }
}

/// A view that provides an inherited environment value to its subtree.
pub struct Provided {
    view: AnyView,
    provider: Provider,
}

impl View for Provided {
    fn metadata(&self) -> crate::ViewMetadata {
        crate::ViewMetadata::default()
    }

    fn describe(&self) -> Option<Description> {
        Some(Description::Provider(
            self.provider.clone(),
            self.view.clone(),
        ))
    }
}

#[doc(hidden)]
type MemoEquality = dyn Fn(&dyn Any) -> bool;

/// Stored memoization props and their equality rule for deciding subtree reuse.
#[derive(Clone)]
pub struct Memo {
    value: Rc<dyn Any>,
    equal: Rc<MemoEquality>,
}

impl Memo {
    pub(crate) fn same(&self, other: &Self) -> bool {
        (self.equal)(other.value.as_ref())
    }
}

/// A view that can reuse its subtree when explicit props and environment scope match.
pub struct MemoView {
    view: AnyView,
    props: Memo,
}

impl View for MemoView {
    fn metadata(&self) -> crate::ViewMetadata {
        crate::ViewMetadata::memo(self.props.clone())
    }

    fn describe(&self) -> Option<Description> {
        Some(Description::Memo(self.props.clone(), self.view.clone()))
    }
}

/// Composition, interaction, identity, provider, and style modifiers on every view.
/// Concrete element builders also expose inherent modifiers that preserve their types. This trait supplies the fallback for other views.
pub trait ViewExt: View + Sized {
    element_modifiers!(Modified, modify_outer);
    event_modifiers!(Modified, modify_handler);
    focus_modifiers!(Modified, modify_focus);

    /// Add an action for pointer or keyboard activation to the current element.
    /// Text, images, and boxes are activated directly, without changing their kind.
    /// Opaque views receive an outer activation box. Nested interactive controls
    /// are rejected during preparation; this is not an editable-control event hook.
    /// Existing roles are preserved; otherwise the target gets button semantics.
    /// Browser activation targets suppress user text selection, including their
    /// labels, so repeated clicks do not select the control's text.
    ///
    /// Callbacks on the same target run in registration order, in one mutation
    /// batch; other event handlers (for example scroll observers) are preserved.
    /// Structural modifier order defines the hit area: `padding(...).on_press(...)`
    /// includes the padding, while `on_press(...).padding(...)` does not.
    fn on_press(self, action: impl Fn(&mut ActionContext<'_>) + 'static) -> Modified {
        if let Some(Description::Element(mut element, children, mut handler)) = self.describe() {
            crate::element::add_press_handler(&mut element, &mut handler, action);
            return Modified(Description::Element(element, children, handler));
        }
        let mut element = Element::new(ElementKind::Box);
        let mut handler = None;
        crate::element::add_press_handler(&mut element, &mut handler, action);
        Modified(Description::Element(
            Box::new(element),
            vec![self.into_any()],
            handler,
        ))
    }

    /// Reuse a subtree when explicit props and provider scope match.
    /// Reactive reads still invalidate it; include changing nonreactive inputs in props.
    fn memo<T: PartialEq + 'static>(self, props: T) -> MemoView {
        let props = Rc::new(props);
        let compare = props.clone();
        MemoView {
            view: self.into_any(),
            props: Memo {
                value: props,
                equal: Rc::new(move |other| {
                    other
                        .downcast_ref::<T>()
                        .is_some_and(|other| other == compare.as_ref())
                }),
            },
        }
    }

    /// Give this child a sibling-local identity independent of its list position.
    fn key(self, key: impl ToString) -> KeyView {
        KeyView {
            view: self.into_any(),
            key: key.to_string(),
        }
    }

    /// Modify a primitive directly when possible, or introduce a box wrapper.
    /// `wrap` forces a new box, making modifier order significant.
    fn modify(self, wrap: bool, f: impl Fn(&mut Element) + 'static) -> Modified {
        if !wrap {
            if let Some(Description::Element(mut element, children, handler)) = self.describe() {
                f(&mut element);
                return Modified(Description::Element(element, children, handler));
            }
        }
        let mut element = Element::new(ElementKind::Box);
        f(&mut element);
        Modified(Description::Element(
            Box::new(element),
            vec![self.into_any()],
            None,
        ))
    }

    /// Add a padding wrapper around this view.
    fn padding(self, padding: impl Into<Insets>) -> Modified {
        let padding = padding.into();
        self.modify(true, move |e| e.style.padding = padding)
    }

    /// Add a wrapper with fixed width and height in logical pixels.
    fn frame(self, width: f32, height: f32) -> Modified {
        self.modify(true, move |e| {
            e.style.width = Some(width);
            e.style.height = Some(height)
        })
    }

    /// Wrap this content in a vertical scroll view. Use `.axis(...)` to change its direction.
    fn scrollable(self) -> ScrollView {
        scroll_view(self)
    }

    /// Provide a shared model by identity; descendants read it with ViewContext::object.
    fn environment_object<T: 'static>(self, value: Rc<T>) -> Provided {
        self.environment::<crate::environment::ObjectKey<T>>(crate::environment::ObjectValue(Some(
            value,
        )))
    }

    /// Override a typed environment value for this subtree.
    fn environment<K: EnvironmentKey>(self, value: K::Value) -> Provided {
        Provided {
            view: self.into_any(),
            provider: Provider::new::<K>(value),
        }
    }
}

impl<V: View> ViewExt for V {}

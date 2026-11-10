//! Typed builders for primitive elements and layout containers.

use crate::{
    ActionContext, Alignment, AnyView, Description, Element, ElementKind, Empty, Handler, ImageFit,
    ImageSource, Layout, View,
};

macro_rules! element_builder {
    ($name:ident) => {
        impl crate::modifiers::ElementBuilder for $name {
            fn element_mut(&mut self) -> &mut Element {
                &mut self.element
            }
            fn handler_mut(&mut self) -> &mut Option<Handler> { &mut self.handler }
        }

        impl $name {
            crate::modifiers::element_modifiers!(pub Self, crate::modifiers::configure);
            crate::modifiers::event_modifiers!(pub Self, crate::modifiers::configure_handler);

            /// Add pointer/keyboard activation, preserving this builder.
            /// Browser activation targets suppress user text selection.
            pub fn on_press(mut self, action: impl Fn(&mut ActionContext<'_>) + 'static) -> Self {
                crate::element::add_press_handler(&mut self.element, &mut self.handler, action);
                self
            }
        }
    };
}

macro_rules! leaf_builder {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone)]
        pub struct $name {
            element: Element,
            handler: Option<Handler>,
        }

        impl $name {
            fn new(element: Element) -> Self {
                Self { element, handler: None }
            }
        }

        impl View for $name {
            fn describe(&self) -> Option<Description> {
                Some(Description::Element(
                    Box::new(self.element.clone()),
                    Vec::new(),
                    self.handler.clone(),
                ))
            }
        }

        element_builder!($name);
    };
}

macro_rules! container_builder {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone)]
        pub struct $name {
            element: Element,
            children: Vec<AnyView>,
            handler: Option<Handler>,
        }

        impl $name {
            fn new(element: Element) -> Self {
                Self { element, children: Vec::new(), handler: None }
            }

            /// Append a child view after the existing children.
            pub fn child(mut self, child: impl View) -> Self {
                self.children.push(child.into_any());
                self
            }

            /// Append child views in iteration order.
            pub fn children<V: View>(mut self, children: impl IntoIterator<Item = V>) -> Self {
                self.children.extend(children.into_iter().map(View::into_any));
                self
            }

            /// Preserve a child position when absent, so later unkeyed siblings do not shift.
            pub fn maybe_child<V: View>(mut self, child: Option<V>) -> Self {
                self.children.push(child.map(View::into_any).unwrap_or_else(|| Empty.into_any()));
                self
            }
        }

        impl View for $name {
            fn describe(&self) -> Option<Description> {
                Some(Description::Element(
                    Box::new(self.element.clone()),
                    self.children.clone(),
                    self.handler.clone(),
                ))
            }
        }

        element_builder!($name);
    };
}

leaf_builder!(
    /// Displayed text with typography and heading/status semantics.
    ///
    /// ```compile_fail
    /// use refract_core::prelude::*;
    /// text("Title").child(image("cover.png"));
    /// ```
    ///
    /// ```compile_fail
    /// use refract_core::prelude::*;
    /// text("Title").fit(ImageFit::Cover);
    /// ```
    Text
);
leaf_builder!(
    /// An image with sizing, fit, labeling, and activation.
    ///
    /// ```compile_fail
    /// use refract_core::prelude::*;
    /// image("cover.png").width(200.0).alignment(Alignment::Start);
    /// ```
    ///
    /// ```compile_fail
    /// use refract_core::prelude::*;
    /// image("cover.png").child(text("Caption"));
    /// ```
    ///
    /// ```compile_fail
    /// use refract_core::prelude::*;
    /// image("cover.png").font_size(16.0);
    /// ```
    Image
);
container_builder!(
    /// A general layout box containing arbitrary child views.
    Container
);
container_builder!(
    /// A row or column with child spacing and alignment.
    Stack
);
container_builder!(
    /// Children layered in declaration order; later children appear in front.
    ///
    /// ```compile_fail
    /// use refract_core::prelude::*;
    /// z_stack().gap(12.0);
    /// ```
    Overlay
);
leaf_builder!(
    /// Flexible empty space, with a default growth factor of one.
    Spacer
);

impl Text {
    crate::modifiers::text_modifiers!(pub Self, crate::modifiers::configure);

    /// Display a single line, truncating overflow with an ellipsis on both hosts.
    /// Constrain the text's width to enable truncation. Whitespace is collapsed;
    /// the underlying text and accessible label are retained in full.
    pub fn truncate(mut self) -> Self {
        self.element.style.truncate = true;
        self
    }

    /// Give the text the semantic role of a status message.
    pub fn status(mut self) -> Self {
        self.element.role = Some("status".into());
        self
    }

    /// Give the text a heading role, clamping its level to the range 1–6.
    pub fn heading(mut self, level: u8) -> Self {
        self.element.role = Some("heading".into());
        self.element.level = Some(level.clamp(1, 6));
        self
    }
}

impl Image {
    /// Fit the image pixels inside its content box.
    pub fn fit(mut self, fit: ImageFit) -> Self {
        self.element.style.image_fit = fit;
        self
    }
}

/// The direction in which a stack arranges its children.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Direction {
    /// Arrange children in a row.
    Horizontal,
    /// Arrange children in a column.
    #[default]
    Vertical,
}

impl Stack {
    /// Arrange children horizontally or vertically. Defaults to vertical.
    pub fn direction(mut self, direction: Direction) -> Self {
        self.element.style.layout = match direction {
            Direction::Horizontal => Layout::Row,
            Direction::Vertical => Layout::Column,
        };
        self
    }

    /// Set the space between children in logical pixels; must be nonnegative.
    pub fn gap(mut self, gap: f32) -> Self {
        self.element.style.gap = gap;
        self
    }

    /// Align children across the stack's direction of layout. Defaults to stretch.
    pub fn alignment(mut self, align: Alignment) -> Self {
        self.element.style.align = align;
        self
    }

    /// Distribute children along the stack's direction of layout.
    pub fn justify(mut self, align: Alignment) -> Self {
        self.element.style.justify = align;
        self
    }
}

impl Overlay {
    /// Align each child horizontally and vertically within the shared cell.
    /// Both axes default to stretch; explicit child sizes are preserved.
    /// `SpaceBetween` behaves like `Start` because children overlap rather than
    /// being distributed. Alignment does not change declaration or paint order.
    ///
    /// ```
    /// use refract_core::prelude::*;
    /// let card = z_stack()
    ///     .alignment(Alignment::End, Alignment::Start)
    ///     .child(view().width(240.0).height(160.0).background(Color::BLUE))
    ///     .child(text("New").padding(8.0).background(Color::WHITE));
    /// ```
    pub fn alignment(mut self, horizontal: Alignment, vertical: Alignment) -> Self {
        self.element.style.justify = horizontal;
        self.element.style.align = vertical;
        self
    }
}

/// Create a text view with configurable typography and heading or status semantics.
/// The view owns its text; strings are displayed as plain text, not parsed as markup.
pub fn text(value: impl Into<String>) -> Text {
    let mut element = Element::new(ElementKind::Text);
    element.text = value.into();
    Text::new(element)
}

/// Create a vertical stack. Use `.direction(...)` to change its orientation.
pub fn stack() -> Stack {
    let mut element = Element::new(ElementKind::Box);
    element.style.layout = Layout::Column;
    element.style.align = Alignment::Stretch;
    Stack::new(element)
}

/// Shorthand for `stack().direction(Direction::Horizontal)`.
pub fn h_stack() -> Stack {
    stack().direction(Direction::Horizontal)
}

/// Shorthand for `stack()`.
pub fn v_stack() -> Stack {
    stack()
}

/// Create an overlay stack; later children appear in front of earlier children.
/// Children share one cell and stretch by default. Use [`Overlay::alignment`]
/// to position smaller children independently along the horizontal and vertical axes.
pub fn z_stack() -> Overlay {
    let mut element = Element::new(ElementKind::Box);
    element.style.layout = Layout::Overlay;
    element.style.align = Alignment::Stretch;
    element.style.justify = Alignment::Stretch;
    Overlay::new(element)
}

/// Create a basic layout box for styling, semantics, or interaction.
pub fn view() -> Container {
    Container::new(Element::new(ElementKind::Box))
}

/// Create flexible empty space that grows to fill available room in a stack.
/// The default growth factor is one; use `.grow(...)` to change it.
pub fn spacer() -> Spacer {
    Spacer::new(Element::new(ElementKind::Box)).grow(1.0)
}

/// Create an image with optional alternative text supplied through [`Image::label`].
/// Without a label, the browser emits an empty `alt` attribute.
pub fn image(source: impl Into<ImageSource>) -> Image {
    let mut element = Element::new(ElementKind::Image);
    element.source = Some(source.into());
    Image::new(element)
}

//! Portable Refract authoring, mounted state, and semantic host updates.
//!
//! Views describe composition and read graph-backed state through [`ViewContext`].
//! [`App`] retains mounted identities, reconciles affected views, and prepares
//! semantic [`Patch`] values for a host adapter.
//!
//! An adapter applies a [`Pending`] update before committing it to the app.
//! Commit publishes subscriptions and runs effect setup and cleanup. Layout,
//! native input behavior, and rendering remain the adapter's responsibility.

mod element;
mod events;
mod environment;
mod field;
mod image;
mod modifiers;
mod primitives;
mod runtime;
mod scroll;
mod style;
mod view;

pub use element::{ActionContext, Element, ElementKind, Handler, Interaction};
pub use events::*;
pub use environment::{Environment, EnvironmentKey};
pub use field::{field, Field};
pub use image::{ImageFit, ImageFormat, ImageSource};
pub use modifiers::{KeyView, Memo, MemoView, Modified, Provided, ViewExt};
pub use primitives::{
    h_stack, image, spacer, stack, text, v_stack, view, z_stack, Container, Image, Overlay, Spacer,
    Stack, Direction, Text,
};
pub use refract_graph::{
    AttributeId, Binding, Dependencies, Derived, GraphStats, Observer, Runtime, State,
};
pub use runtime::*;
pub use scroll::*;
pub use style::*;
pub use view::*;

/// An authoring, validation, or reconciliation error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewError(pub String);

impl ViewError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl std::fmt::Display for ViewError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ViewError {}

/// Common types and constructors for writing views.
pub mod prelude {
    pub use crate::primitives::{image, view};
    pub use crate::events::{EventPoint, Key, KeyEvent, KeyState, Modifiers, PointerButton, PointerButtons, PointerEvent, PointerKind, PointerPhase};
    pub use crate::{
        field, fragment, h_stack, scroll_view, spacer, stack, text, v_stack, view_fn, z_stack,
        ActionContext, Alignment, AnyView, Axis, Binding, Border, BorderSide, BoxShadow, Color,
        Container, CornerRadii, Derived, Empty, EnvironmentKey, Field, GradientStop, Image,
        ImageFit, ImageFormat, ImageSource,
        Insets, LinearGradient, Overlay, Runtime, ScrollEvent, ScrollFollow, ScrollMetrics,
        ScrollOffset, ScrollPosition, ScrollSize, ScrollView, Spacer, Stack, Direction, State,
        Text, View, ViewContext, ViewError, ViewExt,
    };
}

//! Portable Refract authoring, mounted state, and semantic host updates.
//!
//! Views describe composition and read graph-backed state through [`ViewContext`].
//! [`App`] retains mounted identities, reconciles affected views, and prepares
//! semantic [`Patch`] values for a host adapter.
//!
//! An adapter applies a [`Pending`] update before committing it to the app.
//! Commit publishes subscriptions and runs effect setup and cleanup. Layout,
//! native input behavior, and rendering remain the adapter's responsibility.

pub mod drawing;
mod element;
mod events;
mod image;
mod presentation;
mod runtime;
mod style;
mod view;

pub use drawing::*;
pub use element::{ActionContext, Element, ElementGeometry, ElementKind, Handler, Interaction};
pub use events::*;
pub use image::{ImageFit, ImageFormat, ImageSource};
pub use presentation::*;
pub use refract_graph::{
    AttributeId, Binding, Dependencies, Derived, GraphStats, MessageSender, MessageSubscription,
    Observer, Read, Runtime, SendError, State,
};
pub use runtime::*;
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
    pub use crate::drawing::*;
    pub use crate::events::{
        EventPoint, Key, KeyEvent, KeyState, Modifiers, PointerButton, PointerButtons,
        PointerEvent, PointerKind, PointerPhase,
    };
    pub use crate::view::{image, view};
    pub use crate::{
        field, fragment, h_stack, scroll_view, spacer, stack, text, v_stack, view_fn, z_stack,
        ActionContext, Alignment, AnyView, Axis, Binding, Border, BorderSide, BoxShadow, Color,
        Container, CornerRadii, Derived, Direction, Empty, EnvironmentKey, Field, FocusRing,
        Gradient, GradientStop, HistoryMode, Image, ImageFit, ImageFormat, ImageSource, Insets,
        LinearGradient, MemoryHistory, MessageSender, MessageSubscription, Overlay, RadialGradient,
        Read, Request, RequestStatus, Runtime, ScrollEvent, ScrollFollow, ScrollMetrics,
        ScrollOffset, ScrollPosition, ScrollSize, ScrollView, SendError, Spacer, Stack, State,
        Surface, SurfaceCapabilities, SurfaceCommand, SurfaceDriver, SurfaceError, SurfaceRequest,
        Text, Url, View, ViewContext, ViewError, ViewExt, Viewport,
    };
    pub use crate::{
        with_animation, with_transaction, Animation, Transaction, Transform, Transition, UnitPoint,
    };
}

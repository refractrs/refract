//! Portable Refract authoring, mounted state, and semantic host updates.
//!
//! Views describe composition and read graph-backed state through [`ViewContext`].
//! [`App`] retains mounted identities, reconciles affected views, and prepares
//! semantic [`Patch`] values for a host adapter.
//!
//! An adapter applies a [`Pending`] update before committing it to the app.
//! Commit publishes subscriptions and runs effect setup and cleanup. Layout,
//! native input behavior, and rendering remain the adapter's responsibility.

mod animation;
pub use animation::*;
mod transform;
pub use transform::*;
mod transaction;
pub use transaction::{with_animation, with_transaction, AnimationScope, Transaction, TransactionView};
mod element;
mod environment;
mod events;
mod field;
mod focus;
mod image;
mod modifiers;
mod primitives;
mod runtime;
mod scroll;
mod style;
mod surface;
mod view;

pub use element::{ActionContext, Element, ElementKind, Handler, Interaction};
pub use environment::{Environment, EnvironmentKey};
pub use events::*;
pub use field::{field, Field};
pub use image::{ImageFit, ImageFormat, ImageSource};
pub use modifiers::{KeyView, Memo, MemoView, Modified, Provided, ViewExt};
pub use primitives::{
    h_stack, image, spacer, stack, text, v_stack, view, z_stack, Container, Direction, Image,
    Overlay, Spacer, Stack, Text,
};
pub use refract_graph::{
    AttributeId, Binding, Dependencies, Derived, GraphStats, MessageSender, MessageSubscription,
    Observer, Read, Runtime, SendError, State,
};
pub use runtime::*;
pub use scroll::*;
pub use style::*;
pub use surface::*;
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
    pub use crate::{with_animation, with_transaction, Animation, Transaction, Transform, UnitPoint};
    pub use crate::events::{
        EventPoint, Key, KeyEvent, KeyState, Modifiers, PointerButton, PointerButtons,
        PointerEvent, PointerKind, PointerPhase,
    };
    pub use crate::primitives::{image, view};
    pub use crate::{
        field, fragment, h_stack, scroll_view, spacer, stack, text, v_stack, view_fn, z_stack,
        ActionContext, Alignment, AnyView, Axis, Binding, Border, BorderSide, BoxShadow, Color,
        Container, CornerRadii, Derived, Direction, Empty, EnvironmentKey, Field, FocusRing, Gradient,
        GradientStop, HistoryMode, Image, ImageFit, ImageFormat, ImageSource, Insets,
        LinearGradient, MemoryHistory, MessageSender, MessageSubscription, Overlay, RadialGradient,
        Read, Request, RequestStatus, Runtime, ScrollEvent, ScrollFollow, ScrollMetrics,
        ScrollOffset, ScrollPosition, ScrollSize, ScrollView, SendError, Spacer, Stack, State,
        Surface, SurfaceCapabilities, SurfaceCommand, SurfaceDriver, SurfaceError, SurfaceRequest,
        Text, Url, View, ViewContext, ViewError, ViewExt, Viewport,
    };
}

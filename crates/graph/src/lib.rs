//! Single-threaded typed attributes, dynamic dependencies, and owned subscriptions.
//!
//! The graph has three kinds of attributes:
//! - [`State`] stores values written by application code.
//! - [`Derived`] caches computations and re-evaluates them during propagation.
//! - [`Observer`] records dependencies for work evaluated outside the graph, such
//!   as a UI view. Propagation marks observers dirty without running that work.
//!
//! Reads inside [`Runtime::capture`] collect dependencies. Derived computations
//! publish those dependencies immediately; observers publish them separately with
//! [`Runtime::accept`], so speculative UI work can be discarded safely.
//!
//! Background results enter through [`Runtime::channel`]. Only owned messages
//! cross threads; registered callbacks write state in a batch when the host calls
//! [`Runtime::drain_messages`]. Dropping the subscription closes delivery.
//!
//! Cloned handles share ownership. Dropping the last handle queues retirement;
//! [`Runtime::collect`] removes the attribute and its edges at a safe point.

mod channel;
mod derived;
mod handle;
mod observer;
mod runtime;
mod state;

pub use channel::{MessageSender, MessageSubscription, SendError};
pub use derived::Derived;
pub use handle::AttributeId;
pub use observer::{Dependencies, Observer};
pub use runtime::{GraphStats, Runtime};
pub use state::{Binding, BindingId, Read, State};

#[cfg(test)]
mod tests;

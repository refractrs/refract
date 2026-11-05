//! Staged dependencies and caller-evaluated subscriptions.

use crate::{handle::Handle, AttributeId, Runtime};

/// A staged set of reads, not a live subscription or an owning handle.
#[derive(Clone, Default, Debug)]
pub struct Dependencies(pub(crate) Vec<AttributeId>);

impl Dependencies {
    /// Add an observer as a dependency, for example to aggregate a subtree.
    pub fn include(&mut self, observer: &Observer) {
        if !self.0.contains(&observer.id()) {
            self.0.push(observer.id());
        }
    }
}

/// A subscription for work evaluated by the caller rather than by the graph.
///
/// Stage reads with [`Runtime::capture`] and publish them with [`Runtime::accept`].
/// The graph marks this observer dirty when its accepted dependencies change.
#[derive(Clone)]
pub struct Observer(Handle<()>);

impl Observer {
    pub(crate) fn new(handle: Handle<()>) -> Self {
        Self(handle)
    }

    pub fn id(&self) -> AttributeId {
        self.0.id()
    }

    /// Whether the caller needs to evaluate this observer against the graph.
    pub fn is_dirty(&self, rt: &Runtime) -> bool {
        rt.is_dirty(self.id())
    }
}

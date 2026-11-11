//! Staged dependencies and caller-evaluated subscriptions.

use crate::{handle::Handle, AttributeId, Runtime};
use std::collections::HashSet;

/// A staged set of reads, not a live subscription or an owning handle.
#[derive(Clone, Default, Debug)]
pub struct Dependencies {
    pub(crate) sources: Vec<AttributeId>,
    // Most bodies read very few attributes. Keep those captures as a small
    // vector, and index wider sets to avoid quadratic duplicate checks.
    // Box only the uncommon index so small dependency sets stay compact when
    // retained in a capture stack or a pending UI update.
    #[allow(clippy::box_collection)]
    seen: Option<Box<HashSet<AttributeId>>>,
}

impl Dependencies {
    /// Add an observer as a dependency, for example to aggregate a subtree.
    pub fn include(&mut self, observer: &Observer) {
        self.insert(observer.id());
    }

    pub(crate) fn insert(&mut self, id: AttributeId) {
        let fresh = match &mut self.seen {
            Some(seen) => seen.insert(id),
            None => !self.sources.contains(&id),
        };
        if fresh {
            self.sources.push(id);
            if self.seen.is_none() && self.sources.len() == 128 {
                self.seen = Some(Box::new(self.sources.iter().copied().collect()));
            }
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

//! Attribute storage, dependency publication, propagation, and retirement.

use crate::{handle::Handle, AttributeId, Dependencies, Observer};
use std::{
    any::Any,
    cell::RefCell,
    collections::{HashMap, HashSet, VecDeque},
    rc::Rc,
};

type Compute = dyn Fn(&Runtime) -> Box<dyn Any>;
pub(crate) type Equal = dyn Fn(&dyn Any, &dyn Any) -> bool;

pub(crate) struct Computation {
    pub(crate) compute: Rc<Compute>,
    pub(crate) equal: Option<Rc<Equal>>,
}

// Each edge is stored in both directions: sources support subscription
// replacement and retirement; readers support downstream invalidation.
struct Attribute {
    value: Box<dyn Any>,
    revision: u64,
    sources: Vec<AttributeId>,
    readers: HashSet<AttributeId>,
    compute: Option<Computation>,
    observer: bool,
    dirty: bool,
}

/// Current graph size and cumulative propagation counters.
#[derive(Clone, Copy, Debug, Default)]
pub struct GraphStats {
    pub attributes: usize,
    pub edges: usize,
    /// Derived recomputations after their initial allocation.
    pub computations: u64,
    /// Observer transitions from clean to dirty.
    pub invalidations: u64,
}

/// Owns attribute storage, dependency tracking, and the propagation work queue.
///
/// State writes settle derived computations and invalidate observers. The caller
/// decides when to evaluate observers and when to collect retired attributes.
pub struct Runtime {
    attributes: HashMap<AttributeId, Attribute>,

    // A stack isolates reads made by nested captures.
    tracking: RefCell<Vec<Dependencies>>,
    retired: Rc<RefCell<Vec<AttributeId>>>,

    // `queued` deduplicates pending work, but a processed node can be enqueued
    // again if another input changes before propagation finishes.
    queue: VecDeque<AttributeId>,
    queued: HashSet<AttributeId>,
    batch_depth: usize,

    // State-write version used by callers to reject stale prepared work.
    epoch: u64,
    dirty_observers: HashSet<AttributeId>,
    wake: Option<Rc<dyn Fn()>>,

    computations: u64,
    invalidations: u64,
}

impl Default for Runtime {
    fn default() -> Self {
        Self::new()
    }
}

impl Runtime {
    pub fn new() -> Self {
        Self {
            attributes: HashMap::new(),
            tracking: RefCell::new(Vec::new()),
            retired: Rc::default(),
            queue: VecDeque::new(),
            queued: HashSet::new(),
            batch_depth: 0,
            epoch: 0,
            dirty_observers: HashSet::new(),
            wake: None,
            computations: 0,
            invalidations: 0,
        }
    }

    pub(crate) fn allocate<T: 'static>(
        &mut self,
        value: T,
        compute: Option<Computation>,
    ) -> Handle<T> {
        let id = AttributeId::new();
        self.attributes.insert(
            id,
            Attribute {
                value: Box::new(value),
                revision: 0,
                sources: Vec::new(),
                readers: HashSet::new(),
                compute,
                observer: false,
                dirty: false,
            },
        );
        Handle::new(id, Rc::downgrade(&self.retired))
    }

    // Keep state mutation and invalidation together; typed handles do not need
    // access to the runtime's storage or propagation queue.
    pub(crate) fn write<T: 'static>(&mut self, id: AttributeId, value: T) {
        self.epoch = self.epoch.checked_add(1).expect("mutation epoch exhausted");
        let attr = self
            .attributes
            .get_mut(&id)
            .expect("state belongs to another or disposed runtime");
        *attr
            .value
            .downcast_mut::<T>()
            .expect("attribute type mismatch") = value;
        attr.revision = attr.revision.checked_add(1).expect("revision exhausted");
        self.enqueue_readers(id);
        if self.batch_depth == 0 {
            self.settle();
        }
    }

    pub(crate) fn is_dirty(&self, id: AttributeId) -> bool {
        self.attributes[&id].dirty
    }

    pub(crate) fn read_untracked<T: 'static>(&self, id: AttributeId) -> &T {
        self.attributes
            .get(&id)
            .expect("attribute belongs to another or disposed runtime")
            .value
            .downcast_ref()
            .expect("attribute type mismatch")
    }

    pub(crate) fn read<T: 'static>(&self, id: AttributeId) -> &T {
        let value = self.read_untracked(id);
        if let Some(reads) = self.tracking.borrow_mut().last_mut() {
            if !reads.0.contains(&id) {
                reads.0.push(id);
            }
        }
        value
    }

    /// Install a scheduler notification, called after propagation dirties observers.
    pub fn set_waker(&mut self, wake: impl Fn() + 'static) {
        self.wake = Some(Rc::new(wake));
    }

    /// Remove the scheduler notification.
    pub fn clear_waker(&mut self) {
        self.wake = None;
    }

    /// Allocate an initially dirty observer with no accepted dependencies.
    pub fn observer(&mut self) -> Observer {
        let handle = self.allocate((), None);
        let attr = self.attributes.get_mut(&handle.id()).unwrap();
        attr.observer = true;
        attr.dirty = true;
        Observer::new(handle)
    }

    /// Evaluate a closure and return its reads without publishing graph edges.
    /// Nested captures collect their own reads independently.
    pub fn capture<R>(&self, f: impl FnOnce(&Self) -> R) -> (R, Dependencies) {
        self.tracking.borrow_mut().push(Dependencies::default());

        struct Pop<'a>(&'a RefCell<Vec<Dependencies>>);

        impl Drop for Pop<'_> {
            fn drop(&mut self) {
                self.0.borrow_mut().pop();
            }
        }

        // Restore the capture stack even if the closure unwinds.
        let guard = Pop(&self.tracking);
        let value = f(self);
        let reads = self.tracking.borrow().last().unwrap().clone();
        drop(guard);
        (value, reads)
    }

    /// Capture reads while allowing allocation and mutation, such as mounted
    /// state initialization during UI preparation. This does not roll back writes.
    pub fn capture_mut<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> (R, Dependencies) {
        self.tracking.borrow_mut().push(Dependencies::default());
        // A guard borrowing `tracking` would prevent passing `&mut self` to f.
        // Catch only to restore the stack, then resume the original panic.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(self)));
        let reads = self.tracking.borrow_mut().pop().unwrap();
        match result {
            Ok(value) => (value, reads),
            Err(e) => std::panic::resume_unwind(e),
        }
    }

    /// Replace an observer's subscriptions and mark its accepted output clean.
    pub fn accept(&mut self, observer: &Observer, dependencies: Dependencies) {
        self.replace_dependencies(observer.id(), dependencies);
        self.attributes
            .get_mut(&observer.id())
            .expect("foreign observer")
            .dirty = false;
        self.dirty_observers.remove(&observer.id());
    }

    /// Mark an observer clean while retaining its existing subscriptions.
    pub fn mark_clean(&mut self, observer: &Observer) {
        self.attributes
            .get_mut(&observer.id())
            .expect("foreign observer")
            .dirty = false;
        self.dirty_observers.remove(&observer.id());
    }

    pub(crate) fn replace_dependencies(&mut self, target: AttributeId, dependencies: Dependencies) {
        // Validate before removing existing edges so invalid input cannot leave
        // a partially replaced subscription.
        for source in &dependencies.0 {
            assert!(self.attributes.contains_key(source), "foreign dependency");
            assert_ne!(*source, target, "self dependency");
        }
        let attr = self.attributes.get_mut(&target).expect("foreign observer");
        if attr.sources == dependencies.0 {
            return;
        }
        let previous = std::mem::replace(&mut attr.sources, dependencies.0);
        for source in previous {
            if let Some(attr) = self.attributes.get_mut(&source) {
                attr.readers.remove(&target);
            }
        }
        let sources = self.attributes[&target].sources.clone();
        for source in sources {
            self.attributes
                .get_mut(&source)
                .unwrap()
                .readers
                .insert(target);
        }
    }

    fn enqueue_readers(&mut self, source: AttributeId) {
        for &id in &self.attributes[&source].readers {
            if self.queued.insert(id) {
                self.queue.push_back(id);
            }
        }
    }

    fn settle(&mut self) {
        let mut visits = HashMap::<AttributeId, usize>::new();
        let mut wake = false;

        while let Some(id) = self.queue.pop_front() {
            self.queued.remove(&id);
            let n = visits.entry(id).or_default();
            *n += 1;
            assert!(
                *n <= 1024,
                "attribute graph did not settle; possible dependency cycle at {id:?}"
            );
            let Some(attr) = self.attributes.get(&id) else {
                continue;
            };

            if attr.observer {
                // Observer bodies belong to the caller. Only propagate dirtiness
                // here, including to observers that aggregate other observers.
                wake = true;
                self.dirty_observers.insert(id);
                if attr.dirty {
                    continue;
                }
                let attr = self.attributes.get_mut(&id).unwrap();
                attr.dirty = true;
                self.invalidations += 1;
                wake = true;
                self.enqueue_readers(id);
            } else if let Some(compute) = &attr.compute {
                let f = compute.compute.clone();
                let equal = compute.equal.clone();
                let (value, deps) = self.capture(|rt| f(rt));
                // Equal output can still come from different inputs after a
                // conditional branch, so dependency replacement is unconditional.
                let changed =
                    equal.is_none_or(|eq| !eq(self.attributes[&id].value.as_ref(), value.as_ref()));
                self.replace_dependencies(id, deps);
                self.computations += 1;
                if changed {
                    let attr = self.attributes.get_mut(&id).unwrap();
                    attr.value = value;
                    attr.revision = attr.revision.checked_add(1).expect("revision exhausted");
                    self.enqueue_readers(id);
                }
            }
        }

        // Notify once after this worklist settles, not once per dirty node.
        if wake {
            if let Some(waker) = &self.wake {
                waker();
            }
        }
    }

    /// Defer propagation until the outermost successful batch finishes.
    /// State writes are immediate; derived values can remain stale inside the batch.
    /// Unwinding restores batch depth but does not roll back writes or settle work.
    pub fn batch<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        self.batch_depth += 1;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(self)));
        self.batch_depth -= 1;
        match result {
            Ok(value) => {
                if self.batch_depth == 0 {
                    self.settle();
                }
                value
            }
            Err(e) => std::panic::resume_unwind(e),
        }
    }

    /// Remove retired attributes, detach their edges, and discard their queued work.
    pub fn collect(&mut self) {
        loop {
            // Dropping a computation can release handles captured by its closure.
            // Drain repeatedly until those cascading retirements are also removed.
            let retired = std::mem::take(&mut *self.retired.borrow_mut());
            if retired.is_empty() {
                break;
            }
            for id in retired {
                if let Some(attr) = self.attributes.remove(&id) {
                    for source in &attr.sources {
                        if let Some(a) = self.attributes.get_mut(source) {
                            a.readers.remove(&id);
                        }
                    }
                    for reader in &attr.readers {
                        if let Some(a) = self.attributes.get_mut(reader) {
                            a.sources.retain(|s| *s != id);
                        }
                    }
                    self.queued.remove(&id);
                    self.dirty_observers.remove(&id);
                }
            }
        }
        self.queue.retain(|id| self.attributes.contains_key(id));
    }

    /// Monotonic state-write version for validating prepared work.
    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// Drain pending notifications without clearing observers' dirty flags.
    /// Evaluation is acknowledged separately with `accept` or `mark_clean`.
    pub fn take_dirty_observers(&mut self) -> Vec<AttributeId> {
        self.dirty_observers.drain().collect()
    }

    /// Inspect current storage size and cumulative propagation work.
    pub fn stats(&self) -> GraphStats {
        GraphStats {
            attributes: self.attributes.len(),
            edges: self.attributes.values().map(|a| a.sources.len()).sum(),
            computations: self.computations,
            invalidations: self.invalidations,
        }
    }
}

//! Attribute storage, dependency publication, propagation, and retirement.

use crate::{handle::Handle, AttributeId, Dependencies, Observer};
use std::{
    any::Any,
    cell::{Cell, OnceCell, RefCell},
    cmp::Reverse,
    collections::{BinaryHeap, HashMap, HashSet, VecDeque},
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
    compute: Option<Box<DerivedAttribute>>,
    observer: bool,
    dirty: bool,
    queued: bool,
    cause: Option<MutationContext>,
    notified_epoch: u64,
}

// Keep evaluation storage with the computation so each settlement can stage
// a result without allocating another box. State and observer nodes stay small.
struct DerivedAttribute {
    computation: Computation,
    // A conservative topological level. Removing edges need not lower it.
    level: u64,
    evaluating: Cell<EvaluationState>,
    evaluation: OnceCell<Evaluation>,
}

impl Attribute {
    fn level(&self) -> u64 {
        self.compute.as_ref().map_or(0, |derived| derived.level)
    }

    fn evaluation(&self) -> Option<&Evaluation> {
        self.compute.as_ref()?.evaluation.get()
    }
}

#[derive(Clone, Copy, PartialEq)]
enum EvaluationState {
    Idle,
    Scheduled,
    Checking,
    Computing,
    // Count actual computations at publication, including equal outputs.
    Computed,
}

struct Evaluation {
    // Only changed values replace storage. Equal computations still publish
    // their dependencies, while skipped computations publish neither.
    value: Option<Box<dyn Any>>,
    dependencies: Option<Dependencies>,
    cause: Option<MutationContext>,
}

type EvaluationFrame = (AttributeId, Option<usize>, bool);

#[derive(Default)]
struct WorkQueue {
    level: u64,
    ready: Vec<AttributeId>,
    later: BinaryHeap<Reverse<(u64, AttributeId)>>,
}

impl WorkQueue {
    fn push(&mut self, level: u64, id: AttributeId) {
        // Chains and same-level fan-outs use a reusable vector. Only work at
        // other levels needs heap ordering; ties do not need to be sorted.
        if self.ready.is_empty() && self.later.is_empty() {
            self.level = level;
        } else if level < self.level {
            self.later
                .extend(self.ready.drain(..).map(|id| Reverse((self.level, id))));
            self.level = level;
        }
        if level == self.level {
            self.ready.push(id);
        } else {
            self.later.push(Reverse((level, id)));
        }
    }

    fn pop(&mut self) -> Option<(u64, AttributeId)> {
        if let Some(id) = self.ready.pop() {
            return Some((self.level, id));
        }
        let Reverse((level, id)) = self.later.pop()?;
        self.level = level;
        while self
            .later
            .peek()
            .is_some_and(|Reverse((next, _))| *next == level)
        {
            let Reverse((_, id)) = self.later.pop().unwrap();
            self.ready.push(id);
        }
        Some((level, id))
    }
}

/// Opaque context of the latest mutation affecting an observer. The graph does
/// not interpret this payload. An absent payload explicitly represents an
/// ordinary mutation and supersedes an earlier contextual mutation.
#[derive(Clone)]
pub struct MutationContext {
    pub epoch: u64,
    value: Option<Rc<dyn Any>>,
}

impl MutationContext {
    pub fn get<T: 'static>(&self) -> Option<&T> {
        self.value.as_ref()?.downcast_ref()
    }
}

/// Current graph size and cumulative propagation counters.
#[derive(Clone, Copy, Debug, Default)]
pub struct GraphStats {
    pub attributes: usize,
    pub edges: usize,
    /// Successfully published derived recomputations after initial allocation.
    pub computations: u64,
    /// Observer transitions from clean to dirty.
    pub invalidations: u64,
}

/// Owns attribute storage, dependency tracking, and the propagation work queue.
///
/// State writes settle derived computations and invalidate observers. The caller
/// decides when to evaluate observers and when to collect retired attributes.
pub struct Runtime {
    pub(crate) channels: crate::channel::Channels,
    attributes: HashMap<AttributeId, Attribute>,

    // A stack isolates reads made by nested captures.
    tracking: RefCell<Vec<Dependencies>>,
    spare_dependencies: RefCell<Dependencies>,
    retired: Rc<RefCell<Vec<AttributeId>>>,

    // Writes enqueue immediate readers. Settlement stages the affected derived
    // subgraph before publishing any values or invalidating observers.
    queue: VecDeque<AttributeId>,
    settling: bool,
    evaluated: RefCell<Vec<AttributeId>>,
    evaluation_stack: RefCell<Vec<EvaluationFrame>>,
    work: WorkQueue,
    level_updates: Vec<AttributeId>,
    readers: Vec<AttributeId>,
    settling_level: Cell<u64>,
    #[cfg(test)]
    pub(crate) validations: usize,
    batch_depth: usize,
    mutation_context: Option<Rc<dyn Any>>,

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
            channels: Default::default(),
            attributes: HashMap::new(),
            tracking: RefCell::new(Vec::new()),
            spare_dependencies: RefCell::default(),
            retired: Rc::default(),
            queue: VecDeque::new(),
            settling: false,
            evaluated: RefCell::new(Vec::new()),
            evaluation_stack: RefCell::new(Vec::new()),
            work: WorkQueue::default(),
            level_updates: Vec::new(),
            readers: Vec::new(),
            settling_level: Cell::new(0),
            #[cfg(test)]
            validations: 0,
            batch_depth: 0,
            mutation_context: None,
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
                compute: compute.map(|computation| {
                    Box::new(DerivedAttribute {
                        computation,
                        level: 0,
                        evaluating: Cell::new(EvaluationState::Idle),
                        evaluation: OnceCell::new(),
                    })
                }),
                observer: false,
                dirty: false,
                queued: false,
                cause: None,
                notified_epoch: 0,
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
        attr.cause = Some(MutationContext {
            epoch: self.epoch,
            value: self.mutation_context.clone(),
        });
        self.enqueue_readers(id);
        if self.batch_depth == 0 {
            self.settle();
        }
    }

    pub(crate) fn is_dirty(&self, id: AttributeId) -> bool {
        self.attributes[&id].dirty
    }

    pub(crate) fn read_untracked<T: 'static>(&self, id: AttributeId) -> &T {
        let attr = self
            .attributes
            .get(&id)
            .expect("attribute belongs to another or disposed runtime");
        if self.settling && attr.compute.is_some() {
            self.evaluate(id);
            if let Some(value) = attr.evaluation().and_then(|result| result.value.as_ref()) {
                return value.downcast_ref().expect("attribute type mismatch");
            }
        }
        attr.value.downcast_ref().expect("attribute type mismatch")
    }

    pub(crate) fn read<T: 'static>(&self, id: AttributeId) -> &T {
        let value = self.read_untracked(id);
        if let Some(reads) = self.tracking.borrow_mut().last_mut() {
            reads.insert(id);
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
        let dependencies = std::mem::take(&mut *self.spare_dependencies.borrow_mut());
        self.tracking.borrow_mut().push(dependencies);

        struct Pop<'a> {
            tracking: &'a RefCell<Vec<Dependencies>>,
            active: bool,
        }

        impl Drop for Pop<'_> {
            fn drop(&mut self) {
                if self.active {
                    self.tracking.borrow_mut().pop();
                }
            }
        }

        // Restore the capture stack even if the closure unwinds.
        let mut guard = Pop {
            tracking: &self.tracking,
            active: true,
        };
        let value = f(self);
        let reads = self.tracking.borrow_mut().pop().unwrap();
        guard.active = false;
        (value, reads)
    }

    /// Capture reads while allowing allocation and mutation, such as mounted
    /// state initialization during UI preparation. This does not roll back writes.
    pub fn capture_mut<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> (R, Dependencies) {
        let dependencies = std::mem::take(&mut *self.spare_dependencies.borrow_mut());
        self.tracking.borrow_mut().push(dependencies);
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
        self.mark_clean(observer);
    }

    /// Mark an observer clean while retaining its existing subscriptions.
    pub fn mark_clean(&mut self, observer: &Observer) {
        let attr = self
            .attributes
            .get_mut(&observer.id())
            .expect("foreign observer");
        attr.dirty = false;
        self.dirty_observers.remove(&observer.id());
        attr.cause = None;
    }

    fn recycle_dependencies(&self, mut dependencies: Dependencies) {
        // Keep one reusable capture, rather than a spare allocation per node.
        // Nested captures and staged edge changes retain independent buffers.
        let mut spare = self.spare_dependencies.borrow_mut();
        if dependencies.sources.capacity() >= spare.sources.capacity() {
            dependencies.clear();
            *spare = dependencies;
        }
    }

    pub(crate) fn replace_dependencies(
        &mut self,
        target: AttributeId,
        mut dependencies: Dependencies,
    ) {
        // Accepted edges are already valid, and collection removes retired
        // sources from this list. An unchanged read set needs no revalidation.
        if self
            .attributes
            .get(&target)
            .expect("foreign observer")
            .sources
            == dependencies.sources
        {
            self.recycle_dependencies(dependencies);
            return;
        }
        // Validate before removing existing edges so invalid input cannot leave
        // a partially replaced subscription.
        for source in &dependencies.sources {
            assert!(self.attributes.contains_key(source), "foreign dependency");
            assert_ne!(*source, target, "self dependency");
        }
        let attr = self.attributes.get_mut(&target).expect("foreign observer");
        let previous = std::mem::take(&mut attr.sources);
        for source in &previous {
            if let Some(attr) = self.attributes.get_mut(source) {
                attr.readers.remove(&target);
            }
        }
        for source in &dependencies.sources {
            self.attributes
                .get_mut(source)
                .unwrap()
                .readers
                .insert(target);
        }
        self.attributes.get_mut(&target).unwrap().sources =
            std::mem::replace(&mut dependencies.sources, previous);
        self.recycle_dependencies(dependencies);
        self.update_level(target);
    }

    fn update_level(&mut self, id: AttributeId) {
        let Some(attr) = self.attributes.get(&id) else {
            return;
        };
        if attr.compute.is_none() {
            return;
        }
        let level = attr
            .sources
            .iter()
            .map(|id| self.attributes[id].level())
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .expect("dependency level exhausted");
        if level > attr.level() {
            self.level_updates.extend(&attr.readers);
            self.attributes
                .get_mut(&id)
                .unwrap()
                .compute
                .as_mut()
                .unwrap()
                .level = level;
        }
    }

    fn enqueue_readers(&mut self, source: AttributeId) {
        let cause = self.attributes[&source].cause.clone();
        // Derived results are published only after all computations settle.
        // Their derived readers have already resolved these changes on reads.
        let settled = self.attributes[&source].compute.is_some();
        let mut readers = std::mem::take(&mut self.readers);
        readers.extend(
            self.attributes[&source]
                .readers
                .iter()
                .copied()
                .filter(|id| !settled || self.attributes[id].observer),
        );
        for id in readers.drain(..) {
            let attr = self.attributes.get_mut(&id).unwrap();
            if let Some(cause) = &cause {
                if attr
                    .cause
                    .as_ref()
                    .is_none_or(|old| old.epoch < cause.epoch)
                {
                    attr.cause = Some(cause.clone());
                }
            }
            if !attr.queued {
                attr.queued = true;
                self.queue.push_back(id);
            }
        }
        self.readers = readers;
    }

    // Resolve accepted dependencies first, then let reads resolve any newly
    // selected dependencies. OnceCell keeps borrowed values stable throughout
    // evaluation without mutating the published attribute storage.
    fn evaluate(&self, id: AttributeId) {
        let attr = &self.attributes[&id];
        let Some(derived) = &attr.compute else {
            return;
        };
        if derived.evaluation.get().is_some() || attr.level() < self.settling_level.get() {
            return;
        }
        assert!(
            derived.evaluating.get() != EvaluationState::Computing,
            "attribute graph dependency cycle at {id:?}"
        );
        if derived.evaluating.get() == EvaluationState::Checking {
            // A new read can reverse an accepted edge. Resolve actual reads
            // now; only reentering a computation proves a current cycle.
            self.compute(id);
            return;
        }
        // Validate existing edges iteratively, even for directly queued nodes,
        // so deep chains with shared inputs do not recurse through closures.
        let mut stack = std::mem::take(&mut *self.evaluation_stack.borrow_mut());
        stack.push((id, None, false));
        'evaluate: while let Some((id, next_source, mut needs_compute)) = stack.pop() {
            let attr = &self.attributes[&id];
            let derived = attr.compute.as_ref().unwrap();
            if derived.evaluation.get().is_some() {
                continue;
            }
            if next_source.is_none() {
                needs_compute |= derived.evaluating.get() == EvaluationState::Scheduled;
                derived.evaluating.set(EvaluationState::Checking);
                self.evaluated.borrow_mut().push(id);
            }
            for index in next_source.unwrap_or(0)..attr.sources.len() {
                let source = attr.sources[index];
                let input = &self.attributes[&source];
                if input.compute.is_none() {
                    continue;
                }
                if let Some(result) = input.evaluation() {
                    needs_compute |= result.value.is_some();
                } else if input.level() < self.settling_level.get() {
                    // All pending work below the current level has settled.
                    // No changed input can still reach this clean ancestor.
                    continue;
                } else if matches!(
                    input.compute.as_ref().unwrap().evaluating.get(),
                    EvaluationState::Checking | EvaluationState::Computing
                ) {
                    // This old edge may disappear on the new branch. Let the
                    // computation decide whether it still reads that input.
                    needs_compute = true;
                    break;
                } else {
                    stack.push((id, Some(index), needs_compute));
                    stack.push((source, None, false));
                    continue 'evaluate;
                }
            }
            if needs_compute {
                self.compute(id);
            } else {
                assert!(derived
                    .evaluation
                    .set(Evaluation {
                        value: None,
                        dependencies: None,
                        cause: attr.cause.clone(),
                    })
                    .is_ok());
                derived.evaluating.set(EvaluationState::Idle);
            }
        }
        // Nested reads may have returned a smaller stack while this one was
        // in use. Retain the larger allocation for the next evaluation.
        let mut spare = self.evaluation_stack.borrow_mut();
        if spare.capacity() < stack.capacity() {
            *spare = stack;
        }
    }

    fn compute(&self, id: AttributeId) {
        let attr = &self.attributes[&id];
        let derived = attr.compute.as_ref().unwrap();
        derived.evaluating.set(EvaluationState::Computing);
        let changed = |source: &AttributeId| {
            self.attributes[source]
                .evaluation()
                .filter(|result| result.value.is_some())
        };
        let mut result = Evaluation {
            value: None,
            dependencies: None,
            cause: attr.cause.clone(),
        };
        let computation = &derived.computation;
        let (value, dependencies) = self.capture(|rt| (computation.compute)(rt));
        // Newly selected inputs can carry a later mutation context too.
        let new_sources = if attr.sources == dependencies.sources {
            &[][..]
        } else {
            &dependencies.sources[..]
        };
        for source in attr.sources.iter().chain(new_sources) {
            if let Some(cause) = changed(source).and_then(|result| result.cause.as_ref()) {
                if result
                    .cause
                    .as_ref()
                    .is_none_or(|old| old.epoch < cause.epoch)
                {
                    result.cause = Some(cause.clone());
                }
            }
        }
        if computation
            .equal
            .as_ref()
            .is_none_or(|eq| !eq(attr.value.as_ref(), value.as_ref()))
        {
            result.value = Some(value);
        }
        if attr.sources == dependencies.sources {
            self.recycle_dependencies(dependencies);
        } else {
            result.dependencies = Some(dependencies);
        }
        assert!(derived.evaluation.set(result).is_ok());
        derived.evaluating.set(EvaluationState::Computed);
    }

    fn schedule(&self, id: AttributeId, work: &mut WorkQueue) {
        let attr = &self.attributes[&id];
        if let Some(derived) = &attr.compute {
            if derived.evaluation.get().is_none()
                && derived.evaluating.get() == EvaluationState::Idle
            {
                derived.evaluating.set(EvaluationState::Scheduled);
                work.push(attr.level(), id);
            }
        }
    }

    fn settle(&mut self) {
        // Dependency replacement can temporarily reverse accepted edges.
        // Repair downstream levels only after all replacements are published.
        while let Some(id) = self.level_updates.pop() {
            self.update_level(id);
        }
        self.settling = true;
        self.settling_level.set(0);
        let mut work = std::mem::take(&mut self.work);
        for id in &self.queue {
            self.schedule(*id, &mut work);
        }
        let evaluated = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut propagated = 0;
            while let Some((level, id)) = work.pop() {
                debug_assert!(level >= self.settling_level.get());
                self.settling_level.set(level);
                self.evaluate(id);
                // Evaluation may resolve more than this node. Propagate every
                // changed result once, and stop at equal outputs. Reads pull
                // any dependencies that have not reached the worklist yet.
                let evaluated = self.evaluated.borrow();
                for id in &evaluated[propagated..] {
                    let attr = &self.attributes[id];
                    if attr.evaluation().unwrap().value.is_some() {
                        for reader in &attr.readers {
                            self.schedule(*reader, &mut work);
                        }
                    }
                }
                propagated = evaluated.len();
            }
        }));
        self.settling = false;
        let mut touched = std::mem::take(self.evaluated.get_mut());
        #[cfg(test)]
        {
            self.validations += touched.len();
        }
        if let Err(error) = evaluated {
            while let Some((_, id)) = work.pop() {
                self.attributes[&id]
                    .compute
                    .as_ref()
                    .unwrap()
                    .evaluating
                    .set(EvaluationState::Idle);
            }
            self.work = work;
            for id in touched.drain(..) {
                let attr = self.attributes.get_mut(&id).unwrap();
                let derived = attr.compute.as_mut().unwrap();
                derived.evaluation.take();
                derived.evaluating.set(EvaluationState::Idle);
            }
            *self.evaluated.get_mut() = touched;
            // Keep the original work queued so a later settlement can retry.
            std::panic::resume_unwind(error);
        }
        self.work = work;

        for id in touched.drain(..) {
            let derived = self
                .attributes
                .get_mut(&id)
                .unwrap()
                .compute
                .as_mut()
                .unwrap();
            let computed =
                derived.evaluating.replace(EvaluationState::Idle) == EvaluationState::Computed;
            let result = derived.evaluation.take().unwrap();
            if let Some(dependencies) = result.dependencies {
                self.replace_dependencies(id, dependencies);
            }
            if computed {
                self.computations += 1;
            }
            let attr = self.attributes.get_mut(&id).unwrap();
            attr.cause = result.cause;
            if let Some(value) = result.value {
                attr.value = value;
                attr.revision = attr.revision.checked_add(1).expect("revision exhausted");
                // Queue observer notifications now, but deliver them only after
                // every staged value and dependency set has been published.
                self.enqueue_readers(id);
            }
        }
        *self.evaluated.get_mut() = touched;

        let mut wake = false;
        while let Some(id) = self.queue.pop_front() {
            let Some(attr) = self.attributes.get_mut(&id) else {
                continue;
            };
            attr.queued = false;

            if attr.observer {
                // Observer bodies belong to the caller. Only propagate dirtiness
                // here, including to observers that aggregate other observers.
                wake = true;
                self.dirty_observers.insert(id);
                let epoch = attr.cause.as_ref().map_or(0, |cause| cause.epoch);
                if attr.dirty && attr.notified_epoch >= epoch {
                    continue;
                }
                if !attr.dirty {
                    self.invalidations += 1;
                }
                attr.dirty = true;
                attr.notified_epoch = epoch;
                wake = true;
                self.enqueue_readers(id);
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

    /// Run synchronous mutations with caller-owned context, preserving nested
    /// scopes and restoring context on unwind. Deferred propagation carries the
    /// context captured at each write, not the context at batch settlement.
    pub fn with_mutation_context<T: 'static, R>(
        &mut self,
        context: T,
        f: impl FnOnce(&mut Self) -> R,
    ) -> R {
        let previous = self.mutation_context.replace(Rc::new(context));
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.batch(f)));
        self.mutation_context = previous;
        match result {
            Ok(value) => value,
            Err(error) => std::panic::resume_unwind(error),
        }
    }

    /// Context remains available until an observer's output is accepted. Draining
    /// dirty notifications or abandoning a prepared UI does not consume it.
    pub fn observer_context(&self, observer: &Observer) -> Option<&MutationContext> {
        self.attributes.get(&observer.id())?.cause.as_ref()
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

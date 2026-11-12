//! Mounted state, incremental reconciliation, and host commit coordination.

use crate::{element::ElementIdentity, environment::Entry, *};
use crate::transaction::AnimationContext;
use std::{
    any::{Any, TypeId},
    cell::{Cell, RefCell},
    collections::{BTreeMap, HashMap, HashSet},
    rc::{Rc, Weak},
};

/// Stable identity within one app; removed identities are not reused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ElementId(pub u64);

type MountId = ElementId;

#[path = "focus_runtime.rs"]
mod focus_runtime;

/// Work performed while preparing a single update.
#[derive(Clone, Copy, Debug, Default)]
pub struct UpdateStats {
    /// Mounted descriptions evaluated, including primitive views.
    pub evaluated: usize,
    /// Mounts reconciled after clean-subtree bailouts, including ancestors whose
    /// own bodies were skipped. Excludes child matching and memo equality checks.
    pub visited: usize,
    pub created: usize,
    pub removed: usize,
    pub patches: usize,
}

/// Semantic changes for a host adapter to apply in order.
#[derive(Clone, Debug, PartialEq)]
pub enum Patch {
    Create(ElementId, Element),
    /// Replace the element description; adapters can diff individual properties.
    Update(ElementId, Element),
    /// Set ordered children; `None` denotes the adapter's root container.
    Children(Option<ElementId>, Vec<ElementId>),
    Remove(ElementId),
}

/// A patch batch based on the host's previously committed revision.
#[derive(Clone, Debug)]
pub struct Update {
    pub base_revision: u64,
    pub revision: u64,
    pub patches: Vec<Patch>,
    pub stats: UpdateStats,
    /// Animation intent for changed or removed host elements in this update.
    pub animations: BTreeMap<ElementId, Transaction>,
}

/// Effect teardown, consumed once when the effect is replaced or unmounted.
pub type Cleanup = Box<dyn FnOnce()>;

type StartEffect = dyn Fn(&mut Runtime) -> Cleanup;

// Candidate mounts share an accepted effect's lifecycle cells when dependencies
// are equal. Merely cloning a candidate must neither start nor stop the effect.
#[derive(Clone)]
struct Effect {
    dependencies: Rc<dyn Any>,
    start: Rc<StartEffect>,
    cleanup: Rc<RefCell<Option<Cleanup>>>,
    started: Rc<Cell<bool>>,
}

impl Effect {
    fn stop(&self) {
        if let Some(cleanup) = self.cleanup.borrow_mut().take() {
            cleanup();
        }
        self.started.set(false);
    }

    fn start(&self, rt: &mut Runtime) {
        if !self.started.replace(true) {
            *self.cleanup.borrow_mut() = Some((self.start)(rt));
        }
    }
}

/// Weak lifetime proof for view-owned asynchronous completions.
#[derive(Clone)]
pub struct ScopeToken(Weak<Cell<bool>>);

impl ScopeToken {
    pub fn is_alive(&self) -> bool {
        self.0.upgrade().is_some_and(|v| v.get())
    }
}

/// Access to tracked reads and named mounted storage during body evaluation.
/// Declarations are staged here; effects start only after the candidate commits.
pub struct ViewContext<'a> {
    runtime: &'a mut Runtime,
    environment: &'a Environment,

    // Previous declarations are retained until the candidate is committed.
    previous_slots: &'a HashMap<String, Rc<dyn Any>>,
    slots: HashMap<String, Rc<dyn Any>>,

    previous_effects: &'a HashMap<String, Effect>,
    effects: HashMap<String, Effect>,

    owner: &'a Rc<Cell<bool>>,
    error: Option<ViewError>,
}

impl ViewContext<'_> {
    /// Read a binding or read-only handle and subscribe to its graph attributes.
    pub fn read<T: 'static>(&self, binding: &Read<T>) -> T {
        binding.get(self.runtime)
    }

    /// Inspect a readable handle without cloning its stored value, tracking its reads.
    /// Mapped bindings compute their projection from a borrowed source.
    /// Return an owned result; the inspected reference cannot escape the callback.
    pub fn with<T: 'static, R>(&self, binding: &Read<T>, f: impl FnOnce(&T) -> R) -> R {
        binding.with(self.runtime, f)
    }

    /// Borrow state while tracking its dependency. The borrow prevents mutable
    /// access to this context until its last use.
    pub fn read_state_ref<T: 'static>(&self, state: &State<T>) -> &T {
        state.get(self.runtime)
    }

    /// Borrow a cached derived value while tracking its dependency.
    pub fn read_derived_ref<T: 'static>(&self, value: &Derived<T>) -> &T {
        value.get(self.runtime)
    }

    /// Read and clone state while tracking its dependency.
    pub fn read_state<T: Clone + 'static>(&self, state: &State<T>) -> T {
        state.get(self.runtime).clone()
    }

    /// Read and clone a cached derived value while tracking its dependency.
    pub fn read_derived<T: Clone + 'static>(&self, value: &Derived<T>) -> T {
        value.get(self.runtime).clone()
    }

    /// Reuse graph state in a named slot, initializing it when that slot is new.
    /// Slot names and types must remain stable across evaluations of this mount.
    pub fn state<T: Clone + 'static>(
        &mut self,
        name: &str,
        initial: impl FnOnce() -> T,
    ) -> Binding<T> {
        let state = self.remember(name, |rt| State::new(rt, initial()));
        state.binding()
    }

    /// Retain a shared value in a named slot without making the value reactive.
    /// Initializers can run in discarded preparations; use effects for external work.
    pub fn remember<T: 'static>(
        &mut self,
        name: &str,
        initial: impl FnOnce(&mut Runtime) -> T,
    ) -> Rc<T> {
        if name.is_empty() || self.slots.contains_key(name) {
            self.error = Some(ViewError::new(format!(
                "duplicate or empty mounted slot: {name}"
            )));
        }
        let value = if let Some(value) = self.previous_slots.get(name) {
            match value.clone().downcast::<T>() {
                Ok(value) => value,
                Err(_) => {
                    self.error = Some(ViewError::new(format!("mounted slot changed type: {name}")));
                    Rc::new(initial(self.runtime))
                }
            }
        } else {
            Rc::new(initial(self.runtime))
        };
        self.slots.insert(name.to_owned(), value.clone());
        value
    }

    /// Read the nearest environment value, materializing its default if needed.
    pub fn environment<K: EnvironmentKey>(&mut self) -> K::Value {
        self.environment.ensure::<K>(self.runtime);
        self.environment.read::<K>(self.runtime)
    }

    /// Read an inherited shared model, or fail if no provider exists.
    /// Read the model's graph bindings separately to subscribe to its fields.
    pub fn object<T: 'static>(&mut self) -> Result<Rc<T>, ViewError> {
        self.environment::<crate::environment::ObjectKey<T>>()
            .0
            .ok_or_else(|| {
                ViewError::new(format!(
                    "missing environment object: {}",
                    std::any::type_name::<T>()
                ))
            })
    }

    /// Obtain a weak lifetime token for completions owned by this mounted view.
    pub fn scope(&self) -> ScopeToken {
        ScopeToken(Rc::downgrade(self.owner))
    }

    /// Stage an effect keyed by name and explicit dependency equality.
    /// Equal dependencies retain the previous setup and captures. Changed or removed
    /// effects are cleaned up at commit before replacement setups run.
    pub fn effect<T: PartialEq + 'static>(
        &mut self,
        name: &str,
        dependencies: T,
        start: impl Fn(&mut Runtime) -> Cleanup + 'static,
    ) {
        if self.effects.contains_key(name) {
            self.error = Some(ViewError::new(format!("duplicate effect: {name}")));
        }
        let effect = if let Some(old) = self
            .previous_effects
            .get(name)
            .filter(|e| e.dependencies.downcast_ref::<T>() == Some(&dependencies))
        {
            old.clone()
        } else {
            Effect {
                dependencies: Rc::new(dependencies),
                start: Rc::new(start),
                cleanup: Rc::default(),
                started: Rc::new(Cell::new(false)),
            }
        };
        self.effects.insert(name.to_owned(), effect);
    }
}

// The accepted description and resources for one composition boundary. During
// preparation, only affected mounts are cloned into the candidate overlay.
#[derive(Clone)]
struct Mounted {
    id: MountId,
    parent: Option<MountId>,
    view: AnyView,
    environment: Environment,
    element: Option<Element>,
    handler: Option<Handler>,
    children: Rc<Vec<MountId>>,

    // Traversing an ancestor for a dirty descendant must not copy every state
    // slot. Body evaluation tracks declarations in its private context; existing
    // slots keep the same names, types, and shared values throughout a mount.
    slots: Rc<HashMap<String, Rc<dyn Any>>>,
    effects: HashMap<String, Effect>,
    provider: Option<Entry>,
    memo: Option<crate::Memo>,
    animation_scope: Option<AnimationScope>,

    // Body reads and descendant dirtiness are tracked separately. A dirty
    // descendant does not require re-evaluating this mount's own body.
    body: Observer,
    subtree: Observer,
    owner: Rc<Cell<bool>>,
}

#[derive(Clone, Hash, PartialEq, Eq)]
enum Slot {
    Key(String),
    Index(usize),
}

// Keys replace positional identity, but type and host identity must also match.
#[derive(Clone, Hash, PartialEq, Eq)]
struct Identity(Slot, TypeId, Option<ElementIdentity>);

fn identity(view: &AnyView, index: usize) -> Identity {
    Identity(
        view.identity_key()
            .map(Slot::Key)
            .unwrap_or(Slot::Index(index)),
        view.type_id(),
        view.element_identity(),
    )
}

/// A prepared candidate whose patches have not yet been acknowledged by the host.
/// Dropping it leaves the accepted mounted tree and effect lifecycles intact.
pub struct Pending {
    app_identity: Rc<()>,
    input_revision: u64,
    epoch: u64,
    pub update: Update,
    changed: BTreeMap<MountId, Rc<Mounted>>,
    removed: HashSet<MountId>,
    root: MountId,
    subscriptions: Vec<(Observer, Option<Dependencies>)>,
    focus_bindings: BTreeMap<ElementId, crate::focus::FocusBinding>,
}

impl Pending {
    pub fn patches(&self) -> &[Patch] {
        &self.update.patches
    }
}

// Overlay of changed mounts and staged subscriptions. Unaffected mounts stay
// in App::mounts, avoiding a full-tree clone for each preparation.
struct Preparation {
    changed: BTreeMap<MountId, Rc<Mounted>>,
    removed: HashSet<MountId>,
    subscriptions: Vec<(Observer, Option<Dependencies>)>,
    stats: UpdateStats,
    animations: BTreeMap<ElementId, Transaction>,
}

impl Preparation {
    fn new() -> Self {
        Self {
            changed: BTreeMap::new(),
            removed: HashSet::new(),
            subscriptions: vec![],
            stats: UpdateStats::default(),
            animations: BTreeMap::new(),
        }
    }
}

/// Retained application tree and its reactive graph, independent of any host.
/// Drive updates with prepare, host patch application, and commit in that order.
pub struct App {
    identity: Rc<()>,
    input_revision: u64,
    runtime: Runtime,
    environment: Environment,
    view: AnyView,
    root: Option<MountId>,
    mounts: HashMap<MountId, Rc<Mounted>>,
    observer_mounts: HashMap<AttributeId, MountId>,

    // Record paths to dirty descendants so traversal can skip unaffected siblings.
    dirty: HashSet<MountId>,
    dirty_children: HashMap<MountId, HashSet<MountId>>,

    next_id: u64,
    revision: u64,
    replace_root: bool,
    last_stats: UpdateStats,
    focus: crate::focus::Focus,
}

impl App {
    pub fn new(view: impl View) -> Self {
        Self {
            identity: Rc::new(()),
            input_revision: 0,
            runtime: Runtime::new(),
            environment: Environment::new(),
            view: view.into_any(),
            root: None,
            mounts: HashMap::new(),
            observer_mounts: HashMap::new(),
            dirty: HashSet::new(),
            dirty_children: HashMap::new(),
            next_id: 1,
            revision: 0,
            replace_root: true,
            last_stats: UpdateStats::default(),
            focus: Default::default(),
        }
    }

    pub fn runtime(&self) -> &Runtime {
        &self.runtime
    }

    pub fn runtime_mut(&mut self) -> &mut Runtime {
        &mut self.runtime
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn stats(&self) -> UpdateStats {
        self.last_stats
    }

    pub fn mounted_count(&self) -> usize {
        self.mounts.len()
    }

    /// Replace the root description, preserving mounted state where identity matches.
    pub fn set_view(&mut self, view: impl View) {
        self.input_revision = self
            .input_revision
            .checked_add(1)
            .expect("root revision exhausted");
        self.view = view.into_any();
        self.replace_root = true;
    }

    pub fn set_environment<K: EnvironmentKey>(&mut self, value: K::Value) {
        self.environment.set::<K>(&mut self.runtime, value);
    }

    /// Install an object in the root environment before evaluating views.
    pub fn set_environment_object<T: 'static>(&mut self, value: Rc<T>) {
        self.set_environment::<crate::environment::ObjectKey<T>>(crate::environment::ObjectValue(
            Some(value),
        ));
    }

    /// Batch model writes before the next preparation.
    pub fn mutate<R>(&mut self, action: impl FnOnce(&mut Runtime) -> R) -> R {
        self.runtime.batch(action)
    }

    /// Dispatch to the mounted handler. Disabled targets only receive focus loss.
    pub fn dispatch(&mut self, id: ElementId, event: Event) -> bool {
        self.dispatch_event(id, event).handled
    }

    /// Dispatch synchronously, bubbling keyboard and pointer events through the
    /// mounted tree in one mutation batch. Hosts apply the returned native controls.
    pub fn dispatch_event(&mut self, id: ElementId, event: Event) -> crate::EventResponse {
        let mut response = crate::EventResponse::default();
        let Some(target) = self.mounts.get(&id) else {
            return response;
        };
        // Disabling a focused control still ends its focus session. Suppress
        // user input on disabled controls, but let observers clear focus state.
        let focus_lost = matches!(event, Event::Focus(false) | Event::FocusWithin(false));
        if !target.owner.get()
            || (!focus_lost && target.element.as_ref().is_some_and(|e| e.disabled))
        {
            return response;
        }
        let mut route = Vec::new();
        let mut cursor = Some(id);
        while let Some(current) = cursor {
            let Some(mount) = self.mounts.get(&current) else {
                break;
            };
            if mount.owner.get() && (focus_lost || !mount.element.as_ref().is_some_and(|e| e.disabled)) {
                if let Some(handler) = &mount.handler {
                    route.push((current, handler.clone(), mount.environment.clone()));
                }
            }
            cursor = if event.bubbles() { mount.parent } else { None };
        }
        self.runtime.batch(|runtime| {
            for (current_target, handler, environment) in route {
                response.handled = true;
                handler(
                    &mut ActionContext {
                        runtime,
                        environment: &environment,
                        target: id,
                        current_target,
                        event: &event,
                        response: &mut response,
                    },
                    &event,
                );
                if response.propagation_stopped {
                    break;
                }
            }
        });
        response
    }

    // Mark traversal paths, not body dependencies. Body observers decide whether
    // an ancestor needs evaluation or only a visit to its affected children.
    fn add_dirty(&mut self, id: MountId) {
        let mut cursor = Some(id);
        while let Some(id) = cursor {
            // A previously marked mount already has a path to the root. The
            // child-to-parent edge below is recorded before reaching it, so
            // additional dirty branches still join the existing path.
            if !self.dirty.insert(id) {
                break;
            }
            cursor = self.mounts.get(&id).and_then(|m| m.parent);
            if let Some(parent) = cursor {
                self.dirty_children.entry(parent).or_default().insert(id);
            }
        }
    }

    /// Reconcile affected views and validate patches without publishing the candidate.
    /// Returns None when no root replacement or graph invalidation requires work.
    /// This stages subscriptions and effects, not a rollback of arbitrary model writes.
    pub fn prepare(&mut self) -> Result<Option<Pending>, ViewError> {
        for id in self.runtime.take_dirty_observers() {
            if let Some(mount) = self.observer_mounts.get(&id).copied() {
                self.add_dirty(mount);
            }
        }
        if !self.replace_root && self.dirty.is_empty() {
            return Ok(None);
        }

        let mut tx = Preparation::new();
        let old_root = self.root.filter(|id| {
            self.mounts[id].view.type_id() == self.view.type_id()
                && self.mounts[id].view.element_identity() == self.view.element_identity()
                && self.mounts[id].view.identity_key() == self.view.identity_key()
        });
        if self.root != old_root {
            if let Some(root) = self.root {
                self.remove_tree(root, &AnimationContext::default(), &mut tx);
            }
        }
        let env = self.environment.clone();
        let view = self.view.clone();
        let result = self.build(old_root, None, view, env, self.replace_root, AnimationContext::default(), &mut tx);
        let root = match result {
            Ok(root) => root,
            Err(error) => {
                drop(tx);
                self.runtime.collect();
                return Err(error);
            }
        };
        let patches = match self.patches(root, &tx) {
            Ok(patches) => patches,
            Err(error) => {
                drop(tx);
                self.runtime.collect();
                return Err(error);
            }
        };
        let focus_bindings = match self.prepare_focus(&tx) {
            Ok(bindings) => bindings,
            Err(error) => {
                drop(tx);
                self.runtime.collect();
                return Err(error);
            }
        };
        tx.stats.patches = patches.len();
        Ok(Some(Pending {
            app_identity: self.identity.clone(),
            input_revision: self.input_revision,
            epoch: self.runtime.epoch(),
            update: Update {
                base_revision: self.revision,
                revision: self.revision + 1,
                patches,
                stats: tx.stats,
                animations: tx.animations,
            },
            changed: tx.changed,
            removed: tx.removed,
            root,
            subscriptions: tx.subscriptions,
            focus_bindings,
        }))
    }

    // Candidate values shadow accepted mounts; untouched values stay shared.
    fn get<'a>(&'a self, id: MountId, tx: &'a Preparation) -> &'a Rc<Mounted> {
        tx.changed.get(&id).unwrap_or_else(|| &self.mounts[&id])
    }

    fn remove_tree(&self, id: MountId, context: &AnimationContext, tx: &mut Preparation) {
        if tx.removed.insert(id) {
            let mut context = context.clone();
            if let Some(scope) = &self.mounts[&id].animation_scope {
                if scope.value.is_none() { context.overrides.push(scope.modify.clone()); }
            }
            tx.animations.insert(id, context.resolved());
            for child in self.mounts[&id].children.iter() {
                self.remove_tree(*child, &context, tx);
            }
            tx.stats.removed += 1;
        }
    }

    fn build(
        &mut self,
        old: Option<MountId>,
        parent: Option<MountId>,
        view: AnyView,
        environment: Environment,
        force: bool,
        mut animation_context: AnimationContext,
        tx: &mut Preparation,
    ) -> Result<MountId, ViewError> {
        if let Some(id) = old {
            if !force && !self.dirty.contains(&id) {
                return Ok(id);
            }
        }
        let memo = view.memo_props();
        let reuse_memo = memo.as_ref().zip(old).is_some_and(|(new, id)| {
            let previous = &self.mounts[&id];
            previous.memo.as_ref().is_some_and(|old| new.same(old))
                && environment.same_scope(&previous.environment)
        });
        // Equal memo inputs retain the accepted subtree. When neither this body
        // nor a descendant is dirty, there is no candidate mount or subscription
        // work to stage. A changed provider scope must still reach descendants.
        if let Some(id) = old.filter(|_| reuse_memo) {
            if !self.dirty.contains(&id) && !self.mounts[&id].subtree.is_dirty(&self.runtime) {
                return Ok(id);
            }
        }
        tx.stats.visited += 1;
        let previous = old.map(|id| self.mounts[&id].clone());
        let id = if let Some(id) = old {
            id
        } else {
            let id = ElementId(self.next_id);
            self.next_id = self
                .next_id
                .checked_add(1)
                .expect("element identity exhausted");
            id
        };
        let mut mount = previous
            .as_ref()
            .map(|m| (**m).clone())
            .unwrap_or_else(|| Mounted {
                id,
                parent,
                view: view.clone(),
                environment: environment.clone(),
                element: None,
                handler: None,
                children: Rc::new(vec![]),
                slots: Rc::default(),
                effects: HashMap::new(),
                provider: None,
                memo: None,
                animation_scope: None,
                body: self.runtime.observer(),
                subtree: self.runtime.observer(),
                owner: Rc::new(Cell::new(false)),
            });
        if previous.is_none() {
            tx.stats.created += 1;
        }
        mount.memo = memo;
        mount.view = view.clone();
        mount.environment = environment.clone();
        if let Some(cause) = self.runtime.observer_context(&mount.body) {
            if cause.epoch > animation_context.epoch {
                animation_context.epoch = cause.epoch;
                animation_context.transaction = cause.get::<Transaction>().copied().unwrap_or_default();
            }
        }

        // Memo can skip new parent descriptions, but never a dirty body observer.
        let evaluate =
            (force && !reuse_memo) || mount.body.is_dirty(&self.runtime) || previous.is_none();
        if evaluate {
            tx.stats.evaluated += 1;
            let (result, reads) = self.runtime.capture_mut(|runtime| {
                let mut cx = ViewContext {
                    runtime,
                    environment: &environment,
                    previous_slots: &mount.slots,
                    slots: HashMap::new(),
                    previous_effects: &mount.effects,
                    effects: HashMap::new(),
                    owner: &mount.owner,
                    error: None,
                };
                let description = view.evaluate(&mut cx)?;
                if let Some(error) = cx.error {
                    return Err(error);
                }
                if previous.is_some()
                    && (cx.previous_slots.len() != cx.slots.len()
                        || cx
                            .previous_slots
                            .keys()
                            .any(|name| !cx.slots.contains_key(name)))
                {
                    return Err(ViewError::new(
                        "mounted state declarations must be unconditional and stable",
                    ));
                }
                Ok((description, cx.slots, cx.effects))
            });
            let (description, slots, effects) =
                result.map_err(|e| ViewError::new(format!("view {}: {e}", id.0)))?;
            // Existing declarations were validated above, and remember() reuses
            // their Rc values. Keep the accepted map rather than allocating an
            // identical map on every body evaluation (including empty primitives).
            if previous.is_none() && !slots.is_empty() {
                mount.slots = Rc::new(slots);
            }
            mount.effects = effects;
            tx.subscriptions.push((mount.body.clone(), Some(reads)));
            let (descriptions, child_env) = match description {
                Description::Element(element, children, handler) => {
                    element.validate()?;
                    if element.kind == ElementKind::Text && !children.is_empty() {
                        return Err(ViewError::new("text cannot contain children"));
                    }
                    mount.element = Some(*element);
                    mount.handler = handler;
                    mount.provider = None;
                    (children, environment.clone())
                }
                Description::Memo(_, child) => {
                    mount.element = None;
                    mount.handler = None;
                    mount.provider = None;
                    (vec![child], environment.clone())
                }
                Description::Transaction(scope, child) => {
                    mount.animation_scope = Some(scope);
                    mount.element = None;
                    mount.handler = None;
                    mount.provider = None;
                    (vec![child], environment.clone())
                }
                Description::Group(children) => {
                    mount.element = None;
                    mount.handler = None;
                    mount.provider = None;
                    (children, environment.clone())
                }
                Description::Provider(provider, child) => {
                    let (scope, entry) =
                        provider.apply(&mut self.runtime, &environment, mount.provider.as_ref());
                    mount.provider = Some(entry);
                    mount.element = None;
                    mount.handler = None;
                    (vec![child], scope)
                }
            };

            if let Some(scope) = &mount.animation_scope {
                let active = match &scope.value {
                    None => true,
                    Some(value) => previous.as_ref().and_then(|m| m.animation_scope.as_ref()).and_then(|s| s.value.as_ref()).is_some_and(|old| !value.same(old)),
                };
                if active { animation_context.overrides.push(scope.modify.clone()); }
            }

            // Match siblings by key (or position), type, and host identity.
            // Unmatched old identities are retired only if this candidate commits.
            let mut keys = HashSet::new();
            for child in &descriptions {
                if let Some(key) = child.identity_key() {
                    if !keys.insert(key.clone()) {
                        return Err(ViewError::new(format!("duplicate sibling key: {key}")));
                    }
                }
            }
            // Unchanged sibling prefixes need no lookup table. Keep the keyed
            // fallback for the suffix after an insertion, removal, or reorder.
            let mut children = Vec::with_capacity(descriptions.len());
            let mut descriptions = descriptions.into_iter().enumerate().peekable();
            while let Some((index, child)) = descriptions.peek() {
                let Some(&old) = mount.children.get(*index) else {
                    break;
                };
                if identity(&self.mounts[&old].view, *index) != identity(child, *index) {
                    break;
                }
                let (_, child) = descriptions.next().unwrap();
                children.push(self.build(
                    Some(old),
                    Some(id),
                    child,
                    child_env.clone(),
                    true,
                    animation_context.clone(),
                    tx,
                )?);
            }
            let mut existing: HashMap<Identity, MountId> = mount
                .children
                .iter()
                .enumerate()
                .skip(children.len())
                .map(|(index, id)| (identity(&self.mounts[id].view, index), *id))
                .collect();
            for (index, child) in descriptions {
                let old = existing.remove(&identity(&child, index));
                children.push(self.build(old, Some(id), child, child_env.clone(), true, animation_context.clone(), tx)?);
            }
            for removed in existing.into_values() {
                self.remove_tree(removed, &animation_context, tx);
            }
            mount.children = Rc::new(children);
        } else {
            if let Some(scope) = &mount.animation_scope {
                if scope.value.is_none() { animation_context.overrides.push(scope.modify.clone()); }
            }
            // Keep this body and its child descriptions; visit only dirty paths.
            let dirty = self.dirty_children.get(&id).cloned().unwrap_or_default();
            for child in dirty {
                let m = self.mounts[&child].clone();
                self.build(
                    Some(child),
                    Some(id),
                    m.view.clone(),
                    m.environment.clone(),
                    false,
                    animation_context.clone(),
                    tx,
                )?;
            }
        }

        // Aggregate body and child subscriptions for ancestor invalidation.
        // A skipped body keeps its existing subscription topology.
        let dependencies = if evaluate {
            let mut dependencies = Dependencies::default();
            dependencies.include(&mount.body);
            for child in mount.children.iter() {
                dependencies.include(&self.get(*child, tx).subtree);
            }
            Some(dependencies)
        } else {
            None
        };
        tx.subscriptions.push((mount.subtree.clone(), dependencies));
        if mount.element.is_some() { tx.animations.insert(id, animation_context.resolved()); }
        tx.changed.insert(id, Rc::new(mount));
        Ok(id)
    }

    // Composition-only mounts disappear at the host boundary. Stop flattening
    // when a real element is reached; its descendants belong to that element.
    fn flatten(&self, id: MountId, tx: Option<&Preparation>, out: &mut Vec<ElementId>) {
        let m = match tx {
            Some(tx) => self.get(id, tx),
            None => &self.mounts[&id],
        };
        if m.element.is_some() {
            out.push(id)
        } else {
            for child in m.children.iter() {
                self.flatten(*child, tx, out);
            }
        }
    }

    fn element_children(&self, id: MountId, tx: Option<&Preparation>) -> Vec<ElementId> {
        let m = match tx {
            Some(tx) => self.get(id, tx),
            None => &self.mounts[&id],
        };
        let mut out = vec![];
        for child in m.children.iter() {
            self.flatten(*child, tx, &mut out);
        }
        out
    }

    fn host_parent(&self, id: MountId, tx: &Preparation) -> Option<ElementId> {
        let mut parent = self.get(id, tx).parent;
        while let Some(id) = parent {
            let m = self.get(id, tx);
            if m.element.is_some() {
                return Some(id);
            }
            parent = m.parent;
        }
        None
    }

    // Compare affected descriptions and only recompute child order for parents
    // whose structure changed. A text-only update needs no Children patch.
    fn patches(&self, root: MountId, tx: &Preparation) -> Result<Vec<Patch>, ViewError> {
        let mut patches = vec![];
        let mut parents = HashSet::new();
        for (id, m) in &tx.changed {
            let old = self.mounts.get(id);
            match (&m.element, old.and_then(|m| m.element.as_ref())) {
                (Some(new), None) => {
                    patches.push(Patch::Create(*id, new.clone()));
                    parents.insert(Some(*id));
                    parents.insert(self.host_parent(*id, tx));
                }
                (Some(new), Some(previous)) => {
                    if new.identity() != previous.identity() {
                        return Err(ViewError::new("host identity changed without a new key"));
                    }
                    if new != previous {
                        patches.push(Patch::Update(*id, new.clone()));
                    }
                }
                (None, Some(_)) => {
                    return Err(ViewError::new(
                        "element structure changed without a new key",
                    ))
                }
                _ => {}
            }
            if old.is_none_or(|old| old.children != m.children) {
                parents.insert(if m.element.is_some() {
                    Some(*id)
                } else {
                    self.host_parent(*id, tx)
                });
            }
        }
        if self.root != Some(root) {
            parents.insert(None);
        }
        let mut parents: Vec<_> = parents.into_iter().collect();
        parents.sort();
        for parent in parents {
            let children = if let Some(id) = parent {
                self.element_children(id, Some(tx))
            } else {
                let mut v = vec![];
                self.flatten(root, Some(tx), &mut v);
                v
            };
            let previous = match parent {
                Some(id) if self.mounts.contains_key(&id) => self.element_children(id, None),
                None => {
                    let mut v = vec![];
                    if let Some(id) = self.root {
                        self.flatten(id, None, &mut v);
                    }
                    v
                }
                _ => vec![],
            };
            if children != previous {
                patches.push(Patch::Children(parent, children));
            }
        }
        for id in &tx.removed {
            if self.mounts[id].element.is_some() {
                patches.push(Patch::Remove(*id));
            }
        }
        // Validate HTML-independent control nesting against the affected ancestor paths.
        for (id, m) in &tx.changed {
            if m.element
                .as_ref()
                .is_some_and(|e| e.interaction.is_some() || e.kind == ElementKind::TextInput)
            {
                let mut parent = m.parent;
                while let Some(p) = parent {
                    let ancestor = self.get(p, tx);
                    if ancestor
                        .element
                        .as_ref()
                        .is_some_and(|e| e.interaction == Some(Interaction::Activate))
                    {
                        return Err(ViewError::new(format!(
                            "interactive element {} nested inside another interactive control",
                            id.0
                        )));
                    }
                    parent = ancestor.parent;
                }
            }
        }
        Ok(patches)
    }

    /// Publish only after the adapter successfully applied the entire update.
    /// The candidate must still match this app, root input, model epoch, and revision.
    /// Keep prepare/apply/commit together without intervening model mutations.
    pub fn commit(&mut self, pending: Pending) -> Result<(), ViewError> {
        if !Rc::ptr_eq(&pending.app_identity, &self.identity)
            || pending.input_revision != self.input_revision
            || pending.epoch != self.runtime.epoch()
            || pending.update.base_revision != self.revision
        {
            return Err(ViewError::new("stale commit revision"));
        }

        let mut stop = vec![];
        let mut start = vec![];
        for id in &pending.removed {
            if let Some(old) = self.mounts.remove(id) {
                old.owner.set(false);
                stop.extend(old.effects.values().cloned());
                self.observer_mounts.remove(&old.body.id());
                self.observer_mounts.remove(&old.subtree.id());
            }
        }
        for (id, new) in pending.changed {
            if let Some(old) = self.mounts.get(&id) {
                for (name, effect) in &old.effects {
                    if new
                        .effects
                        .get(name)
                        .is_none_or(|e| !Rc::ptr_eq(&e.cleanup, &effect.cleanup))
                    {
                        stop.push(effect.clone());
                    }
                }
            }
            start.extend(new.effects.values().cloned());
            new.owner.set(true);
            self.observer_mounts.insert(new.body.id(), id);
            self.observer_mounts.insert(new.subtree.id(), id);
            self.mounts.insert(id, new);
        }

        // Publish dependencies only after the host has accepted the candidate.
        for (observer, deps) in pending.subscriptions {
            if let Some(deps) = deps {
                self.runtime.accept(&observer, deps)
            } else {
                self.runtime.mark_clean(&observer)
            };
        }
        self.commit_focus(pending.focus_bindings);
        self.root = Some(pending.root);
        self.revision = pending.update.revision;
        self.last_stats = pending.update.stats;
        self.replace_root = false;
        self.dirty.clear();
        self.dirty_children.clear();

        // Teardown precedes replacement setup. Setup writes are batched and may
        // invalidate the newly accepted tree for a subsequent update.
        for effect in stop {
            effect.stop();
        }
        self.runtime.batch(|rt| {
            for effect in start {
                effect.start(rt)
            }
        });
        self.runtime.collect();
        Ok(())
    }

    /// Snapshot accepted host elements in identity order, primarily for inspection.
    pub fn elements(&self) -> Vec<(ElementId, Element)> {
        let mut result: Vec<_> = self
            .mounts
            .values()
            .filter_map(|m| m.element.clone().map(|e| (m.id, e)))
            .collect();
        result.sort_by_key(|(id, _)| *id);
        result
    }

    /// Release mounted resources, invalidate scopes and candidates, and run cleanup.
    /// The host adapter remains responsible for removing its own native resources.
    pub fn dispose(&mut self) {
        self.identity = Rc::new(());
        self.runtime.clear_waker();
        self.runtime.close_channels();
        for mount in self.mounts.values() {
            mount.owner.set(false);
            for effect in mount.effects.values() {
                effect.stop();
            }
        }
        self.clear_focus_bindings();
        self.mounts.clear();
        self.observer_mounts.clear();
        self.dirty.clear();
        self.dirty_children.clear();
        self.root = None;
        self.view = Empty.into_any();
        self.replace_root = true;
        self.environment = Environment::new();
        self.runtime.collect();
    }
}

impl Drop for App {
    fn drop(&mut self) {
        self.dispose()
    }
}

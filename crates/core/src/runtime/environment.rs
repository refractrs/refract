//! Graph-backed environment values and scoped context providers.

use crate::*;
use std::{
    any::{Any, TypeId},
    cell::RefCell,
    collections::HashMap,
    rc::Rc,
};

/// A typed inherited value with an equality rule and a fallback default.
pub trait EnvironmentKey: 'static {
    /// Equality controls whether providing a value requires a new graph cell.
    type Value: Clone + PartialEq + 'static;

    fn default() -> Self::Value;
}

// Keep the graph cell and the provider value together. The snapshot lets a
// provider compare its next input without reading or mutating the accepted cell.
#[derive(Clone)]
pub(crate) struct Entry {
    pub(crate) cell: Rc<dyn Any>,
    value: Rc<dyn Any>,
}

/// A shared root environment plus inherited overrides for one subtree.
/// Values are stored in graph state so captured reads can subscribe to changes.
#[derive(Clone, Default)]
pub struct Environment {
    root: Rc<RefCell<HashMap<TypeId, Entry>>>,
    overrides: Rc<HashMap<TypeId, Entry>>,
}

impl Environment {
    // Memo reuse requires the same cells, not merely equal current values.
    pub(crate) fn same_scope(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.root, &other.root)
            && self.overrides.len() == other.overrides.len()
            && self.overrides.iter().all(|(key, value)| {
                other
                    .overrides
                    .get(key)
                    .is_some_and(|v| Rc::ptr_eq(&value.cell, &v.cell))
            })
    }

    pub fn new() -> Self {
        Self::default()
    }

    /// Update a root value. Existing subtree overrides continue to take precedence.
    pub fn set<K: EnvironmentKey>(&self, rt: &mut Runtime, value: K::Value) {
        let existing = self.root.borrow().get(&TypeId::of::<K>()).cloned();
        if let Some(entry) = existing {
            entry
                .cell
                .downcast_ref::<State<K::Value>>()
                .unwrap()
                .set_if_changed(rt, value);
        } else {
            self.root.borrow_mut().insert(
                TypeId::of::<K>(),
                Entry {
                    cell: Rc::new(State::new(rt, value.clone())),
                    value: Rc::new(value),
                },
            );
        }
    }

    // Materialize a default cell before a view reads it, so later root writes
    // can invalidate that read even when no value was explicitly provided yet.
    pub(crate) fn ensure<K: EnvironmentKey>(&self, rt: &mut Runtime) {
        if !self.overrides.contains_key(&TypeId::of::<K>())
            && !self.root.borrow().contains_key(&TypeId::of::<K>())
        {
            self.set::<K>(rt, K::default());
        }
    }

    /// Read the nearest provided value and track its graph cell, if present.
    /// An absent key returns its default without allocating a subscription cell.
    pub fn read<K: EnvironmentKey>(&self, rt: &Runtime) -> K::Value {
        let root = self.root.borrow();
        self.overrides
            .get(&TypeId::of::<K>())
            .or_else(|| root.get(&TypeId::of::<K>()))
            .map(|e| {
                e.cell
                    .downcast_ref::<State<K::Value>>()
                    .unwrap()
                    .get(rt)
                    .clone()
            })
            .unwrap_or_else(K::default)
    }

    // Overrides are copied only when this branch extends its inherited scope.
    fn with(&self, key: TypeId, entry: Entry) -> Self {
        let mut child = self.clone();
        Rc::make_mut(&mut child.overrides).insert(key, entry);
        child
    }
}

type ApplyProvider = dyn Fn(&mut Runtime, &Environment, Option<&Entry>) -> (Environment, Entry);

#[doc(hidden)]
#[derive(Clone)]
pub struct Provider(Rc<ApplyProvider>);

impl Provider {
    pub(crate) fn new<K: EnvironmentKey>(value: K::Value) -> Self {
        Self(Rc::new(move |rt, env, old| {
            let entry = if let Some(old) =
                old.filter(|old| old.value.downcast_ref::<K::Value>() == Some(&value))
            {
                old.clone()
            } else {
                // Preparation may be discarded. Allocate a replacement instead
                // of writing into a cell still used by the committed subtree.
                Entry {
                    cell: Rc::new(State::new(rt, value.clone())),
                    value: Rc::new(value.clone()),
                }
            };
            (env.with(TypeId::of::<K>(), entry.clone()), entry)
        }))
    }

    pub(crate) fn apply(
        &self,
        rt: &mut Runtime,
        env: &Environment,
        old: Option<&Entry>,
    ) -> (Environment, Entry) {
        (self.0)(rt, env, old)
    }
}

// Context objects compare by identity. Observable fields inside an object
// should use graph state; replacing the Rc changes the provided object itself.
pub(crate) struct ObjectValue<T>(pub Option<Rc<T>>);

impl<T> Clone for ObjectValue<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T> PartialEq for ObjectValue<T> {
    fn eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (Some(a), Some(b)) => Rc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        }
    }
}

pub(crate) struct ObjectKey<T>(std::marker::PhantomData<T>);

impl<T: 'static> EnvironmentKey for ObjectKey<T> {
    type Value = ObjectValue<T>;
    fn default() -> Self::Value {
        ObjectValue(None)
    }
}

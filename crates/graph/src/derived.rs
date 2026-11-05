//! Cached computations with dynamically tracked dependencies.

use crate::{
    handle::Handle,
    runtime::{Computation, Equal},
    Runtime,
};
use std::rc::Rc;

/// An owning handle to a cached computation with dynamically tracked inputs.
pub struct Derived<T>(Handle<T>);

impl<T> Clone for Derived<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T: 'static> Derived<T> {
    /// Compute immediately and propagate every subsequent recomputation.
    pub fn new(rt: &mut Runtime, f: impl Fn(&Runtime) -> T + 'static) -> Self {
        Self::create(rt, f, None)
    }

    /// Propagate only when the computed value changes according to `PartialEq`.
    /// Dependencies are still replaced when the result compares equal.
    pub fn distinct(rt: &mut Runtime, f: impl Fn(&Runtime) -> T + 'static) -> Self
    where
        T: PartialEq,
    {
        Self::create(
            rt,
            f,
            Some(Rc::new(|a, b| {
                a.downcast_ref::<T>() == b.downcast_ref::<T>()
            })),
        )
    }

    fn create(
        rt: &mut Runtime,
        f: impl Fn(&Runtime) -> T + 'static,
        equal: Option<Rc<Equal>>,
    ) -> Self {
        let (value, deps) = rt.capture(|rt| f(rt));
        let handle = rt.allocate(
            value,
            Some(Computation {
                compute: Rc::new(move |rt| Box::new(f(rt))),
                equal,
            }),
        );
        rt.replace_dependencies(handle.id(), deps);
        Self(handle)
    }

    /// Read the value and record a dependency in the active capture, if any.
    pub fn get<'a>(&self, rt: &'a Runtime) -> &'a T {
        rt.read(self.0.id())
    }
}

//! Mutable state and read/write projections of that state.

use crate::{handle::Handle, AttributeId, Runtime};
use std::{ops::Deref, rc::Rc};

/// An owning handle to mutable graph state.
///
/// Clones refer to the same attribute. The last owner queues its retirement in
/// the originating runtime.
pub struct State<T>(Handle<T>);

impl<T> Clone for State<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T: 'static> State<T> {
    /// Allocate state in this runtime.
    pub fn new(rt: &mut Runtime, value: T) -> Self {
        Self(rt.allocate(value, None))
    }

    pub fn id(&self) -> AttributeId {
        self.0.id()
    }

    /// Read the value and record a dependency in the active capture, if any.
    pub fn get<'a>(&self, rt: &'a Runtime) -> &'a T {
        rt.read(self.id())
    }

    /// Replace the value and invalidate readers, even if the value is equal.
    /// Propagation settles immediately unless a batch is active.
    pub fn set(&self, rt: &mut Runtime, value: T) {
        rt.write(self.id(), value);
    }

    /// Mutate a cloned value and write it back without tracking the initial read.
    pub fn update(&self, rt: &mut Runtime, f: impl FnOnce(&mut T))
    where
        T: Clone,
    {
        let mut value = rt.read_untracked::<T>(self.id()).clone();
        f(&mut value);
        self.set(rt, value);
    }

    /// Skip the write and invalidation when the current value compares equal.
    pub fn set_if_changed(&self, rt: &mut Runtime, value: T)
    where
        T: PartialEq,
    {
        if rt.read_untracked::<T>(self.id()) != &value {
            self.set(rt, value);
        }
    }

    /// Share tracked reads without granting mutation access.
    pub fn read_only(&self) -> Read<T>
    where
        T: Clone,
    {
        Read {
            read: Rc::new(self.clone()),
        }
    }

    /// Create a shared read/write binding that keeps this state alive.
    pub fn binding(&self) -> Binding<T>
    where
        T: Clone,
    {
        Binding::from_state(self.clone())
    }
}

// Keep owned reads available for computed projections that need not implement
// Clone, while letting inspection borrow directly from state storage.
trait ReadBinding<T> {
    fn get(&self, rt: &Runtime) -> T;
    fn with(&self, rt: &Runtime, f: &mut dyn FnMut(&T));
}

impl<T: Clone + 'static> ReadBinding<T> for State<T> {
    fn get(&self, rt: &Runtime) -> T {
        State::get(self, rt).clone()
    }

    fn with(&self, rt: &Runtime, f: &mut dyn FnMut(&T)) {
        f(State::get(self, rt));
    }
}

struct Projection<T, F> {
    source: Read<T>,
    get: F,
}

impl<T: 'static, U, F: Fn(&T) -> U> ReadBinding<U> for Projection<T, F> {
    fn get(&self, rt: &Runtime) -> U {
        self.source.with(rt, &self.get)
    }

    fn with(&self, rt: &Runtime, f: &mut dyn FnMut(&U)) {
        f(&self.get(rt));
    }
}

type WriteBinding<T> = dyn Fn(&mut Runtime, T);

/// A clonable read/write interface to state or a projection of that state.
///
/// Creating or cloning a binding does not read the graph. Reading it within a
/// capture subscribes to the underlying attributes.
pub struct Binding<T> {
    read: Read<T>,
    write: Rc<WriteBinding<T>>,
}

impl<T> Clone for Binding<T> {
    fn clone(&self) -> Self {
        Self {
            read: self.read.clone(),
            write: self.write.clone(),
        }
    }
}

impl<T> Deref for Binding<T> {
    type Target = Read<T>;

    fn deref(&self) -> &Self::Target {
        &self.read
    }
}

impl<T: 'static> Binding<T> {
    fn from_state(state: State<T>) -> Self
    where
        T: Clone,
    {
        Self {
            read: state.read_only(),
            write: Rc::new(move |rt, value| state.set(rt, value)),
        }
    }

    /// Share the reader without exposing writes or allocating another graph node.
    pub fn read_only(&self) -> Read<T> {
        self.read.clone()
    }

    /// Read through the binding, tracking any underlying attribute reads.
    pub fn get(&self, rt: &Runtime) -> T {
        self.read.get(rt)
    }

    /// Inspect a value while tracking its underlying attribute reads.
    /// State-backed bindings borrow storage without cloning the value. Mapped
    /// bindings compute their projection from a borrowed source. The inspected
    /// reference cannot escape the callback; return an owned result instead.
    ///
    /// ```compile_fail
    /// use refract_graph::{Runtime, State};
    /// let mut rt = Runtime::new();
    /// let binding = State::new(&mut rt, String::from("hello")).binding();
    /// let escaped = binding.with(&rt, |value| value.as_str());
    /// println!("{escaped}");
    /// ```
    pub fn with<R>(&self, rt: &Runtime, f: impl FnOnce(&T) -> R) -> R {
        self.read.with(rt, f)
    }

    /// Write through the binding to its underlying state.
    pub fn set(&self, rt: &mut Runtime, value: T) {
        (self.write)(rt, value)
    }

    /// Read, modify, and write a value through the binding.
    /// Unlike [`State::update`], the read participates in active capture tracking.
    pub fn update(&self, rt: &mut Runtime, f: impl FnOnce(&mut T)) {
        let mut v = self.get(rt);
        f(&mut v);
        self.set(rt, v);
    }

    /// Skip the write and invalidation when the current value compares equal.
    pub fn set_if_changed(&self, rt: &mut Runtime, value: T)
    where
        T: PartialEq,
    {
        if self.with(rt, |current| current != &value) {
            self.set(rt, value);
        }
    }

    /// Project a value without allocating another graph attribute.
    /// Reads depend on the source attribute, not on an independently tracked field.
    /// The getter borrows the source; reading does not clone the parent value.
    /// Writes still read, modify, and replace the parent through its binding.
    pub fn map<U: 'static>(
        &self,
        get: impl Fn(&T) -> U + 'static,
        set: impl Fn(&mut T, U) + 'static,
    ) -> Binding<U> {
        let w = self.clone();
        Binding {
            read: Read {
                read: Rc::new(Projection {
                    source: self.read_only(),
                    get,
                }),
            },
            write: Rc::new(move |rt, value| w.update(rt, |parent| set(parent, value))),
        }
    }
}

/// Read-only access to an existing attribute; cloning allocates no graph nodes.
///
/// ```compile_fail
/// use refract_graph::{Runtime, State};
/// let mut rt = Runtime::new();
/// let read = State::new(&mut rt, 10).read_only();
/// read.set(&mut rt, 20);
/// ```
pub struct Read<T> {
    read: Rc<dyn ReadBinding<T>>,
}

impl<T> Clone for Read<T> {
    fn clone(&self) -> Self {
        Self {
            read: self.read.clone(),
        }
    }
}

impl<T: 'static> Read<T> {
    /// Read an owned value, tracking any underlying attribute reads.
    /// State-backed readers clone the stored value; projections compute it.
    pub fn get(&self, rt: &Runtime) -> T {
        self.read.get(rt)
    }

    /// Inspect a value while tracking its underlying attribute reads.
    /// State-backed readers borrow storage without cloning; projections compute
    /// a temporary value. Return an owned result so the reference cannot escape.
    ///
    /// ```compile_fail
    /// use refract_graph::{Runtime, State};
    /// let mut rt = Runtime::new();
    /// let read = State::new(&mut rt, String::from("hello")).read_only();
    /// let escaped = read.with(&rt, |value| value.as_str());
    /// println!("{escaped}");
    /// ```
    pub fn with<R>(&self, rt: &Runtime, f: impl FnOnce(&T) -> R) -> R {
        let mut f = Some(f);
        let mut result = None;
        self.read.with(rt, &mut |value| {
            result = Some(f.take().expect("reader called twice")(value));
        });
        result.expect("reader did not call its callback")
    }
}

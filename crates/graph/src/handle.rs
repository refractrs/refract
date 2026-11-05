//! Attribute identity and shared handle ownership.

use std::{
    cell::RefCell,
    marker::PhantomData,
    rc::{Rc, Weak},
    sync::atomic::{AtomicU64, Ordering},
};

// IDs are unique across runtimes, so a foreign handle cannot alias a local node.
// The atomic provides identity allocation only; graph access is single-threaded.
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Opaque identity of an attribute in its originating runtime.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct AttributeId(u64);

impl AttributeId {
    // The replacement, `try_update`, requires Rust 1.95; our minimum is 1.93.
    #[allow(deprecated)]
    pub(crate) fn new() -> Self {
        Self(
            NEXT_ID
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
                .expect("attribute identity exhausted"),
        )
    }
}

// Handle lifetime is separate from attribute storage. Drop only queues an ID;
// it never borrows the runtime or mutates graph edges during another operation.
struct Lease {
    id: AttributeId,
    retired: Weak<RefCell<Vec<AttributeId>>>,
}

impl Drop for Lease {
    fn drop(&mut self) {
        if let Some(queue) = self.retired.upgrade() {
            queue.borrow_mut().push(self.id);
        }
    }
}

pub(crate) struct Handle<T> {
    lease: Rc<Lease>,
    marker: PhantomData<T>,
}

impl<T> Clone for Handle<T> {
    fn clone(&self) -> Self {
        Self {
            lease: self.lease.clone(),
            marker: PhantomData,
        }
    }
}

impl<T> Handle<T> {
    pub(crate) fn new(id: AttributeId, retired: Weak<RefCell<Vec<AttributeId>>>) -> Self {
        Self {
            lease: Rc::new(Lease { id, retired }),
            marker: PhantomData,
        }
    }

    pub(crate) fn id(&self) -> AttributeId {
        self.lease.id
    }
}

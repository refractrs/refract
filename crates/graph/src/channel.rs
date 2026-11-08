//! Owned messages cross into the graph; callbacks and graph handles stay local.

use crate::Runtime;
use std::{
    any::Any,
    cell::RefCell,
    collections::{HashMap, HashSet, VecDeque},
    fmt,
    marker::PhantomData,
    rc::{Rc, Weak},
};

#[cfg(not(target_arch = "wasm32"))]
use std::sync::{Arc, Mutex};

const CAPACITY: usize = 1024;
const DRAIN_BUDGET: usize = 128;
type Payload = Box<dyn Any + Send>;
type Callback = Rc<RefCell<Box<dyn FnMut(&mut Runtime, Payload)>>>;
type Registry = Rc<RefCell<HashMap<u64, Callback>>>;

#[cfg(not(target_arch = "wasm32"))]
type Shared = Arc<Mutex<Inbox>>;
#[cfg(not(target_arch = "wasm32"))]
type WeakInbox = std::sync::Weak<Mutex<Inbox>>;
#[cfg(not(target_arch = "wasm32"))]
type Wake = Arc<dyn Fn() + Send + Sync>;
#[cfg(target_arch = "wasm32")]
type Shared = Rc<RefCell<Inbox>>;
#[cfg(target_arch = "wasm32")]
type WeakInbox = Weak<RefCell<Inbox>>;
#[cfg(target_arch = "wasm32")]
type Wake = Rc<dyn Fn()>;

#[derive(Default)]
struct Inbox {
    queue: VecDeque<(u64, Payload)>,
    active: HashSet<u64>,
    wake: Option<Wake>,
}

// These closures only manipulate inbox metadata. User callbacks, notifications,
// and destruction of user-owned payloads/captures happen after releasing it.
fn with_inbox<R>(shared: &Shared, f: impl FnOnce(&mut Inbox) -> R) -> R {
    #[cfg(not(target_arch = "wasm32"))]
    {
        f(&mut shared.lock().expect("message inbox poisoned"))
    }
    #[cfg(target_arch = "wasm32")]
    {
        f(&mut shared.borrow_mut())
    }
}

fn downgrade(shared: &Shared) -> WeakInbox {
    #[cfg(not(target_arch = "wasm32"))]
    {
        Arc::downgrade(shared)
    }
    #[cfg(target_arch = "wasm32")]
    {
        Rc::downgrade(shared)
    }
}

fn notify(wake: Option<Wake>) {
    if let Some(wake) = wake {
        wake();
    }
}

/// Nonblocking admission failure, retaining the original message for the caller.
#[derive(Debug, PartialEq, Eq)]
pub enum SendError<T> {
    /// The runtime's shared inbox has 1,024 pending messages. Retry later or
    /// throttle the producer; never spin or block the UI thread waiting for space.
    Full(T),
    /// The subscription or runtime has closed. No further delivery is possible.
    Closed(T),
}

impl<T> fmt::Display for SendError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Full(_) => "message inbox is full",
            Self::Closed(_) => "message channel is closed",
        })
    }
}

impl<T: fmt::Debug> std::error::Error for SendError<T> {}

/// A weak, clonable endpoint for delivering owned results to a UI-local callback.
///
/// On native targets this is `Send + Sync`; on WASM it stays on the browser's
/// main thread. Worker results must cross `postMessage` first, then be decoded
/// and submitted here. Sending never accesses graph state or runs the callback.
pub struct MessageSender<T> {
    inbox: WeakInbox,
    id: u64,
    marker: PhantomData<fn(T)>,
}

impl<T> Clone for MessageSender<T> {
    fn clone(&self) -> Self {
        Self {
            inbox: self.inbox.clone(),
            id: self.id,
            marker: PhantomData,
        }
    }
}

impl<T: Send + 'static> MessageSender<T> {
    /// Admit a message without waiting for space. Success means queued, not
    /// applied or painted. Closing the subscription discards accepted messages
    /// that have not executed. Messages are FIFO in admission order across all
    /// channels in this runtime, but not relative to host input/update queues.
    pub fn try_send(&self, value: T) -> Result<(), SendError<T>> {
        let Some(inbox) = self.inbox.upgrade() else {
            return Err(SendError::Closed(value));
        };
        let wake = with_inbox(&inbox, |inbox| {
            if !inbox.active.contains(&self.id) {
                return Err(SendError::Closed(value));
            }
            if inbox.queue.len() >= CAPACITY {
                return Err(SendError::Full(value));
            }
            let empty = inbox.queue.is_empty();
            inbox.queue.push_back((self.id, Box::new(value)));
            Ok(if empty { inbox.wake.clone() } else { None })
        })?;
        notify(wake);
        Ok(())
    }
}

/// Keeps a receive callback registered on the UI thread. Drop to close delivery
/// and release the callback's captures. This does not cancel the producer itself.
/// Return [`Self::into_cleanup`] from effect setup so replacement/unmount also
/// rejects late results from the previous operation.
#[must_use = "dropping the subscription immediately closes the channel"]
pub struct MessageSubscription {
    inbox: WeakInbox,
    callbacks: Weak<RefCell<HashMap<u64, Callback>>>,
    id: u64,
}

impl MessageSubscription {
    /// Transfer this subscription into a cleanup callback suitable for returning
    /// from effect setup. Delivery stays open until the callback runs or is
    /// dropped. This closes delivery only; cancel the producer separately.
    #[must_use = "return or retain the cleanup callback to keep the channel open"]
    pub fn into_cleanup(self) -> Box<dyn FnOnce()> {
        Box::new(move || drop(self))
    }
}

impl Drop for MessageSubscription {
    fn drop(&mut self) {
        if let Some(inbox) = self.inbox.upgrade() {
            let discarded = with_inbox(&inbox, |inbox| {
                inbox.active.remove(&self.id);
                let mut discarded = Vec::new();
                let mut retained = VecDeque::new();
                for message in inbox.queue.drain(..) {
                    if message.0 == self.id {
                        discarded.push(message);
                    } else {
                        retained.push_back(message);
                    }
                }
                inbox.queue = retained;
                discarded
            });
            drop(discarded);
        }
        if let Some(callbacks) = self.callbacks.upgrade() {
            let callback = callbacks.borrow_mut().remove(&self.id);
            drop(callback);
        }
    }
}

#[derive(Default)]
pub(crate) struct Channels {
    inbox: Shared,
    callbacks: Registry,
    next_id: u64,
    draining: bool,
}

impl Drop for Channels {
    fn drop(&mut self) {
        // Close admission before dropping user values, even if another thread
        // has temporarily upgraded the inbox's weak reference.
        let (messages, wake) = with_inbox(&self.inbox, |inbox| {
            inbox.active.clear();
            (std::mem::take(&mut inbox.queue), inbox.wake.take())
        });
        let callbacks = std::mem::take(&mut *self.callbacks.borrow_mut());
        drop((messages, wake, callbacks));
    }
}

impl Runtime {
    /// Register a typed result callback without moving its captures off the UI
    /// thread. Keep the subscription alive until delivery is no longer wanted.
    /// Create view-owned channels in committed effect setup, not view evaluation.
    ///
    /// ```
    /// use refract_graph::{Runtime, State};
    /// let mut rt = Runtime::new();
    /// let state = State::new(&mut rt, String::new());
    /// let binding = state.binding();
    /// let (sender, subscription) = rt.channel(move |rt, value: String| {
    ///     binding.set_if_changed(rt, value);
    /// });
    /// std::thread::spawn(move || sender.try_send("loaded".into()).unwrap())
    ///     .join().unwrap();
    /// rt.drain_messages(); // Host adapters do this before preparing views.
    /// assert_eq!(state.get(&rt), "loaded");
    /// drop(subscription);
    /// ```
    pub fn channel<T: Send + 'static>(
        &mut self,
        mut receive: impl FnMut(&mut Runtime, T) + 'static,
    ) -> (MessageSender<T>, MessageSubscription) {
        let channels = &mut self.channels;
        let id = channels.next_id;
        channels.next_id = id.checked_add(1).expect("channel identity exhausted");
        let callback: Callback = Rc::new(RefCell::new(Box::new(move |rt, value| {
            receive(
                rt,
                *value
                    .downcast::<T>()
                    .expect("channel payload type mismatch"),
            );
        })));
        channels.callbacks.borrow_mut().insert(id, callback);
        with_inbox(&channels.inbox, |inbox| {
            inbox.active.insert(id);
        });
        (
            MessageSender {
                inbox: downgrade(&channels.inbox),
                id,
                marker: PhantomData,
            },
            MessageSubscription {
                inbox: downgrade(&channels.inbox),
                callbacks: Rc::downgrade(&channels.callbacks),
                id,
            },
        )
    }

    /// Install the native host's message notification. It may run on a producer
    /// thread and must schedule a later UI flush, never execute graph work inline.
    /// Pending messages trigger a notification immediately upon installation.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn set_message_waker(&mut self, wake: impl Fn() + Send + Sync + 'static) {
        self.install_message_waker(Arc::new(wake));
    }

    /// Install the browser host's notification, which must schedule a later flush.
    /// Pending messages trigger a notification immediately upon installation.
    #[cfg(target_arch = "wasm32")]
    pub fn set_message_waker(&mut self, wake: impl Fn() + 'static) {
        self.install_message_waker(Rc::new(wake));
    }

    fn install_message_waker(&mut self, wake: Wake) {
        let (old, pending) = with_inbox(&self.channels.inbox, |inbox| {
            (inbox.wake.replace(wake.clone()), !inbox.queue.is_empty())
        });
        drop(old);
        if pending {
            wake();
        }
    }

    /// Whether messages remain queued, for manual hosts and deterministic tests.
    pub fn has_pending_messages(&self) -> bool {
        with_inbox(&self.channels.inbox, |inbox| !inbox.queue.is_empty())
    }

    /// Deliver a snapshot of at most 128 messages in one graph mutation batch.
    /// Returns the number of callbacks invoked. Nested drains do nothing; messages
    /// sent during delivery wait for a later drain. Remaining work re-notifies the
    /// host so continuous producers still yield to input and rendering.
    ///
    /// Like `batch`, callback panics do not roll back writes. Undelivered messages
    /// in the extracted snapshot are discarded on unwind; later queued work stays.
    pub fn drain_messages(&mut self) -> usize {
        if self.channels.draining {
            return 0;
        }
        let messages: Vec<_> = with_inbox(&self.channels.inbox, |inbox| {
            let count = inbox.queue.len().min(DRAIN_BUDGET);
            inbox.queue.drain(..count).collect()
        });
        self.channels.draining = true;
        // Closing/reopening channels during a callback must not route an old
        // snapshot into a new registration with a reused numeric identity.
        let callbacks = self.channels.callbacks.clone();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.batch(|rt| {
                let mut delivered = 0;
                for (id, value) in messages {
                    let callback = callbacks.borrow().get(&id).cloned();
                    if let Some(callback) = callback {
                        (callback.borrow_mut())(rt, value);
                        delivered += 1;
                    }
                }
                delivered
            })
        }));
        self.channels.draining = false;
        let wake = with_inbox(&self.channels.inbox, |inbox| {
            if inbox.queue.is_empty() {
                None
            } else {
                inbox.wake.clone()
            }
        });
        notify(wake);
        match result {
            Ok(delivered) => delivered,
            Err(error) => std::panic::resume_unwind(error),
        }
    }

    /// Close every channel, discard pending messages, release callback captures,
    /// and remove the message waker. Hosts call this when disposing their app.
    /// New channels can subsequently be created, with a new host notification.
    pub fn close_channels(&mut self) {
        // Keep recursive drains suppressed even when called inside a callback.
        let draining = self.channels.draining;
        let old = std::mem::take(&mut self.channels);
        self.channels.draining = draining;
        drop(old);
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::{Derived, State};
    use std::{
        cell::Cell,
        sync::atomic::{AtomicUsize, Ordering},
        time::Duration,
    };

    #[test]
    fn worker_results_batch_graph_propagation_and_preserve_equal_writes() {
        fn send_sync<T: Send + Sync>() {}
        send_sync::<MessageSender<String>>();
        let mut rt = Runtime::new();
        let value = State::new(&mut rt, 0);
        let derived = Derived::new(&mut rt, {
            let value = value.clone();
            move |rt| *value.get(rt) * 2
        });
        let observer = rt.observer();
        let (_, deps) = rt.capture(|rt| *derived.get(rt));
        rt.accept(&observer, deps);
        let owner = std::thread::current().id();
        let (sender, _subscription) = rt.channel({
            let binding = value.binding();
            move |rt, result| {
                assert_eq!(std::thread::current().id(), owner);
                binding.set_if_changed(rt, result);
            }
        });
        std::thread::spawn({
            let sender = sender.clone();
            move || {
                sender.try_send(1).unwrap();
                sender.try_send(2).unwrap();
            }
        })
        .join()
        .unwrap();
        assert_eq!(*value.get(&rt), 0);
        assert!(!observer.is_dirty(&rt));
        assert_eq!(rt.drain_messages(), 2);
        assert_eq!(*derived.get(&rt), 4);
        assert_eq!(rt.stats().computations, 1);
        assert!(observer.is_dirty(&rt));
        rt.mark_clean(&observer);
        sender.try_send(2).unwrap();
        rt.drain_messages();
        assert!(!observer.is_dirty(&rt));
        assert_eq!(rt.stats().computations, 1);
    }

    #[test]
    fn capacity_is_shared_and_close_returns_values_and_releases_captures() {
        let mut rt = Runtime::new();
        let captured = Rc::new(());
        let weak = Rc::downgrade(&captured);
        let (sender, subscription) = rt.channel(move |_, _: usize| {
            let _ = &captured;
        });
        let (other, _other_subscription) = rt.channel(|_, _: usize| {});
        for n in 0..CAPACITY {
            sender.try_send(n).unwrap();
        }
        assert_eq!(other.try_send(9), Err(SendError::Full(9)));
        drop(subscription);
        assert!(weak.upgrade().is_none());
        assert_eq!(sender.try_send(10), Err(SendError::Closed(10)));
        assert!(!rt.has_pending_messages());
        other.try_send(11).unwrap();
        drop(rt);
        assert_eq!(other.try_send(12), Err(SendError::Closed(12)));
    }

    #[test]
    fn bounded_fifo_drains_rearm_and_waker_installation_observes_pending_work() {
        let mut rt = Runtime::new();
        let values = Rc::new(RefCell::new(Vec::new()));
        let (a, _a) = rt.channel({
            let values = values.clone();
            move |_, v| values.borrow_mut().push(v)
        });
        let (b, _b) = rt.channel({
            let values = values.clone();
            move |_, v| values.borrow_mut().push(v)
        });
        for n in 0..300 {
            if n % 2 == 0 {
                a.try_send(n)
            } else {
                b.try_send(n)
            }
            .unwrap();
        }
        let wakes = Arc::new(AtomicUsize::new(0));
        rt.set_message_waker({
            let wakes = wakes.clone();
            move || {
                wakes.fetch_add(1, Ordering::Relaxed);
            }
        });
        assert_eq!(wakes.load(Ordering::Relaxed), 1);
        assert_eq!(rt.drain_messages(), 128);
        assert_eq!(wakes.load(Ordering::Relaxed), 2);
        assert_eq!(rt.drain_messages(), 128);
        assert_eq!(rt.drain_messages(), 44);
        assert_eq!(*values.borrow(), (0..300).collect::<Vec<_>>());
        assert!(!rt.has_pending_messages());
        a.try_send(300).unwrap();
        assert_eq!(wakes.load(Ordering::Relaxed), 4);
    }

    #[test]
    fn callbacks_can_send_and_close_subscriptions_without_reentrant_delivery() {
        let mut rt = Runtime::new();
        let delivered = Rc::new(Cell::new(0));
        let (next, subscription) = rt.channel({
            let delivered = delivered.clone();
            move |_, ()| delivered.set(delivered.get() + 1)
        });
        let mut subscription = Some(subscription);
        let (first, _first) = rt.channel(move |rt, close| {
            if close {
                subscription.take();
            } else {
                next.try_send(()).unwrap();
            }
            assert_eq!(rt.drain_messages(), 0);
        });
        first.try_send(false).unwrap();
        assert_eq!(rt.drain_messages(), 1);
        assert_eq!(delivered.get(), 0);
        rt.drain_messages();
        assert_eq!(delivered.get(), 1);
        // Both land in the same snapshot: closing the second registration must
        // suppress its queued callback, not just messages still in the inbox.
        first.try_send(false).unwrap();
        first.try_send(true).unwrap();
        rt.drain_messages();
        rt.drain_messages();
        assert_eq!(delivered.get(), 1);
    }

    #[test]
    fn close_and_reopen_during_delivery_cannot_retarget_an_old_snapshot() {
        let mut rt = Runtime::new();
        let retained = Rc::new(RefCell::new(Vec::new()));
        let (first, _first) = rt.channel({
            let retained = retained.clone();
            move |rt, ()| {
                rt.close_channels();
                for _ in 0..2 {
                    let (_, guard) = rt.channel(|_, _: String| panic!("old message retargeted"));
                    retained.borrow_mut().push(guard);
                }
                assert_eq!(rt.drain_messages(), 0);
            }
        });
        let (second, _second) = rt.channel(|_, ()| panic!("closed snapshot delivered"));
        first.try_send(()).unwrap();
        second.try_send(()).unwrap();
        assert_eq!(rt.drain_messages(), 1);
        assert_eq!(first.try_send(()), Err(SendError::Closed(())));
    }

    #[test]
    fn closing_later_subscription_in_the_same_snapshot_discards_its_message() {
        let mut rt = Runtime::new();
        let guard = Rc::new(RefCell::new(None));
        let (close, _close) = rt.channel({
            let guard = guard.clone();
            move |_, ()| {
                guard.borrow_mut().take();
            }
        });
        let (sender, subscription) = rt.channel(|_, ()| panic!("closed callback executed"));
        *guard.borrow_mut() = Some(subscription);
        close.try_send(()).unwrap();
        sender.try_send(()).unwrap();
        assert_eq!(rt.drain_messages(), 1);
    }

    #[test]
    fn concurrent_producers_do_not_lose_wakes_or_messages() {
        let mut rt = Runtime::new();
        let values = Rc::new(RefCell::new(Vec::new()));
        let (sender, _subscription) = rt.channel({
            let values = values.clone();
            move |_, v| values.borrow_mut().push(v)
        });
        let (wake, notified) = std::sync::mpsc::channel();
        rt.set_message_waker(move || {
            wake.send(()).unwrap();
        });
        let workers: Vec<_> = (0..4)
            .map(|worker| {
                let sender = sender.clone();
                std::thread::spawn(move || {
                    for n in 0..500 {
                        let mut value = worker * 500 + n;
                        loop {
                            match sender.try_send(value) {
                                Ok(()) => break,
                                Err(SendError::Full(v)) => {
                                    value = v;
                                    std::thread::yield_now();
                                }
                                Err(SendError::Closed(_)) => panic!("unexpected closure"),
                            }
                        }
                    }
                })
            })
            .collect();
        while values.borrow().len() < 2000 {
            notified
                .recv_timeout(Duration::from_secs(5))
                .expect("lost wake");
            rt.drain_messages();
        }
        for worker in workers {
            worker.join().unwrap();
        }
        let mut values = values.borrow_mut();
        values.sort_unstable();
        assert_eq!(*values, (0..2000).collect::<Vec<_>>());
    }

    #[test]
    fn callback_panic_restores_drain_and_batch_state() {
        let mut rt = Runtime::new();
        let (sender, _subscription) = rt.channel(|_, fail| {
            if fail {
                panic!("callback failed");
            }
        });
        sender.try_send(true).unwrap();
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| rt.drain_messages())).is_err()
        );
        sender.try_send(false).unwrap();
        assert_eq!(rt.drain_messages(), 1);
    }
}

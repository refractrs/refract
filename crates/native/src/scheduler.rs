//! UI-thread mutation admission and cancellable timers. No executor or worker
//! thread is needed to drive a clock; the event loop sleeps until the next deadline.

use refract_core::{Runtime, ScopeToken};
use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    rc::{Rc, Weak},
    time::{Duration, Instant},
};

const QUEUE_CAPACITY: usize = 1024;
const DRAIN_BUDGET: usize = 128;
type Action = Box<dyn FnOnce(&mut Runtime)>;
type Tick = Rc<dyn Fn(&mut Runtime)>;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ScheduleError {
    #[error("native app is disposed")]
    Disposed,
    #[error("view scope is disposed")]
    ScopeDisposed,
    #[error("native mutation queue is full")]
    Full,
    #[error("timer duration must be positive and fit a monotonic deadline")]
    InvalidDuration,
}

struct Timer {
    scope: ScopeToken,
    period: Duration,
    next: Instant,
    callback: Tick,
}

struct Shared {
    queue: VecDeque<(Option<ScopeToken>, Action)>,
    timers: HashMap<u64, Timer>,
    next_id: u64,
    wake: Rc<dyn Fn()>,
}

/// A weak, UI-thread handle. Admission succeeds before the mutation executes.
/// It may be captured by mounted effects without keeping a disposed app alive.
#[derive(Clone)]
pub struct NativeHandle(Weak<RefCell<Shared>>);

/// Cancels its timer on drop. Return it from an effect's cleanup closure.
pub struct TimerGuard {
    shared: Weak<RefCell<Shared>>,
    id: u64,
}

impl Drop for TimerGuard {
    fn drop(&mut self) {
        if let Some(shared) = self.shared.upgrade() {
            let wake = {
                let mut shared = shared.borrow_mut();
                shared.timers.remove(&self.id);
                shared.wake.clone()
            };
            wake();
        }
    }
}

impl NativeHandle {
    /// Admit a mutation for the next event-loop flush. This handle is UI-thread only.
    pub fn update(&self, action: impl FnOnce(&mut Runtime) + 'static) -> Result<(), ScheduleError> {
        self.enqueue(None, Box::new(action))
    }

    /// Admit a mutation that is discarded if its mounted scope dies before execution.
    pub fn update_scoped(
        &self,
        scope: ScopeToken,
        action: impl FnOnce(&mut Runtime) + 'static,
    ) -> Result<(), ScheduleError> {
        self.enqueue(Some(scope), Box::new(action))
    }

    fn enqueue(&self, scope: Option<ScopeToken>, action: Action) -> Result<(), ScheduleError> {
        let shared = self.0.upgrade().ok_or(ScheduleError::Disposed)?;
        let wake = {
            let mut shared = shared.borrow_mut();
            if scope.as_ref().is_some_and(|scope| !scope.is_alive()) {
                return Err(ScheduleError::ScopeDisposed);
            }
            if shared.queue.len() >= QUEUE_CAPACITY {
                return Err(ScheduleError::Full);
            }
            shared.queue.push_back((scope, action));
            shared.wake.clone()
        };
        wake();
        Ok(())
    }

    /// Start a repeating timer. Its callback runs in a graph mutation batch on the
    /// UI thread; missed periods are coalesced rather than replayed after sleep.
    pub fn interval(
        &self,
        period: Duration,
        scope: ScopeToken,
        callback: impl Fn(&mut Runtime) + 'static,
    ) -> Result<TimerGuard, ScheduleError> {
        let next = Instant::now()
            .checked_add(period)
            .filter(|_| !period.is_zero())
            .ok_or(ScheduleError::InvalidDuration)?;
        if !scope.is_alive() {
            return Err(ScheduleError::ScopeDisposed);
        }
        let shared = self.0.upgrade().ok_or(ScheduleError::Disposed)?;
        let (id, wake) = {
            let mut state = shared.borrow_mut();
            let id = state.next_id;
            state.next_id = id.checked_add(1).expect("timer identity exhausted");
            state.timers.insert(
                id,
                Timer {
                    scope,
                    period,
                    next,
                    callback: Rc::new(callback),
                },
            );
            (id, state.wake.clone())
        };
        wake();
        Ok(TimerGuard {
            shared: Rc::downgrade(&shared),
            id,
        })
    }
}

pub(crate) struct Scheduler(Rc<RefCell<Shared>>);

impl Scheduler {
    pub fn new(wake: impl Fn() + 'static) -> Self {
        Self(Rc::new(RefCell::new(Shared {
            queue: VecDeque::new(),
            timers: HashMap::new(),
            next_id: 1,
            wake: Rc::new(wake),
        })))
    }

    pub fn handle(&self) -> NativeHandle {
        NativeHandle(Rc::downgrade(&self.0))
    }

    pub fn drain(&self, rt: &mut Runtime, now: Instant) {
        // Never hold a RefCell borrow while calling app code. A callback may
        // schedule another update or drop its own timer's cleanup guard.
        for _ in 0..DRAIN_BUDGET {
            let command = self.0.borrow_mut().queue.pop_front();
            let Some((scope, action)) = command else {
                break;
            };
            if scope.is_none_or(|scope| scope.is_alive()) {
                action(rt);
            }
        }
        let due: Vec<_> = {
            let mut shared = self.0.borrow_mut();
            shared.timers.retain(|_, timer| timer.scope.is_alive());
            shared
                .timers
                .iter_mut()
                .filter_map(|(id, timer)| {
                    if timer.next > now {
                        return None;
                    }
                    timer.next = now.checked_add(timer.period).unwrap_or(now);
                    Some((*id, timer.callback.clone(), timer.scope.clone()))
                })
                .collect()
        };
        for (id, callback, scope) in due {
            if scope.is_alive() && self.0.borrow().timers.contains_key(&id) {
                callback(rt);
            }
        }
    }

    pub fn deadline(&self) -> Option<Instant> {
        let shared = self.0.borrow();
        if !shared.queue.is_empty() {
            return Some(Instant::now());
        }
        shared.timers.values().map(|timer| timer.next).min()
    }
}

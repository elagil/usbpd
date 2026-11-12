//! State shared between a mock and its handle, with wakeups on change.

use std::pin::pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use tokio::sync::Notify;

/// State behind a mutex, plus a notification that wakes every waiter when the state changes.
#[derive(Debug, Default)]
pub(crate) struct Shared<S> {
    state: Mutex<S>,
    notify: Notify,
}

impl<S> Shared<S> {
    pub(crate) fn new(state: S) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(state),
            notify: Notify::new(),
        })
    }

    /// Locks the state for reading or for changes that must not wake waiters.
    ///
    /// Ignores poisoning: a panicking test thread must not cascade into unrelated panics.
    pub(crate) fn lock(&self) -> MutexGuard<'_, S> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Changes the state and wakes every waiter, which then re-checks its condition.
    pub(crate) fn update<R>(&self, f: impl FnOnce(&mut S) -> R) -> R {
        let result = f(&mut self.lock());
        self.notify.notify_waiters();
        result
    }

    /// Waits until `check` returns `Some`, re-checking after every [`Shared::update`].
    ///
    /// Checks immediately first. Cancel-safe: `check` only runs inside a poll, so whatever it
    /// consumes is returned by that same poll.
    pub(crate) async fn wait_for<R>(&self, mut check: impl FnMut(&mut S) -> Option<R>) -> R {
        loop {
            // Register for the next notification before checking, so that an update between the
            // check and the await isn't lost.
            let mut notified = pin!(self.notify.notified());
            notified.as_mut().enable();
            let result = check(&mut self.lock());
            if let Some(result) = result {
                return result;
            }
            notified.await;
        }
    }
}

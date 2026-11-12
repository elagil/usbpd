//! Mock of the CC front end.

use std::future::Future;
use std::sync::Arc;

use usbcc_traits::{CcPhy, CcPin, CcState, Termination};

use crate::SimError;
use crate::log::{CcCall, Log};
use crate::shared::Shared;

#[derive(Debug)]
struct CcInner {
    /// What the pins see, as set by the test.
    cc: (CcState, CcState),
    /// Incremented on every change of `cc`, and on every requested spurious wakeup.
    cc_version: u64,
    /// What the port presents, as set through the trait.
    terminations: (Termination, Termination),
    /// Incremented on every call of `set_termination`.
    termination_version: u64,
    fail_next: Option<SimError>,
}

/// The [`CcPhy`] side of the CC mock, handed to the port.
///
/// Reports the [`CcState`]s set through its [`CcHandle`] verbatim, whatever terminations the port
/// presents: keeping the two consistent is up to the test (or, later, a cable model).
///
/// [`CcPhy::wait_for_cc_change`] returns once the state set through the handle differs from the
/// state at the last read by [`CcPhy::cc_state`] or [`CcPhy::wait_for_cc_change`]. This includes
/// changes that were reverted before the read (A → B → A), like an edge-triggered interrupt. The
/// initial state, both pins [`CcState::Open`], counts as read. It never returns spuriously unless
/// the test asks for it with [`CcHandle::trigger_spurious_change`].
#[derive(Debug)]
pub struct MockCcPhy {
    shared: Arc<Shared<CcInner>>,
    log: Log,
    port: &'static str,
    /// `cc_version` at the last read.
    seen: u64,
}

/// The test side of the CC mock. Cloning yields another handle to the same mock.
#[derive(Debug, Clone)]
pub struct CcHandle {
    shared: Arc<Shared<CcInner>>,
}

impl MockCcPhy {
    /// Creates a CC mock that records its calls in `log`, labelled with `port`.
    ///
    /// Both pins start [`CcState::Open`], with [`Termination::Open`] presented.
    pub fn new(log: &Log, port: &'static str) -> (Self, CcHandle) {
        let shared = Shared::new(CcInner {
            cc: (CcState::Open, CcState::Open),
            cc_version: 0,
            terminations: (Termination::Open, Termination::Open),
            termination_version: 0,
            fail_next: None,
        });
        let mock = Self {
            shared: shared.clone(),
            log: log.clone(),
            port,
            seen: 0,
        };
        (mock, CcHandle { shared })
    }

    /// Consumes an injected error.
    fn check_error(&self) -> Result<(), SimError> {
        self.shared.lock().fail_next.take().map_or(Ok(()), Err)
    }
}

impl CcPhy for MockCcPhy {
    type Error = SimError;

    fn set_termination(&mut self, cc1: Termination, cc2: Termination) -> impl Future<Output = Result<(), Self::Error>> {
        self.log.record(self.port, CcCall::SetTermination { cc1, cc2 });
        async move {
            self.check_error()?;
            self.shared.update(|inner| {
                inner.terminations = (cc1, cc2);
                inner.termination_version += 1;
            });
            Ok(())
        }
    }

    fn cc_state(&mut self) -> impl Future<Output = Result<(CcState, CcState), Self::Error>> {
        self.log.record(self.port, CcCall::CcState);
        async move {
            self.check_error()?;
            let inner = self.shared.lock();
            self.seen = inner.cc_version;
            Ok(inner.cc)
        }
    }

    fn wait_for_cc_change(&mut self) -> impl Future<Output = Result<(CcState, CcState), Self::Error>> {
        self.log.record(self.port, CcCall::WaitForCcChange);
        async move {
            self.check_error()?;
            let seen = &mut self.seen;
            let state = self
                .shared
                .wait_for(|inner| {
                    (inner.cc_version != *seen).then(|| {
                        *seen = inner.cc_version;
                        inner.cc
                    })
                })
                .await;
            Ok(state)
        }
    }
}

impl CcHandle {
    /// Sets what CC1 and CC2 see, and wakes the port if that is a change.
    pub fn set_cc(&self, cc1: CcState, cc2: CcState) {
        self.shared.update(|inner| {
            if inner.cc != (cc1, cc2) {
                inner.cc = (cc1, cc2);
                inner.cc_version += 1;
            }
        });
    }

    /// Sets what one pin sees, leaving the other unchanged.
    pub fn set_pin(&self, pin: CcPin, state: CcState) {
        let (cc1, cc2) = self.cc();
        match pin {
            CcPin::Cc1 => self.set_cc(state, cc2),
            CcPin::Cc2 => self.set_cc(cc1, state),
        }
    }

    /// What CC1 and CC2 currently see.
    pub fn cc(&self) -> (CcState, CcState) {
        self.shared.lock().cc
    }

    /// The terminations that the port currently presents on CC1 and CC2.
    pub fn terminations(&self) -> (Termination, Termination) {
        self.shared.lock().terminations
    }

    /// Makes a pending or the next [`CcPhy::wait_for_cc_change`] return once, with an unchanged
    /// state, as the trait allows.
    pub fn trigger_spurious_change(&self) {
        self.shared.update(|inner| inner.cc_version += 1);
    }

    /// Makes the next call of any [`CcPhy`] method fail with `error`, without any effect.
    ///
    /// The error is taken by the first call whose future is polled, which then returns it
    /// immediately.
    pub fn fail_next(&self, error: SimError) {
        self.shared.lock().fail_next = Some(error);
    }

    /// Returns a watcher that reports every [`CcPhy::set_termination`] from now on.
    ///
    /// This is the hook for a model of what is connected to the port, e.g. a cable to another
    /// port, which has to react to the terminations this port presents.
    pub fn watch_terminations(&self) -> TerminationWatcher {
        TerminationWatcher {
            shared: self.shared.clone(),
            seen: self.shared.lock().termination_version,
        }
    }
}

/// Reports the terminations a port presents whenever it sets them. See
/// [`CcHandle::watch_terminations`].
#[derive(Debug)]
pub struct TerminationWatcher {
    shared: Arc<Shared<CcInner>>,
    seen: u64,
}

impl TerminationWatcher {
    /// Waits until [`CcPhy::set_termination`] was called since the last return (or since the
    /// watcher was created), and returns the terminations presented now.
    ///
    /// Returns immediately if it already was. Several calls in between are reported once. A call
    /// that sets the same terminations again is reported as well. Cancel-safe.
    pub async fn changed(&mut self) -> (Termination, Termination) {
        let seen = &mut self.seen;
        self.shared
            .wait_for(|inner| {
                (inner.termination_version != *seen).then(|| {
                    *seen = inner.termination_version;
                    inner.terminations
                })
            })
            .await
    }
}

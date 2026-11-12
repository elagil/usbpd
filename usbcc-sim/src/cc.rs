//! Mock of the CC front end.

use std::sync::Arc;

use tokio::sync::watch;
use usbcc_traits::{CcPhy, CcPin, CcState, Termination};

use crate::SimError;
use crate::log::{CcCall, Log};

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
    shared: Arc<watch::Sender<CcInner>>,
    log: Log,
    port: &'static str,
    /// `cc_version` at the last read.
    seen: u64,
}

/// The test side of the CC mock. Cloning yields another handle to the same mock.
#[derive(Debug, Clone)]
pub struct CcHandle {
    shared: Arc<watch::Sender<CcInner>>,
}

impl MockCcPhy {
    /// Creates a CC mock that records its calls in `log`, labelled with `port`.
    ///
    /// Both pins start [`CcState::Open`], with [`Termination::Open`] presented.
    pub fn new(log: &Log, port: &'static str) -> (Self, CcHandle) {
        let shared = Arc::new(watch::Sender::new(CcInner {
            cc: (CcState::Open, CcState::Open),
            cc_version: 0,
            terminations: (Termination::Open, Termination::Open),
            termination_version: 0,
            fail_next: None,
        }));
        let mock = Self {
            shared: shared.clone(),
            log: log.clone(),
            port,
            seen: 0,
        };
        (mock, CcHandle { shared })
    }

    /// Records the call and consumes an injected error.
    fn begin(&self, call: CcCall) -> Result<(), SimError> {
        self.log.record(self.port, call);
        let mut error = None;
        self.shared.send_modify(|inner| error = inner.fail_next.take());
        error.map_or(Ok(()), Err)
    }
}

impl CcPhy for MockCcPhy {
    type Error = SimError;

    async fn set_termination(&mut self, cc1: Termination, cc2: Termination) -> Result<(), SimError> {
        self.begin(CcCall::SetTermination { cc1, cc2 })?;
        self.shared.send_modify(|inner| {
            inner.terminations = (cc1, cc2);
            inner.termination_version += 1;
        });
        Ok(())
    }

    async fn cc_state(&mut self) -> Result<(CcState, CcState), SimError> {
        self.begin(CcCall::CcState)?;
        let inner = self.shared.borrow();
        self.seen = inner.cc_version;
        Ok(inner.cc)
    }

    async fn wait_for_cc_change(&mut self) -> Result<(CcState, CcState), SimError> {
        self.begin(CcCall::WaitForCcChange)?;
        let seen = self.seen;
        let (version, value) = self
            .shared
            .subscribe()
            .wait_for(|inner| inner.cc_version != seen)
            .await
            .map(|inner| (inner.cc_version, inner.cc))
            .expect("the mock holds the sender");
        self.seen = version;
        Ok(value)
    }
}

impl CcHandle {
    /// Sets what CC1 and CC2 see, and wakes the port if that is a change.
    pub fn set_cc(&self, cc1: CcState, cc2: CcState) {
        self.shared.send_modify(|inner| {
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
        self.shared.borrow().cc
    }

    /// The terminations that the port currently presents on CC1 and CC2.
    pub fn terminations(&self) -> (Termination, Termination) {
        self.shared.borrow().terminations
    }

    /// Makes a pending or the next [`CcPhy::wait_for_cc_change`] return once, with an unchanged
    /// state, as the trait allows.
    pub fn trigger_spurious_change(&self) {
        self.shared.send_modify(|inner| inner.cc_version += 1);
    }

    /// Makes the next call of any [`CcPhy`] method fail with `error`, without any effect.
    ///
    /// The error is taken by the first call whose future is polled, which then returns it
    /// immediately.
    pub fn fail_next(&self, error: SimError) {
        self.shared.send_modify(|inner| inner.fail_next = Some(error));
    }

    /// Returns a watcher that reports every [`CcPhy::set_termination`] from now on.
    ///
    /// This is the hook for a model of what is connected to the port, e.g. a cable to another
    /// port, which has to react to the terminations this port presents.
    pub fn watch_terminations(&self) -> TerminationWatcher {
        TerminationWatcher {
            shared: self.shared.clone(),
            seen: self.shared.borrow().termination_version,
        }
    }
}

/// Reports the terminations a port presents whenever it sets them. See
/// [`CcHandle::watch_terminations`].
#[derive(Debug)]
pub struct TerminationWatcher {
    shared: Arc<watch::Sender<CcInner>>,
    seen: u64,
}

impl TerminationWatcher {
    /// Waits until [`CcPhy::set_termination`] was called since the last return (or since the
    /// watcher was created), and returns the terminations presented now.
    ///
    /// Returns immediately if it already was. Several calls in between are reported once. A call
    /// that sets the same terminations again is reported as well. Cancel-safe.
    pub async fn changed(&mut self) -> (Termination, Termination) {
        let seen = self.seen;
        let (version, value) = self
            .shared
            .subscribe()
            .wait_for(|inner| inner.termination_version != seen)
            .await
            .map(|inner| (inner.termination_version, inner.terminations))
            .expect("the mock holds the sender");
        self.seen = version;
        value
    }
}

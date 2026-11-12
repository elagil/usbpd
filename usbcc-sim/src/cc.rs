//! Mock of the CC front end.

use std::future::pending;

use tokio::sync::watch;
use usbcc_traits::{CcPhy, CcPin, CcState, Termination};

use crate::log::{CcCall, Log};
use crate::{FailNext, SimError, take_error};

/// The [`CcPhy`] side of the CC mock, handed to the port.
///
/// Reports the [`CcState`]s set through its [`CcHandle`] verbatim, whatever terminations the port
/// presents: keeping the two consistent is up to the test (or, later, a cable model).
///
/// [`CcPhy::wait_for_cc_change`] returns once the state was set through the handle since the last
/// read by [`CcPhy::cc_state`] or [`CcPhy::wait_for_cc_change`]. This includes changes that were
/// reverted before the read (A → B → A), like an edge-triggered interrupt. The initial state, both
/// pins [`CcState::Open`], counts as read. It never returns spuriously unless the test asks for it
/// with [`CcHandle::trigger_spurious_change`].
#[derive(Debug)]
pub struct MockCcPhy {
    /// What the pins see; tracks what was read.
    cc: watch::Receiver<(CcState, CcState)>,
    terminations: watch::Sender<(Termination, Termination)>,
    fail_next: FailNext,
    log: Log,
}

/// The test side of the CC mock. Cloning yields another handle to the same mock.
#[derive(Debug, Clone)]
pub struct CcHandle {
    cc: watch::Sender<(CcState, CcState)>,
    terminations: watch::Sender<(Termination, Termination)>,
    fail_next: FailNext,
}

impl MockCcPhy {
    /// Creates a CC mock of hardware that powers up presenting [`Termination::Open`] on both pins.
    ///
    /// Records its calls in `log`. Both pins see [`CcState::Open`].
    pub fn new_open(log: &Log) -> (Self, CcHandle) {
        Self::new(log, Termination::Open)
    }

    /// Creates a CC mock of hardware with dead-battery support, which powers up presenting
    /// [`Termination::Rd`] on both pins.
    ///
    /// Records its calls in `log`. Both pins see [`CcState::Open`].
    pub fn new_dead_battery(log: &Log) -> (Self, CcHandle) {
        Self::new(log, Termination::Rd)
    }

    fn new(log: &Log, termination: Termination) -> (Self, CcHandle) {
        let handle = CcHandle {
            cc: watch::Sender::new((CcState::Open, CcState::Open)),
            terminations: watch::Sender::new((termination, termination)),
            fail_next: FailNext::new(None),
        };
        let mock = Self {
            cc: handle.cc.subscribe(),
            terminations: handle.terminations.clone(),
            fail_next: handle.fail_next.clone(),
            log: log.clone(),
        };
        (mock, handle)
    }

    /// Records the call and consumes an injected error.
    fn begin(&self, call: CcCall) -> Result<(), SimError> {
        self.log.record(call);
        take_error(&self.fail_next)
    }
}

impl CcPhy for MockCcPhy {
    type Error = SimError;

    async fn set_termination(&mut self, cc1: Termination, cc2: Termination) -> Result<(), SimError> {
        self.begin(CcCall::SetTermination { cc1, cc2 })?;
        self.terminations.send_replace((cc1, cc2));
        Ok(())
    }

    async fn cc_state(&mut self) -> Result<(CcState, CcState), SimError> {
        self.begin(CcCall::CcState)?;
        Ok(*self.cc.borrow_and_update())
    }

    async fn wait_for_cc_change(&mut self) -> Result<(CcState, CcState), SimError> {
        self.begin(CcCall::WaitForCcChange)?;
        if self.cc.changed().await.is_err() {
            // Every handle was dropped, so nothing can change any more.
            pending::<()>().await;
        }
        Ok(*self.cc.borrow_and_update())
    }
}

impl CcHandle {
    /// Sets what CC1 and CC2 see, and wakes the port if that is a change.
    pub fn set_cc(&self, cc1: CcState, cc2: CcState) {
        self.cc.send_if_modified(|cc| {
            let modified = *cc != (cc1, cc2);
            *cc = (cc1, cc2);
            modified
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
        *self.cc.borrow()
    }

    /// The terminations that the port currently presents on CC1 and CC2.
    pub fn terminations(&self) -> (Termination, Termination) {
        *self.terminations.borrow()
    }

    /// Makes a pending or the next [`CcPhy::wait_for_cc_change`] return once, with an unchanged
    /// state, as the trait allows.
    pub fn trigger_spurious_change(&self) {
        self.cc.send_modify(|_| {});
    }

    /// Makes the next call of any [`CcPhy`] method fail with `error`, without any effect.
    ///
    /// The error is taken by the first call whose future is polled, which then returns it
    /// immediately.
    pub fn fail_next(&self, error: SimError) {
        self.fail_next.send_replace(Some(error));
    }

    /// Returns a receiver of the terminations the port presents, which marks every
    /// [`CcPhy::set_termination`] from now on as a change, even of identical terminations.
    ///
    /// This is the hook for a model of what is connected to the port, e.g. a cable to another
    /// port, which has to react to the terminations this port presents.
    pub fn watch_terminations(&self) -> watch::Receiver<(Termination, Termination)> {
        self.terminations.subscribe()
    }
}

//! Mock of the power path.

use std::future::Future;
use std::sync::Arc;

use usbcc_traits::{CcPin, PowerPath};

use crate::SimError;
use crate::log::{Log, PowerCall};
use crate::shared::Shared;

/// Snapshot of the switches of a [`MockPowerPath`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PowerPathState {
    /// Set by [`PowerPath::set_active`].
    pub active: bool,
    /// Set by [`PowerPath::set_vbus_sink`].
    pub vbus_sink: bool,
    /// Set by [`PowerPath::set_vbus_source`].
    pub vbus_source: bool,
    /// Set by [`PowerPath::set_vbus_discharge`].
    pub vbus_discharge: bool,
    /// Set by [`PowerPath::set_vconn`].
    pub vconn: Option<CcPin>,
    /// Set by [`PowerPath::set_vconn_discharge`].
    pub vconn_discharge: Option<CcPin>,
}

#[derive(Debug)]
struct PowerInner {
    state: PowerPathState,
    source_ready: bool,
    vconn_discharged: bool,
    in_progress: Option<&'static str>,
    fail_next: Option<SimError>,
}

/// The [`PowerPath`] side of the power path mock, handed to the port.
///
/// Every method is overridden, so every call is recorded, including those with a default
/// implementation in the trait. The `set_*` methods take effect and complete on their first poll.
/// [`PowerPath::prepare_vbus_source`] and [`PowerPath::wait_vconn_discharged`] wait for flags set
/// through the [`PowerPathHandle`]; both are set initially, so they complete immediately unless the
/// test clears them.
///
/// # Panics
///
/// Panics when a method is called while the future of another call still exists, i.e. was neither
/// completed nor dropped. The trait guarantees that `usbcc` never does that. A call is in progress
/// from the method call, not from the first poll.
#[derive(Debug)]
pub struct MockPowerPath {
    shared: Arc<Shared<PowerInner>>,
    log: Log,
    port: &'static str,
}

/// The test side of the power path mock. Cloning yields another handle to the same mock.
#[derive(Debug, Clone)]
pub struct PowerPathHandle {
    shared: Arc<Shared<PowerInner>>,
}

/// Marks a call as in progress until dropped.
struct InProgress {
    shared: Arc<Shared<PowerInner>>,
}

impl Drop for InProgress {
    fn drop(&mut self) {
        self.shared.lock().in_progress = None;
    }
}

impl MockPowerPath {
    /// Creates a power path mock that records its calls in `log`, labelled with `port`.
    ///
    /// Starts inactive with everything off.
    pub fn new(log: &Log, port: &'static str) -> (Self, PowerPathHandle) {
        let shared = Shared::new(PowerInner {
            state: PowerPathState::default(),
            source_ready: true,
            vconn_discharged: true,
            in_progress: None,
            fail_next: None,
        });
        let mock = Self {
            shared: shared.clone(),
            log: log.clone(),
            port,
        };
        (mock, PowerPathHandle { shared })
    }

    /// Records the call, checks for concurrent calls and marks this one as in progress.
    fn begin(&self, name: &'static str, call: PowerCall) -> InProgress {
        self.log.record(self.port, call);
        let mut inner = self.shared.lock();
        if let Some(other) = inner.in_progress {
            drop(inner);
            panic!("`PowerPath::{name}` called while `PowerPath::{other}` is in progress");
        }
        inner.in_progress = Some(name);
        InProgress {
            shared: self.shared.clone(),
        }
    }

    /// A call that changes the state and completes on its first poll.
    fn set(
        &self,
        name: &'static str,
        call: PowerCall,
        apply: impl FnOnce(&mut PowerPathState) + 'static,
    ) -> impl Future<Output = Result<(), SimError>> {
        let guard = self.begin(name, call);
        async move {
            take_error(&guard.shared)?;
            guard.shared.update(|inner| apply(&mut inner.state));
            Ok(())
        }
    }

    /// A call that waits until `ready` is true.
    fn wait(
        &self,
        name: &'static str,
        call: PowerCall,
        ready: fn(&PowerInner) -> bool,
    ) -> impl Future<Output = Result<(), SimError>> + use<> {
        let guard = self.begin(name, call);
        async move {
            take_error(&guard.shared)?;
            guard.shared.wait_for(|inner| ready(inner).then_some(())).await;
            Ok(())
        }
    }
}

/// Consumes an injected error.
fn take_error(shared: &Shared<PowerInner>) -> Result<(), SimError> {
    shared.lock().fail_next.take().map_or(Ok(()), Err)
}

impl PowerPath for MockPowerPath {
    type Error = SimError;

    fn set_active(&mut self, active: bool) -> impl Future<Output = Result<(), Self::Error>> {
        self.set("set_active", PowerCall::SetActive(active), move |s| s.active = active)
    }

    fn set_vbus_sink(&mut self, on: bool) -> impl Future<Output = Result<(), Self::Error>> {
        self.set("set_vbus_sink", PowerCall::SetVbusSink(on), move |s| s.vbus_sink = on)
    }

    fn prepare_vbus_source(&mut self) -> impl Future<Output = Result<(), Self::Error>> {
        self.wait("prepare_vbus_source", PowerCall::PrepareVbusSource, |inner| {
            inner.source_ready
        })
    }

    fn set_vbus_source(&mut self, on: bool) -> impl Future<Output = Result<(), Self::Error>> {
        self.set("set_vbus_source", PowerCall::SetVbusSource(on), move |s| {
            s.vbus_source = on
        })
    }

    fn set_vbus_discharge(&mut self, on: bool) -> impl Future<Output = Result<(), Self::Error>> {
        self.set("set_vbus_discharge", PowerCall::SetVbusDischarge(on), move |s| {
            s.vbus_discharge = on
        })
    }

    fn set_vconn(&mut self, pin: Option<CcPin>) -> impl Future<Output = Result<(), Self::Error>> {
        self.set("set_vconn", PowerCall::SetVconn(pin), move |s| s.vconn = pin)
    }

    fn set_vconn_discharge(&mut self, pin: Option<CcPin>) -> impl Future<Output = Result<(), Self::Error>> {
        self.set("set_vconn_discharge", PowerCall::SetVconnDischarge(pin), move |s| {
            s.vconn_discharge = pin
        })
    }

    fn wait_vconn_discharged(&mut self) -> impl Future<Output = Result<(), Self::Error>> {
        self.wait("wait_vconn_discharged", PowerCall::WaitVconnDischarged, |inner| {
            inner.vconn_discharged
        })
    }
}

impl PowerPathHandle {
    /// The current state of the switches.
    pub fn state(&self) -> PowerPathState {
        self.shared.lock().state
    }

    /// Sets whether the supply behind the source path is ready, i.e. whether
    /// [`PowerPath::prepare_vbus_source`] completes. Initially `true`.
    pub fn set_source_ready(&self, ready: bool) {
        self.shared.update(|inner| inner.source_ready = ready);
    }

    /// Sets whether VCONN is discharged, i.e. whether [`PowerPath::wait_vconn_discharged`]
    /// completes. Initially `true`.
    pub fn set_vconn_discharged(&self, discharged: bool) {
        self.shared.update(|inner| inner.vconn_discharged = discharged);
    }

    /// The method whose call is in progress, e.g. `"prepare_vbus_source"` while that is pending.
    pub fn in_progress(&self) -> Option<&'static str> {
        self.shared.lock().in_progress
    }

    /// Makes the next call of any [`PowerPath`] method fail with `error`, without any effect.
    ///
    /// The error is taken by the first call whose future is polled, which then returns it
    /// immediately.
    pub fn fail_next(&self, error: SimError) {
        self.shared.lock().fail_next = Some(error);
    }
}

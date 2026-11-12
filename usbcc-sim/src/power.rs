//! Mock of the power path.

use tokio::sync::watch;
use usbcc_traits::{CcPin, PowerPath};

use crate::log::{Log, PowerCall};
use crate::{FailNext, SimError, take_error};

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

/// The [`PowerPath`] side of the power path mock, handed to the port.
///
/// Every method is overridden, so every call is recorded, including those with a default
/// implementation in the trait. The `set_*` methods complete immediately.
/// [`PowerPath::prepare_vbus_source`] and [`PowerPath::wait_vconn_discharged`] wait for flags set
/// through the [`PowerPathHandle`]; both are set initially, so they complete immediately unless the
/// test clears them.
#[derive(Debug)]
pub struct MockPowerPath {
    state: watch::Sender<PowerPathState>,
    source_ready: watch::Sender<bool>,
    vconn_discharged: watch::Sender<bool>,
    fail_next: FailNext,
    log: Log,
}

/// The test side of the power path mock. Cloning yields another handle to the same mock.
#[derive(Debug, Clone)]
pub struct PowerPathHandle {
    state: watch::Sender<PowerPathState>,
    source_ready: watch::Sender<bool>,
    vconn_discharged: watch::Sender<bool>,
    fail_next: FailNext,
}

impl MockPowerPath {
    /// Creates a power path mock that records its calls in `log`.
    ///
    /// Starts inactive with everything off.
    pub fn new(log: &Log) -> (Self, PowerPathHandle) {
        let handle = PowerPathHandle {
            state: watch::Sender::default(),
            source_ready: watch::Sender::new(true),
            vconn_discharged: watch::Sender::new(true),
            fail_next: FailNext::default(),
        };
        let mock = Self {
            state: handle.state.clone(),
            source_ready: handle.source_ready.clone(),
            vconn_discharged: handle.vconn_discharged.clone(),
            fail_next: handle.fail_next.clone(),
            log: log.clone(),
        };
        (mock, handle)
    }

    /// Implements a `set_*` method: applies `apply` to the switches.
    async fn set(&self, call: PowerCall, apply: impl FnOnce(&mut PowerPathState)) -> Result<(), SimError> {
        self.begin(call)?;
        self.state.send_modify(apply);
        Ok(())
    }

    /// Implements a waiting method: waits until `flag` is set.
    async fn wait(&self, call: PowerCall, flag: &watch::Sender<bool>) -> Result<(), SimError> {
        self.begin(call)?;
        flag.subscribe()
            .wait_for(|&set| set)
            .await
            .expect("the mock holds the sender");
        Ok(())
    }

    /// Records the call and consumes an injected error.
    fn begin(&self, call: PowerCall) -> Result<(), SimError> {
        self.log.record(call);
        take_error(&self.fail_next)
    }
}

impl PowerPath for MockPowerPath {
    type Error = SimError;

    async fn set_active(&mut self, active: bool) -> Result<(), SimError> {
        self.set(PowerCall::SetActive(active), |s| s.active = active).await
    }

    async fn set_vbus_sink(&mut self, on: bool) -> Result<(), SimError> {
        self.set(PowerCall::SetVbusSink(on), |s| s.vbus_sink = on).await
    }

    async fn prepare_vbus_source(&mut self) -> Result<(), SimError> {
        self.wait(PowerCall::PrepareVbusSource, &self.source_ready).await
    }

    async fn set_vbus_source(&mut self, on: bool) -> Result<(), SimError> {
        self.set(PowerCall::SetVbusSource(on), |s| s.vbus_source = on).await
    }

    async fn set_vbus_discharge(&mut self, on: bool) -> Result<(), SimError> {
        self.set(PowerCall::SetVbusDischarge(on), |s| s.vbus_discharge = on)
            .await
    }

    async fn set_vconn(&mut self, pin: Option<CcPin>) -> Result<(), SimError> {
        self.set(PowerCall::SetVconn(pin), |s| s.vconn = pin).await
    }

    async fn set_vconn_discharge(&mut self, pin: Option<CcPin>) -> Result<(), SimError> {
        self.set(PowerCall::SetVconnDischarge(pin), |s| s.vconn_discharge = pin)
            .await
    }

    async fn wait_vconn_discharged(&mut self) -> Result<(), SimError> {
        self.wait(PowerCall::WaitVconnDischarged, &self.vconn_discharged).await
    }
}

impl PowerPathHandle {
    /// The current state of the switches.
    pub fn state(&self) -> PowerPathState {
        *self.state.borrow()
    }

    /// Sets whether the supply behind the source path is ready, i.e. whether
    /// [`PowerPath::prepare_vbus_source`] completes. Initially `true`.
    pub fn set_source_ready(&self, ready: bool) {
        self.source_ready.send_replace(ready);
    }

    /// Sets whether VCONN is discharged, i.e. whether [`PowerPath::wait_vconn_discharged`]
    /// completes. Initially `true`.
    pub fn set_vconn_discharged(&self, discharged: bool) {
        self.vconn_discharged.send_replace(discharged);
    }

    /// Makes the next call of any [`PowerPath`] method fail with `error`, without any effect.
    ///
    /// The error is taken by the first call whose future is polled, which then returns it
    /// immediately.
    pub fn fail_next(&self, error: SimError) {
        *self.fail_next.lock().unwrap() = Some(error);
    }
}

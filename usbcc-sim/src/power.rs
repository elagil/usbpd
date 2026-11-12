//! Mock of the power path.

use std::future::Future;
use std::sync::Arc;

use tokio::sync::watch;
use usbcc_traits::{CcPin, PowerPath};

use crate::SimError;
use crate::log::{Log, PowerCall};

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
    fail_next: Option<SimError>,
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
    shared: Arc<watch::Sender<PowerInner>>,
    log: Log,
    port: &'static str,
}

/// The test side of the power path mock. Cloning yields another handle to the same mock.
#[derive(Debug, Clone)]
pub struct PowerPathHandle {
    shared: Arc<watch::Sender<PowerInner>>,
}

impl MockPowerPath {
    /// Creates a power path mock that records its calls in `log`, labelled with `port`.
    ///
    /// Starts inactive with everything off.
    pub fn new(log: &Log, port: &'static str) -> (Self, PowerPathHandle) {
        let shared = Arc::new(watch::Sender::new(PowerInner {
            state: PowerPathState::default(),
            source_ready: true,
            vconn_discharged: true,
            fail_next: None,
        }));
        let mock = Self {
            shared: shared.clone(),
            log: log.clone(),
            port,
        };
        (mock, PowerPathHandle { shared })
    }

    /// Implements a `set_*` method: applies `apply` to the switches.
    fn set(
        &self,
        call: PowerCall,
        apply: impl FnOnce(&mut PowerPathState) + 'static,
    ) -> impl Future<Output = Result<(), SimError>> {
        self.log.record(self.port, call);
        let shared = self.shared.clone();
        async move {
            take_error(&shared)?;
            shared.send_modify(|inner| apply(&mut inner.state));
            Ok(())
        }
    }

    /// Implements a waiting method: waits until `ready` is true.
    fn wait(
        &self,
        call: PowerCall,
        ready: fn(&PowerInner) -> bool,
    ) -> impl Future<Output = Result<(), SimError>> + use<> {
        self.log.record(self.port, call);
        let shared = self.shared.clone();
        async move {
            take_error(&shared)?;
            shared
                .subscribe()
                .wait_for(ready)
                .await
                .expect("the mock holds the sender");
            Ok(())
        }
    }
}

/// Consumes an injected error.
fn take_error(shared: &watch::Sender<PowerInner>) -> Result<(), SimError> {
    let mut error = None;
    shared.send_modify(|inner| error = inner.fail_next.take());
    error.map_or(Ok(()), Err)
}

impl PowerPath for MockPowerPath {
    type Error = SimError;

    fn set_active(&mut self, active: bool) -> impl Future<Output = Result<(), Self::Error>> {
        self.set(PowerCall::SetActive(active), move |s| s.active = active)
    }

    fn set_vbus_sink(&mut self, on: bool) -> impl Future<Output = Result<(), Self::Error>> {
        self.set(PowerCall::SetVbusSink(on), move |s| s.vbus_sink = on)
    }

    fn prepare_vbus_source(&mut self) -> impl Future<Output = Result<(), Self::Error>> {
        self.wait(PowerCall::PrepareVbusSource, |inner| inner.source_ready)
    }

    fn set_vbus_source(&mut self, on: bool) -> impl Future<Output = Result<(), Self::Error>> {
        self.set(PowerCall::SetVbusSource(on), move |s| s.vbus_source = on)
    }

    fn set_vbus_discharge(&mut self, on: bool) -> impl Future<Output = Result<(), Self::Error>> {
        self.set(PowerCall::SetVbusDischarge(on), move |s| s.vbus_discharge = on)
    }

    fn set_vconn(&mut self, pin: Option<CcPin>) -> impl Future<Output = Result<(), Self::Error>> {
        self.set(PowerCall::SetVconn(pin), move |s| s.vconn = pin)
    }

    fn set_vconn_discharge(&mut self, pin: Option<CcPin>) -> impl Future<Output = Result<(), Self::Error>> {
        self.set(PowerCall::SetVconnDischarge(pin), move |s| s.vconn_discharge = pin)
    }

    fn wait_vconn_discharged(&mut self) -> impl Future<Output = Result<(), Self::Error>> {
        self.wait(PowerCall::WaitVconnDischarged, |inner| inner.vconn_discharged)
    }
}

impl PowerPathHandle {
    /// The current state of the switches.
    pub fn state(&self) -> PowerPathState {
        self.shared.borrow().state
    }

    /// Sets whether the supply behind the source path is ready, i.e. whether
    /// [`PowerPath::prepare_vbus_source`] completes. Initially `true`.
    pub fn set_source_ready(&self, ready: bool) {
        self.shared.send_modify(|inner| inner.source_ready = ready);
    }

    /// Sets whether VCONN is discharged, i.e. whether [`PowerPath::wait_vconn_discharged`]
    /// completes. Initially `true`.
    pub fn set_vconn_discharged(&self, discharged: bool) {
        self.shared.send_modify(|inner| inner.vconn_discharged = discharged);
    }

    /// Makes the next call of any [`PowerPath`] method fail with `error`, without any effect.
    ///
    /// The error is taken by the first call whose future is polled, which then returns it
    /// immediately.
    pub fn fail_next(&self, error: SimError) {
        self.shared.send_modify(|inner| inner.fail_next = Some(error));
    }
}

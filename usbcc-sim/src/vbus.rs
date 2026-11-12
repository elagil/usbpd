//! Mock of the VBUS measurement.

use std::future::Future;
use std::sync::Arc;

use usbcc_traits::VbusSense;

use crate::SimError;
use crate::log::{Log, VbusCall};
use crate::shared::Shared;

#[derive(Debug)]
struct VbusInner {
    mv: u32,
    fail_next: Option<SimError>,
}

/// The [`VbusSense`] side of the VBUS mock, handed to the port.
///
/// The waits are level-based: they compare the voltage set through the [`VbusHandle`] whenever it
/// changes, with strict comparisons ("below `low_mv`" is `mv < low_mv`). An excursion that the test
/// reverts before the waiting task runs is not seen, as with a polling implementation. They never
/// return spuriously.
#[derive(Debug)]
pub struct MockVbusSense {
    shared: Arc<Shared<VbusInner>>,
    log: Log,
    port: &'static str,
}

/// The test side of the VBUS mock. Cloning yields another handle to the same mock.
#[derive(Debug, Clone)]
pub struct VbusHandle {
    shared: Arc<Shared<VbusInner>>,
}

impl MockVbusSense {
    /// Creates a VBUS mock that records its calls in `log`, labelled with `port`. VBUS starts at
    /// 0 mV.
    pub fn new(log: &Log, port: &'static str) -> (Self, VbusHandle) {
        let shared = Shared::new(VbusInner { mv: 0, fail_next: None });
        let mock = Self {
            shared: shared.clone(),
            log: log.clone(),
            port,
        };
        (mock, VbusHandle { shared })
    }

    fn wait(&self, call: VbusCall, low_mv: u32, high_mv: u32) -> impl Future<Output = Result<(), SimError>> + use<> {
        self.log.record(self.port, call);
        let shared = self.shared.clone();
        async move {
            take_error(&shared)?;
            shared
                .wait_for(|inner| (inner.mv < low_mv || inner.mv > high_mv).then_some(()))
                .await;
            Ok(())
        }
    }
}

/// Consumes an injected error.
fn take_error(shared: &Shared<VbusInner>) -> Result<(), SimError> {
    shared.lock().fail_next.take().map_or(Ok(()), Err)
}

impl VbusSense for MockVbusSense {
    type Error = SimError;

    fn vbus_mv(&mut self) -> impl Future<Output = Result<u32, Self::Error>> {
        self.log.record(self.port, VbusCall::VbusMv);
        async move {
            take_error(&self.shared)?;
            Ok(self.shared.lock().mv)
        }
    }

    fn wait_vbus_outside(&mut self, low_mv: u32, high_mv: u32) -> impl Future<Output = Result<(), Self::Error>> {
        self.wait(VbusCall::WaitVbusOutside { low_mv, high_mv }, low_mv, high_mv)
    }

    // Overridden only to record the method that was called; the semantics are the defaults'.
    fn wait_vbus_below(&mut self, mv: u32) -> impl Future<Output = Result<(), Self::Error>> {
        self.wait(VbusCall::WaitVbusBelow { mv }, mv, u32::MAX)
    }

    fn wait_vbus_above(&mut self, mv: u32) -> impl Future<Output = Result<(), Self::Error>> {
        self.wait(VbusCall::WaitVbusAbove { mv }, 0, mv)
    }
}

impl VbusHandle {
    /// Sets the VBUS voltage in mV, and wakes the port's waits.
    pub fn set_mv(&self, mv: u32) {
        self.shared.update(|inner| inner.mv = mv);
    }

    /// The current VBUS voltage in mV.
    pub fn mv(&self) -> u32 {
        self.shared.lock().mv
    }

    /// Makes the next call of any [`VbusSense`] method fail with `error`, without any effect.
    ///
    /// The error is taken by the first call whose future is polled, which then returns it
    /// immediately.
    pub fn fail_next(&self, error: SimError) {
        self.shared.lock().fail_next = Some(error);
    }
}

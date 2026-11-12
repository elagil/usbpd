//! Mock of the VBUS measurement.

use std::sync::Arc;

use tokio::sync::watch;
use usbcc_traits::VbusSense;

use crate::SimError;
use crate::log::{Log, VbusCall};

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
    shared: Arc<watch::Sender<VbusInner>>,
    log: Log,
}

/// The test side of the VBUS mock. Cloning yields another handle to the same mock.
#[derive(Debug, Clone)]
pub struct VbusHandle {
    shared: Arc<watch::Sender<VbusInner>>,
}

impl MockVbusSense {
    /// Creates a VBUS mock that records its calls in `log`. VBUS starts at
    /// 0 mV.
    pub fn new(log: &Log) -> (Self, VbusHandle) {
        let shared = Arc::new(watch::Sender::new(VbusInner { mv: 0, fail_next: None }));
        let mock = Self {
            shared: shared.clone(),
            log: log.clone(),
        };
        (mock, VbusHandle { shared })
    }

    async fn wait(&self, call: VbusCall, low_mv: u32, high_mv: u32) -> Result<(), SimError> {
        self.begin(call)?;
        self.shared
            .subscribe()
            .wait_for(|inner| inner.mv < low_mv || inner.mv > high_mv)
            .await
            .expect("the mock holds the sender");
        Ok(())
    }

    /// Records the call and consumes an injected error.
    fn begin(&self, call: VbusCall) -> Result<(), SimError> {
        self.log.record(call);
        let mut error = None;
        self.shared.send_modify(|inner| error = inner.fail_next.take());
        error.map_or(Ok(()), Err)
    }
}

impl VbusSense for MockVbusSense {
    type Error = SimError;

    async fn vbus_mv(&mut self) -> Result<u32, SimError> {
        self.begin(VbusCall::VbusMv)?;
        Ok(self.shared.borrow().mv)
    }

    async fn wait_vbus_outside(&mut self, low_mv: u32, high_mv: u32) -> Result<(), SimError> {
        self.wait(VbusCall::WaitVbusOutside { low_mv, high_mv }, low_mv, high_mv)
            .await
    }

    // Overridden only to record the method that was called; the semantics are the defaults'.
    async fn wait_vbus_below(&mut self, mv: u32) -> Result<(), SimError> {
        self.wait(VbusCall::WaitVbusBelow { mv }, mv, u32::MAX).await
    }

    async fn wait_vbus_above(&mut self, mv: u32) -> Result<(), SimError> {
        self.wait(VbusCall::WaitVbusAbove { mv }, 0, mv).await
    }
}

impl VbusHandle {
    /// Sets the VBUS voltage in mV, and wakes the port's waits.
    pub fn set_mv(&self, mv: u32) {
        self.shared.send_modify(|inner| inner.mv = mv);
    }

    /// The current VBUS voltage in mV.
    pub fn mv(&self) -> u32 {
        self.shared.borrow().mv
    }

    /// Makes the next call of any [`VbusSense`] method fail with `error`, without any effect.
    ///
    /// The error is taken by the first call whose future is polled, which then returns it
    /// immediately.
    pub fn fail_next(&self, error: SimError) {
        self.shared.send_modify(|inner| inner.fail_next = Some(error));
    }
}

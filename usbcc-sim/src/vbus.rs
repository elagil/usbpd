//! Mock of the VBUS measurement.

use tokio::sync::watch;
use usbcc_traits::VbusSense;

use crate::log::{Log, VbusCall};
use crate::{FailNext, SimError, take_error};

/// The [`VbusSense`] side of the VBUS mock, handed to the port.
///
/// The waits are level-based: they compare the voltage set through the [`VbusHandle`] whenever it
/// changes, with strict comparisons ("below `low_mv`" is `mv < low_mv`). An excursion that the test
/// reverts before the waiting task runs is not seen, as with a polling implementation. They never
/// return spuriously.
#[derive(Debug)]
pub struct MockVbusSense {
    mv: watch::Sender<u32>,
    fail_next: FailNext,
    log: Log,
}

/// The test side of the VBUS mock. Cloning yields another handle to the same mock.
#[derive(Debug, Clone)]
pub struct VbusHandle {
    mv: watch::Sender<u32>,
    fail_next: FailNext,
}

impl MockVbusSense {
    /// Creates a VBUS mock that records its calls in `log`. VBUS starts at 0 mV.
    pub fn new(log: &Log) -> (Self, VbusHandle) {
        let handle = VbusHandle {
            mv: watch::Sender::new(0),
            fail_next: FailNext::new(None),
        };
        let mock = Self {
            mv: handle.mv.clone(),
            fail_next: handle.fail_next.clone(),
            log: log.clone(),
        };
        (mock, handle)
    }

    async fn wait(&self, call: VbusCall, low_mv: u32, high_mv: u32) -> Result<(), SimError> {
        self.begin(call)?;
        self.mv
            .subscribe()
            .wait_for(|&mv| mv < low_mv || mv > high_mv)
            .await
            .expect("the mock holds the sender");
        Ok(())
    }

    /// Records the call and consumes an injected error.
    fn begin(&self, call: VbusCall) -> Result<(), SimError> {
        self.log.record(call);
        take_error(&self.fail_next)
    }
}

impl VbusSense for MockVbusSense {
    type Error = SimError;

    async fn vbus_mv(&mut self) -> Result<u32, SimError> {
        self.begin(VbusCall::VbusMv)?;
        Ok(*self.mv.borrow())
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
        self.mv.send_replace(mv);
    }

    /// The current VBUS voltage in mV.
    pub fn mv(&self) -> u32 {
        *self.mv.borrow()
    }

    /// Makes the next call of any [`VbusSense`] method fail with `error`, without any effect.
    ///
    /// The error is taken by the first call whose future is polled, which then returns it
    /// immediately.
    pub fn fail_next(&self, error: SimError) {
        self.fail_next.send_replace(Some(error));
    }
}

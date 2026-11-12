//! The shared, ordered call log.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use tokio::time::Instant;
use usbcc::{Event, Info};
use usbcc_traits::{CcPin, Termination};

/// A call of a [`CcPhy`](usbcc_traits::CcPhy) method, with its arguments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CcCall {
    /// [`CcPhy::set_termination`](usbcc_traits::CcPhy::set_termination).
    SetTermination {
        /// Termination requested on CC1.
        cc1: Termination,
        /// Termination requested on CC2.
        cc2: Termination,
    },
    /// [`CcPhy::cc_state`](usbcc_traits::CcPhy::cc_state).
    CcState,
    /// [`CcPhy::wait_for_cc_change`](usbcc_traits::CcPhy::wait_for_cc_change).
    WaitForCcChange,
}

/// A call of a [`VbusSense`](usbcc_traits::VbusSense) method, with its arguments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VbusCall {
    /// [`VbusSense::vbus_mv`](usbcc_traits::VbusSense::vbus_mv).
    VbusMv,
    /// [`VbusSense::wait_vbus_outside`](usbcc_traits::VbusSense::wait_vbus_outside).
    WaitVbusOutside {
        /// Lower bound in mV.
        low_mv: u32,
        /// Upper bound in mV.
        high_mv: u32,
    },
    /// [`VbusSense::wait_vbus_below`](usbcc_traits::VbusSense::wait_vbus_below).
    WaitVbusBelow {
        /// Threshold in mV.
        mv: u32,
    },
    /// [`VbusSense::wait_vbus_above`](usbcc_traits::VbusSense::wait_vbus_above).
    WaitVbusAbove {
        /// Threshold in mV.
        mv: u32,
    },
}

/// A call of a [`PowerPath`](usbcc_traits::PowerPath) method, with its arguments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerCall {
    /// [`PowerPath::set_active`](usbcc_traits::PowerPath::set_active).
    SetActive(bool),
    /// [`PowerPath::set_vbus_sink`](usbcc_traits::PowerPath::set_vbus_sink).
    SetVbusSink(bool),
    /// [`PowerPath::prepare_vbus_source`](usbcc_traits::PowerPath::prepare_vbus_source).
    PrepareVbusSource,
    /// [`PowerPath::set_vbus_source`](usbcc_traits::PowerPath::set_vbus_source).
    SetVbusSource(bool),
    /// [`PowerPath::set_vbus_discharge`](usbcc_traits::PowerPath::set_vbus_discharge).
    SetVbusDischarge(bool),
    /// [`PowerPath::set_vconn`](usbcc_traits::PowerPath::set_vconn).
    SetVconn(Option<CcPin>),
    /// [`PowerPath::set_vconn_discharge`](usbcc_traits::PowerPath::set_vconn_discharge).
    SetVconnDischarge(Option<CcPin>),
    /// [`PowerPath::wait_vconn_discharged`](usbcc_traits::PowerPath::wait_vconn_discharged).
    WaitVconnDischarged,
}

/// An interaction of the state machine with the [`MockDpm`](crate::MockDpm).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DpmCall {
    /// [`DevicePolicyManager::inform`](usbcc::DevicePolicyManager::inform) was called.
    Inform(Info),
    /// [`DevicePolicyManager::get_event`](usbcc::DevicePolicyManager::get_event) returned this
    /// event. Calls that are still pending, or were dropped, aren't recorded.
    EventReturned(Event),
}

/// One recorded call, of any mock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Call {
    /// A [`CcPhy`](usbcc_traits::CcPhy) call.
    Cc(CcCall),
    /// A [`VbusSense`](usbcc_traits::VbusSense) call.
    Vbus(VbusCall),
    /// A [`PowerPath`](usbcc_traits::PowerPath) call.
    Power(PowerCall),
    /// A [`DevicePolicyManager`](usbcc::DevicePolicyManager) interaction.
    Dpm(DpmCall),
}

impl From<CcCall> for Call {
    fn from(call: CcCall) -> Self {
        Self::Cc(call)
    }
}

impl From<VbusCall> for Call {
    fn from(call: VbusCall) -> Self {
        Self::Vbus(call)
    }
}

impl From<PowerCall> for Call {
    fn from(call: PowerCall) -> Self {
        Self::Power(call)
    }
}

impl From<DpmCall> for Call {
    fn from(call: DpmCall) -> Self {
        Self::Dpm(call)
    }
}

/// A [`Call`] with the virtual time at which it was made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    /// Virtual time of the call, relative to the creation of the [`Log`].
    pub at: Duration,
    /// Label of the port whose mock recorded the call, see [`Sim::with_log`](crate::Sim::with_log).
    pub port: &'static str,
    /// The call and its arguments.
    pub call: Call,
}

#[derive(Debug)]
struct LogInner {
    epoch: Instant,
    entries: Vec<Entry>,
}

/// The call log, shared by all mocks created with it, in the order of the calls.
///
/// A call is recorded when its future is first polled. Calls that fail through an injected error
/// are recorded too. Cloning yields another handle to the same log.
#[derive(Debug, Clone)]
pub struct Log {
    inner: Arc<Mutex<LogInner>>,
}

impl Default for Log {
    fn default() -> Self {
        Self::new()
    }
}

impl Log {
    /// Creates an empty log. Its timestamps count from now, in tokio's (virtual) time.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(LogInner {
                epoch: Instant::now(),
                entries: Vec::new(),
            })),
        }
    }

    fn lock(&self) -> MutexGuard<'_, LogInner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn record(&self, port: &'static str, call: impl Into<Call>) {
        let mut inner = self.lock();
        let at = inner.epoch.elapsed();
        inner.entries.push(Entry {
            at,
            port,
            call: call.into(),
        });
    }

    /// The instant that timestamps count from.
    pub fn epoch(&self) -> Instant {
        self.lock().epoch
    }

    /// Virtual time elapsed since [`Log::epoch`], on the same scale as [`Entry::at`].
    pub fn now(&self) -> Duration {
        self.lock().epoch.elapsed()
    }

    /// A copy of all entries, oldest first.
    pub fn entries(&self) -> Vec<Entry> {
        self.lock().entries.clone()
    }

    /// The calls of all entries, without timestamps, oldest first.
    pub fn calls(&self) -> Vec<Call> {
        self.lock().entries.iter().map(|entry| entry.call).collect()
    }

    /// Removes and returns all entries, oldest first.
    ///
    /// Useful to assert on the calls made since the previous `take`.
    pub fn take(&self) -> Vec<Entry> {
        std::mem::take(&mut self.lock().entries)
    }

    /// Removes all entries. The epoch is kept.
    pub fn clear(&self) {
        self.lock().entries.clear();
    }

    /// The number of entries.
    pub fn len(&self) -> usize {
        self.lock().entries.len()
    }

    /// Whether the log has no entries.
    pub fn is_empty(&self) -> bool {
        self.lock().entries.is_empty()
    }
}

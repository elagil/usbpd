//! Test simulator for the `usbcc` USB Type-C state machine.
//!
//! Provides scripted mocks of the `usbcc-traits` hardware traits and of the
//! [`DevicePolicyManager`](usbcc::DevicePolicyManager), for tests that drive a [`Port`] through its public API in tokio's
//! virtual time:
//!
//! - [`MockCcPhy`], [`MockVbusSense`] and [`MockPowerPath`] implement the hardware traits and are
//!   handed to the [`Port`], by value or by `&mut`. Each comes with a cloneable handle
//!   ([`CcHandle`], [`VbusHandle`], [`PowerPathHandle`]) through which the test sets the stimulus,
//!   reads what the port did and injects errors.
//! - [`MockDpm`] implements [`DevicePolicyManager`](usbcc::DevicePolicyManager); the test queues events and reads the
//!   information through its [`DpmHandle`].
//! - Every call is recorded with its virtual timestamp in one ordered [`Log`].
//! - [`SimTimer`] implements [`Timer`] with [`tokio::time::sleep`].
//! - [`run_for`] runs a future for a virtual duration, then drops it. [`SimFutureExt`] adds the
//!   common cases as methods: [`poll_once`](SimFutureExt::poll_once) and
//!   [`settle`](SimFutureExt::settle).
//!
//! [`Sim::new`] creates all of them at once.
//!
//! Tests run on tokio's paused clock, with `#[tokio::test(start_paused = true)]`: virtual time
//! only advances when every task waits, and jumps straight to the next timer. Changes made through
//! the handles take effect immediately and wake the port's pending waits.
//!
//! The stimulus is scripted: the test decides what the CC pins see and what VBUS is. Nothing
//! connects the terminations a port presents to what another port sees; a model of a cable can be
//! built on [`CcHandle::watch_terminations`] and [`CcHandle::set_cc`], with mocks of several
//! ports sharing one [`Log`] through [`Sim::with_log`].
//!
//! # Example
//!
//! The body of a test in `usbcc/tests/`, which runs with `#[tokio::test(start_paused = true)]`.
//! It doesn't run here yet, as the [`Port`] is still a skeleton.
//!
//! ```no_run
//! use std::time::Duration;
//!
//! use usbcc::{Config, PowerRole, RpLevel, State};
//! use usbcc_sim::{PowerCall, Sim, run_for};
//! use usbcc_traits::CcState;
//!
//! # #[tokio::main(flavor = "current_thread", start_paused = true)]
//! # async fn main() {
//! let (sim, mocks) = Sim::new();
//! let (mut port, mut dpm) = mocks.into_port(Config::default());
//!
//! // Nothing attached: the port keeps waiting.
//! let attached = run_for(Duration::from_millis(500), port.wait_attached(&mut dpm)).await;
//! assert!(attached.is_none());
//! assert_eq!(port.state(), State::UnattachedSnk);
//!
//! // A Source attaches on CC1 and supplies VBUS.
//! sim.cc.set_cc(CcState::Rp(RpLevel::Default), CcState::Open);
//! sim.vbus.set_mv(5000);
//! let attachment = run_for(Duration::from_millis(500), port.wait_attached(&mut dpm))
//!     .await
//!     .expect("attached")
//!     .unwrap();
//! assert_eq!(attachment.power_role, PowerRole::Sink);
//! assert!(sim.power.state().vbus_sink);
//! assert!(sim.log.calls().contains(&PowerCall::SetVbusSink(true).into()));
//! # }
//! ```
#![warn(missing_docs)]

mod cc;
mod dpm;
mod log;
mod power;
mod shared;
mod vbus;

use std::fmt;
use std::future::Future;
use std::time::Duration;

pub use cc::{CcHandle, MockCcPhy, TerminationWatcher};
pub use dpm::{DpmHandle, MockDpm};
pub use log::{Call, CcCall, DpmCall, Entry, Log, PowerCall, VbusCall};
pub use power::{MockPowerPath, PowerPathHandle, PowerPathState};
use usbcc::{Config, Port, Timer};
pub use vbus::{MockVbusSense, VbusHandle};

/// An error injected into a mock with `fail_next`; the error type of all mocks.
///
/// The string identifies the injection in assertions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SimError(pub &'static str);

impl fmt::Display for SimError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "injected error: {}", self.0)
    }
}

impl std::error::Error for SimError {}

/// [`Timer`] on tokio's clock, which tests pause.
#[derive(Debug, Clone, Copy, Default)]
pub struct SimTimer;

impl Timer for SimTimer {
    fn after_millis(milliseconds: u64) -> impl Future<Output = ()> {
        tokio::time::sleep(Duration::from_millis(milliseconds))
    }
}

/// A [`Port`] on the mocks of this crate, as created by [`Mocks::into_port`].
pub type SimPort = Port<MockCcPhy, MockVbusSense, MockPowerPath, SimTimer>;

/// Runs `future` for at most `duration` of virtual time, then drops it.
///
/// Returns its output if it completed within `duration`, including exactly at the end of it, and
/// `None` otherwise. Virtual time has advanced by `duration` in the latter case, and by the time
/// the future took in the former. With a zero `duration`, the future is polled once.
///
/// To keep a future alive across several runs, e.g. to change the stimulus while a wait is
/// pending, pin it and pass `&mut` to it: only the reference is dropped.
///
/// ```
/// # use std::time::Duration;
/// # use usbcc_sim::{Sim, run_for};
/// # use usbcc_traits::{CcPhy, CcState, RpLevel};
/// # #[tokio::main(flavor = "current_thread", start_paused = true)]
/// # async fn main() {
/// let (sim, mut mocks) = Sim::new();
/// let mut wait = std::pin::pin!(mocks.cc.wait_for_cc_change());
/// assert!(run_for(Duration::from_millis(100), &mut wait).await.is_none());
/// sim.cc.set_cc(CcState::Rp(RpLevel::Default), CcState::Open);
/// assert!(run_for(Duration::ZERO, &mut wait).await.is_some());
/// # }
/// ```
pub async fn run_for<F: Future>(duration: Duration, future: F) -> Option<F::Output> {
    tokio::time::timeout(duration, future).await.ok()
}

/// Virtual time that [`SimFutureExt::settle`] runs a future for: 1 s, longer than any reaction of
/// the mocks or any timer that a test is waiting out.
pub const SETTLE_TIME: Duration = Duration::from_secs(1);

/// Shorthands for [`run_for`], for any future.
///
/// To keep a future alive across calls, call them on a pinned reference to it, e.g.
/// `wait.as_mut().poll_once()` after `let mut wait = pin!(...)`.
pub trait SimFutureExt: Future + Sized {
    /// Polls the future once, without advancing time, then drops it.
    ///
    /// Returns its output if it was ready, and `None` if it would have waited.
    fn poll_once(self) -> impl Future<Output = Option<Self::Output>> {
        run_for(Duration::ZERO, self)
    }

    /// Runs the future for [`SETTLE_TIME`], then drops it.
    ///
    /// Returns its output if it completed, and `None` if it was still waiting, i.e. for something
    /// that the test has yet to do.
    fn settle(self) -> impl Future<Output = Option<Self::Output>> {
        run_for(SETTLE_TIME, self)
    }
}

impl<F: Future> SimFutureExt for F {}

/// The test side of the mocks of one port: the handles and the log.
#[derive(Debug, Clone)]
pub struct Sim {
    /// The shared call log.
    pub log: Log,
    /// Handle of the CC mock.
    pub cc: CcHandle,
    /// Handle of the VBUS mock.
    pub vbus: VbusHandle,
    /// Handle of the power path mock.
    pub power: PowerPathHandle,
    /// Handle of the DPM mock.
    pub dpm: DpmHandle,
}

/// The port side of the mocks of one port, see [`Sim::new`].
#[derive(Debug)]
pub struct Mocks {
    /// The CC mock.
    pub cc: MockCcPhy,
    /// The VBUS mock.
    pub vbus: MockVbusSense,
    /// The power path mock.
    pub power: MockPowerPath,
    /// The DPM mock.
    pub dpm: MockDpm,
}

impl Sim {
    /// The port label that [`Sim::new`] uses in the log.
    pub const DEFAULT_PORT: &'static str = "port";

    /// Creates the mocks of one port, with a new [`Log`].
    pub fn new() -> (Self, Mocks) {
        Self::with_log(&Log::new(), Self::DEFAULT_PORT)
    }

    /// Creates the mocks of one port that record into an existing `log`, with their entries
    /// labelled `port`. Use this for several ports in one log.
    pub fn with_log(log: &Log, port: &'static str) -> (Self, Mocks) {
        let (cc, cc_handle) = MockCcPhy::new(log, port);
        let (vbus, vbus_handle) = MockVbusSense::new(log, port);
        let (power, power_handle) = MockPowerPath::new(log, port);
        let (dpm, dpm_handle) = MockDpm::new(log, port);
        let sim = Self {
            log: log.clone(),
            cc: cc_handle,
            vbus: vbus_handle,
            power: power_handle,
            dpm: dpm_handle,
        };
        (sim, Mocks { cc, vbus, power, dpm })
    }
}

impl Mocks {
    /// Creates a [`Port`] that owns the hardware mocks, and returns it with the DPM mock.
    pub fn into_port(self, config: Config) -> (SimPort, MockDpm) {
        (Port::new(self.cc, self.vbus, self.power, config), self.dpm)
    }
}

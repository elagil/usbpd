//! Timers that are used by the protocol layer and policy engine.

/// The timer trait to implement by the user application.
pub trait Timer {
    /// Expire after the specified number of milliseconds.
    fn after_millis(milliseconds: u64) -> impl Future<Output = ()>;
}

use core::future::Future;

/// Types of timers that are used for timeouts.
#[allow(missing_docs)]
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum TimerType {
    BISTContMode,
    ChunkingNotSupported,
    ChunkSenderRequest,
    ChunkSenderResponse,
    CRCReceive,
    DataResetFail,
    DataResetFailUFP,
    DiscoverIdentity,
    HardResetComplete,
    NoResponse,
    PSHardReset,
    PSSourceOffSpr,
    PSSourceOffEpr,
    PSSourceOnSpr,
    PSTransitionSpr,
    PSTransitionEpr,
    SenderResponse,
    SinkEPREnter,
    SinkEPRKeepAlive,
    SinkPPSPeriodic,
    SinkRequest,
    SinkWaitCap,
    SourceCapability,
    SourceEPRKeepAlive,
    SourcePPSComm,
    SinkTx,
    SwapSourceStart,
    VCONNDischarge,
    VCONNOn,
    VDMModeEntry,
    VDMModeExit,
    VDMResponse,
}

impl TimerType {
    /// Create a new timer for a given type.
    ///
    /// Times out after a duration that is given by the USB PD specification.
    pub fn new<TIMER: Timer>(timer_type: TimerType) -> impl Future<Output = ()> {
        match timer_type {
            TimerType::BISTContMode => TIMER::after_millis(45),
            TimerType::ChunkingNotSupported => TIMER::after_millis(45),
            TimerType::ChunkSenderRequest => TIMER::after_millis(27),
            TimerType::ChunkSenderResponse => TIMER::after_millis(27),
            TimerType::CRCReceive => TIMER::after_millis(1),
            TimerType::DataResetFail => TIMER::after_millis(350),
            TimerType::DataResetFailUFP => TIMER::after_millis(500),
            TimerType::DiscoverIdentity => TIMER::after_millis(45),
            TimerType::HardResetComplete => TIMER::after_millis(5),
            TimerType::NoResponse => TIMER::after_millis(5000),
            TimerType::PSHardReset => TIMER::after_millis(30),
            TimerType::PSSourceOffSpr => TIMER::after_millis(835),
            TimerType::PSSourceOffEpr => TIMER::after_millis(1260),
            TimerType::PSSourceOnSpr => TIMER::after_millis(435),
            TimerType::PSTransitionSpr => TIMER::after_millis(500),
            TimerType::PSTransitionEpr => TIMER::after_millis(925),
            TimerType::SenderResponse => TIMER::after_millis(30),
            TimerType::SinkEPREnter => TIMER::after_millis(500),
            TimerType::SinkEPRKeepAlive => TIMER::after_millis(375),
            TimerType::SinkPPSPeriodic => TIMER::after_millis(5000),
            TimerType::SinkRequest => TIMER::after_millis(100),
            TimerType::SinkWaitCap => TIMER::after_millis(465),
            TimerType::SourceCapability => TIMER::after_millis(150),
            TimerType::SourceEPRKeepAlive => TIMER::after_millis(875),
            TimerType::SourcePPSComm => TIMER::after_millis(13500),
            TimerType::SinkTx => TIMER::after_millis(18),
            TimerType::SwapSourceStart => TIMER::after_millis(20),
            TimerType::VCONNDischarge => TIMER::after_millis(200),
            TimerType::VCONNOn => TIMER::after_millis(50),
            TimerType::VDMModeEntry => TIMER::after_millis(25),
            TimerType::VDMModeExit => TIMER::after_millis(25),
            TimerType::VDMResponse => TIMER::after_millis(27),
        }
    }
}

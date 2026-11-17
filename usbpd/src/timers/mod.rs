//! Timers that are used by the protocol layer and policy engine.

/// The timer trait to implement by the user application.
pub trait Timer {
    /// Expire after the specified number of milliseconds.
    fn after_millis(milliseconds: u64) -> impl Future<Output = ()>;
}

use core::future::Future;

/// Types of timers that are used for timeouts.
///
/// Timer names, parameters and values per PD 3.2 Tab. 6.69 and Tab. 6.68.
///
/// Note: Timers for features not implemented by this crate (BIST, Data Reset,
/// non-chunking ports, SOP'' cable-plug communication) are intentionally omitted.
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum TimerType {
    /// ChunkSenderRequestTimer (tChunkSenderRequest, 24–30 ms).
    ChunkSenderRequest,
    /// ChunkSenderResponseTimer (tChunkSenderResponse, 24–30 ms).
    ChunkSenderResponse,
    /// CRCReceiveTimer (tReceive, 0.9–1.1 ms).
    CRCReceive,
    /// DiscoverIdentityTimer (tDiscoverIdentity, 40–50 ms).
    DiscoverIdentity,
    /// HardResetCompleteTimer (tHardResetComplete, 4–5 ms).
    HardResetComplete,
    /// NoResponseTimer (tNoResponse, 4.5–5.5 s).
    NoResponse,
    /// PSHardResetTimer (tPSHardReset, 25–35 ms).
    PSHardReset,
    /// PSSourceOffTimer, SPR Mode (tPSSourceOff, 750–920 ms).
    PSSourceOffSpr,
    /// PSSourceOffTimer, EPR Mode (tPSSourceOff, 1120–1400 ms).
    #[cfg(feature = "epr")]
    PSSourceOffEpr,
    /// PSSourceOnTimer, SPR Mode (tPSSourceOn, 390–480 ms).
    PSSourceOnSpr,
    /// PSTransitionTimer, SPR Mode (tPSTransition, 450–550 ms).
    PSTransitionSpr,
    /// PSTransitionTimer, EPR Mode (tPSTransition, 830–1020 ms).
    #[cfg(feature = "epr")]
    PSTransitionEpr,
    /// SenderResponseTimer (tSenderResponse, 27–33 ms).
    SenderResponse,
    /// SinkEPREnterTimer (tEnterEPR, 450–550 ms).
    #[cfg(feature = "epr")]
    SinkEPREnter,
    /// SinkEPRKeepAliveTimer (tSinkEPRKeepAlive, 0.25–0.5 s).
    #[cfg(feature = "epr")]
    SinkEPRKeepAlive,
    /// SinkPPSPeriodicTimer (tPPSRequest, max 10 s).
    SinkPPSPeriodic,
    /// SinkRequestTimer (tSinkRequest, min 100 ms).
    SinkRequest,
    /// SinkWaitCapTimer (tTypeCSinkWaitCap, 310–620 ms).
    SinkWaitCap,
    /// SourceCapabilityTimer (tTypeCSendSourceCap, 100–200 ms).
    SourceCapability,
    /// SourceEPRKeepAliveTimer (tSourceEPRKeepAlive, 0.75–1.0 s).
    #[cfg(feature = "epr")]
    SourceEPRKeepAlive,
    /// SourcePPSCommTimer (tPPSTimeout, 12–15 s).
    SourcePPSComm,
    /// SwapSourceStartTimer (tSwapSourceStart, min 20 ms).
    SwapSourceStart,
    /// VCONNDischargeTimer (tVCONNSourceDischarge, 160–240 ms).
    VCONNDischarge,
    /// VCONNOnTimer (tVCONNSourceTimeout, 100–200 ms).
    VCONNOn,
    /// VDMModeEntryTimer (tVDMWaitModeEntry, 40–50 ms).
    VDMModeEntry,
    /// VDMModeExitTimer (tVDMWaitModeExit, 40–50 ms).
    VDMModeExit,
    /// VDMResponseTimer (tVDMSenderResponse, 24–30 ms).
    VDMResponse,
}

impl TimerType {
    /// Create a new timer for a given type.
    ///
    /// Per PD 3.2 Tab. 6.68, the timeout is a fixed value within the specified range.
    pub fn get_timer<TIMER: Timer>(timer_type: TimerType) -> impl Future<Output = ()> {
        match timer_type {
            TimerType::ChunkSenderRequest => TIMER::after_millis(27),
            TimerType::ChunkSenderResponse => TIMER::after_millis(27),
            TimerType::CRCReceive => TIMER::after_millis(1),
            TimerType::DiscoverIdentity => TIMER::after_millis(45),
            TimerType::HardResetComplete => TIMER::after_millis(5),
            TimerType::NoResponse => TIMER::after_millis(5000),
            TimerType::PSHardReset => TIMER::after_millis(30),
            TimerType::PSSourceOffSpr => TIMER::after_millis(835),
            #[cfg(feature = "epr")]
            TimerType::PSSourceOffEpr => TIMER::after_millis(1260),
            TimerType::PSSourceOnSpr => TIMER::after_millis(435),
            TimerType::PSTransitionSpr => TIMER::after_millis(500),
            #[cfg(feature = "epr")]
            TimerType::PSTransitionEpr => TIMER::after_millis(925),
            TimerType::SenderResponse => TIMER::after_millis(30),
            #[cfg(feature = "epr")]
            TimerType::SinkEPREnter => TIMER::after_millis(500),
            #[cfg(feature = "epr")]
            TimerType::SinkEPRKeepAlive => TIMER::after_millis(375),
            TimerType::SinkPPSPeriodic => TIMER::after_millis(5000), // Max. 10 s
            TimerType::SinkRequest => TIMER::after_millis(100),
            TimerType::SinkWaitCap => TIMER::after_millis(465),
            TimerType::SourceCapability => TIMER::after_millis(150),
            #[cfg(feature = "epr")]
            TimerType::SourceEPRKeepAlive => TIMER::after_millis(875),
            TimerType::SourcePPSComm => TIMER::after_millis(13500),
            TimerType::SwapSourceStart => TIMER::after_millis(20),
            TimerType::VCONNDischarge => TIMER::after_millis(200),
            TimerType::VCONNOn => TIMER::after_millis(150),
            TimerType::VDMModeEntry => TIMER::after_millis(45),
            TimerType::VDMModeExit => TIMER::after_millis(45),
            TimerType::VDMResponse => TIMER::after_millis(27),
        }
    }
}

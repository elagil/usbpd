//! Timers that are used by the protocol layer and policy engine.
pub trait Timer {
    fn after_millis(milliseconds: u64) -> impl Future<Output = ()>;
}

use core::future::Future;
#[allow(dead_code)]
#[derive(Debug)]
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
            TimerType::SenderResponse => TIMER::after_millis(300),
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

#[allow(non_upper_case_globals)]
#[allow(dead_code)]
#[cfg(none)]
mod timer_values {
    const tACTemoUpdate: Timer = TIMER::from_millis(500);
    // pub(crate) const tBISTContMode: Timer = TIMER::from_millis(45);
    const tBISTCarrierMode: Timer = TIMER::from_millis(300);
    const tBISTSharedTestMode: Timer = TIMER::from_secs(1);
    const tCableMessage: Timer = TIMER::from_micros(750);
    // pub(crate) const tChunkingNotSupported: Timer = TIMER::from_millis(45);
    const tChunkReceiverRequest: Timer = TIMER::from_millis(15);
    const tChunkReceiverResponse: Timer = TIMER::from_millis(15);
    // pub(crate) const tChunkSenderRequest: Timer = TIMER::from_millis(27);
    // pub(crate) const tChunkSenderResponse: Timer = TIMER::from_millis(27);
    const tDataReset: Timer = TIMER::from_millis(225);
    // pub(crate) const tDataResetFail: Timer = TIMER::from_millis(350);
    // pub(crate) const tDataResetFailUFP: Timer = TIMER::from_millis(500);
    // pub(crate) const tDiscoverIdentity: Timer = TIMER::from_millis(45);
    const tDRSwapHardReset: Timer = TIMER::from_millis(15);
    const tDRSwapWait: Timer = TIMER::from_millis(100);
    const tEnterUSB: Timer = TIMER::from_millis(500);
    const tEnterUSBWait: Timer = TIMER::from_millis(100);
    // pub(crate) const tEnterEPR: Timer = TIMER::from_millis(500);
    const tEPRSourceCableDiscovery: Timer = TIMER::from_secs(2);
    const tFirstSourceCap: Timer = TIMER::from_millis(250);
    const tFRSwap5V: Timer = TIMER::from_millis(15);
    const tFRSwapComplete: Timer = TIMER::from_millis(15);
    const tFRSwapInit: Timer = TIMER::from_millis(15);
    const tHardReset: Timer = TIMER::from_millis(5);
    // pub(crate) const tHardResetComplete: Timer = TIMER::from_millis(5);
    // pub(crate) const tSourceEPRKeepAlive: Timer = TIMER::from_millis(875);
    // pub(crate) const tSinkEPRKeepAlive: Timer = TIMER::from_millis(375);
    // pub(crate) const tNoResponse: Timer = TIMER::from_secs(5);
    // pub(crate) const tPPSRequest: Timer = TIMER::from_secs(5); // Max is 10 seconds.
    const tPPSTimeout: Timer = TIMER::from_secs(13);
    const tProtErrHardReset: Timer = TIMER::from_millis(15);
    const tProtErrSoftReset: Timer = TIMER::from_millis(15);
    const tPRSwapWait: Timer = TIMER::from_millis(100);
    // pub(crate) const tPSHardReset: Timer = TIMER::from_millis(30);
    // pub(crate) const tPSSourceOffSPR: Timer = TIMER::from_millis(835);
    // const tPSSourceOffEPR: Timer = TIMER::from_millis(1260);
    // pub(crate) const tPSSourceOnSPR: Timer = TIMER::from_millis(435);
    // pub(crate) const tPSTransitionSPR: Timer = TIMER::from_millis(500);
    // const tPSTransitionEPR: Timer = TIMER::from_millis(925);
    // pub(crate) const tReceive: Timer = TIMER::from_millis(1);
    const tReceiverResponse: Timer = TIMER::from_millis(15);
    const tRetry: Timer = TIMER::from_micros(195);
    // pub(crate) const tSenderResponse: Timer = TIMER::from_millis(30);
    const tSinkDelay: Timer = TIMER::from_millis(5);
    // pub(crate) const tSinkRequest: Timer = TIMER::from_millis(100);
    const tSinkTx: Timer = TIMER::from_millis(18);
    const tSoftReset: Timer = TIMER::from_millis(15);
    const tSrcHoldsBus: Timer = TIMER::from_millis(50);
    const tSwapSinkReady: Timer = TIMER::from_millis(15);
    const tSwapSourceStart: Timer = TIMER::from_millis(20);
    const tTransmit: Timer = TIMER::from_micros(195);
    // pub(crate) const tTypeCSendSourceCap: Timer = TIMER::from_millis(150);
    // pub(crate) const tTypeCSinkWaitCap: Timer = TIMER::from_millis(465);
    const tVCONNSourceDischarge: Timer = TIMER::from_millis(200);
    const tVCONNSourceOff: Timer = TIMER::from_millis(25);
    const tVCONNSourceOn: Timer = TIMER::from_millis(50);
    const tVCONNSourceTimeout: Timer = TIMER::from_millis(150);
    const tVCONNSwapWait: Timer = TIMER::from_millis(100);
    const tVDMBusy: Timer = TIMER::from_millis(50);
    const tVDMEnterMode: Timer = TIMER::from_millis(25);
    const tVDMExitMode: Timer = TIMER::from_millis(25);
    const tVDMReceiverResponse: Timer = TIMER::from_millis(15);
    const tVDMSenderResponse: Timer = TIMER::from_millis(27);
    const tVDMWaitModeEntry: Timer = TIMER::from_millis(45);
    const tVDMWaitModeExit: Timer = TIMER::from_millis(45);
}

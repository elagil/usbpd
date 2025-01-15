use embassy_time::{Duration, Instant};

#[allow(dead_code)]
#[derive(Debug)]
pub enum TimerTypes {
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
    PSSourceOff,
    PSSourceOn,
    PSTransition,
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

impl From<TimerTypes> for Timer {
    fn from(ty: TimerTypes) -> Self {
        match ty {
            TimerTypes::BISTContMode => Self::new(ty, timer_values::tBISTContMode),
            TimerTypes::ChunkingNotSupported => Self::new(ty, timer_values::tChunkingNotSupported),
            TimerTypes::ChunkSenderRequest => Self::new(ty, timer_values::tChunkSenderRequest),
            TimerTypes::ChunkSenderResponse => Self::new(ty, timer_values::tChunkSenderResponse),
            TimerTypes::CRCReceive => Self::new(ty, timer_values::tReceive),
            TimerTypes::DataResetFail => Self::new(ty, timer_values::tDataResetFail),
            TimerTypes::DataResetFailUFP => Self::new(ty, timer_values::tDataResetFailUFP),
            TimerTypes::DiscoverIdentity => Self::new(ty, timer_values::tDiscoverIdentity),
            TimerTypes::HardResetComplete => Self::new(ty, timer_values::tHardResetComplete),
            TimerTypes::NoResponse => Self::new(ty, timer_values::tNoResponse),
            TimerTypes::PSHardReset => Self::new(ty, timer_values::tPSHardReset),
            TimerTypes::PSSourceOff => Self::new(ty, timer_values::tPSSourceOffSPR),
            TimerTypes::PSSourceOn => Self::new(ty, timer_values::tPSSourceOnSPR),
            TimerTypes::PSTransition => Self::new(ty, timer_values::tPSTransitionSPR),
            TimerTypes::SenderResponse => Self::new(ty, timer_values::tSenderResponse),
            TimerTypes::SinkEPREnter => Self::new(ty, timer_values::tEnterEPR),
            TimerTypes::SinkEPRKeepAlive => Self::new(ty, timer_values::tSinkEPRKeepAlive),
            TimerTypes::SinkPPSPeriodic => Self::new(ty, timer_values::tPPSRequest),
            TimerTypes::SinkRequest => Self::new(ty, timer_values::tSinkRequest),
            TimerTypes::SinkWaitCap => Self::new(ty, timer_values::tTypeCSinkWaitCap),
            TimerTypes::SourceCapability => Self::new(ty, timer_values::tTypeCSendSourceCap),
            TimerTypes::SourceEPRKeepAlive => Self::new(ty, timer_values::tSourceEPRKeepAlive),
            TimerTypes::SourcePPSComm => todo!(),
            TimerTypes::SinkTx => todo!(),
            TimerTypes::SwapSourceStart => todo!(),
            TimerTypes::VCONNDischarge => todo!(),
            TimerTypes::VCONNOn => todo!(),
            TimerTypes::VDMModeEntry => todo!(),
            TimerTypes::VDMModeExit => todo!(),
            TimerTypes::VDMResponse => todo!(),
        }
    }
}

#[allow(dead_code)]
pub struct Timer {
    ty: TimerTypes,
    start: Instant,
    duration: Duration,
    expires: Option<Instant>,
}

#[allow(dead_code)]
impl Timer {
    pub fn new(ty: TimerTypes, duration: Duration) -> Self {
        Self {
            ty,
            start: Instant::now(),
            duration,
            expires: Some(Instant::now().checked_add(duration).unwrap()),
        }
    }

    pub fn is_expired(&self) -> bool {
        self.expires
            .map_or(false, |expires| expires <= Instant::now())
    }

    pub fn reset(&mut self) {
        self.start = Instant::now();
        self.expires = Some(self.start.checked_add(self.duration).unwrap());
    }

    pub fn elapsed(&self) -> Duration {
        Instant::now().duration_since(self.start)
    }

    pub fn remaining(&self) -> Option<Duration> {
        self.expires
            .map(|expires| expires.duration_since(Instant::now()))
    }

    pub fn ty(&self) -> &TimerTypes {
        &self.ty
    }
}

#[allow(non_upper_case_globals)]
#[allow(dead_code)]
mod timer_values {
    use super::Duration;

    const tACTemoUpdate: Duration = Duration::from_millis(500);
    pub(crate) const tBISTContMode: Duration = Duration::from_millis(45);
    const tBISTCarrierMode: Duration = Duration::from_millis(300);
    const tBISTSharedTestMode: Duration = Duration::from_secs(1);
    const tCableMessage: Duration = Duration::from_micros(750);
    pub(crate) const tChunkingNotSupported: Duration = Duration::from_millis(45);
    const tChunkReceiverRequest: Duration = Duration::from_millis(15);
    const tChunkReceiverResponse: Duration = Duration::from_millis(15);
    pub(crate) const tChunkSenderRequest: Duration = Duration::from_millis(27);
    pub(crate) const tChunkSenderResponse: Duration = Duration::from_millis(27);
    const tDataReset: Duration = Duration::from_millis(225);
    pub(crate) const tDataResetFail: Duration = Duration::from_millis(350);
    pub(crate) const tDataResetFailUFP: Duration = Duration::from_millis(500);
    pub(crate) const tDiscoverIdentity: Duration = Duration::from_millis(45);
    const tDRSwapHardReset: Duration = Duration::from_millis(15);
    const tDRSwapWait: Duration = Duration::from_millis(100);
    const tEnterUSB: Duration = Duration::from_millis(500);
    const tEnterUSBWait: Duration = Duration::from_millis(100);
    pub(crate) const tEnterEPR: Duration = Duration::from_millis(500);
    const tEPRSourceCableDiscovery: Duration = Duration::from_secs(2);
    const tFirstSourceCap: Duration = Duration::from_millis(250);
    const tFRSwap5V: Duration = Duration::from_millis(15);
    const tFRSwapComplete: Duration = Duration::from_millis(15);
    const tFRSwapInit: Duration = Duration::from_millis(15);
    const tHardReset: Duration = Duration::from_millis(5);
    pub(crate) const tHardResetComplete: Duration = Duration::from_millis(5);
    pub(crate) const tSourceEPRKeepAlive: Duration = Duration::from_millis(875);
    pub(crate) const tSinkEPRKeepAlive: Duration = Duration::from_millis(375);
    pub(crate) const tNoResponse: Duration = Duration::from_secs(5);
    pub(crate) const tPPSRequest: Duration = Duration::from_secs(5); // Max is 10 seconds.
    const tPPSTimeout: Duration = Duration::from_secs(13);
    const tProtErrHardReset: Duration = Duration::from_millis(15);
    const tProtErrSoftReset: Duration = Duration::from_millis(15);
    const tPRSwapWait: Duration = Duration::from_millis(100);
    pub(crate) const tPSHardReset: Duration = Duration::from_millis(30);
    pub(crate) const tPSSourceOffSPR: Duration = Duration::from_millis(835);
    const tPSSourceOffEPR: Duration = Duration::from_millis(1260);
    pub(crate) const tPSSourceOnSPR: Duration = Duration::from_millis(435);
    pub(crate) const tPSTransitionSPR: Duration = Duration::from_millis(500);
    const tPSTransitionEPR: Duration = Duration::from_millis(925);
    pub(crate) const tReceive: Duration = Duration::from_millis(1);
    const tReceiverResponse: Duration = Duration::from_millis(15);
    const tRetry: Duration = Duration::from_micros(195);
    pub(crate) const tSenderResponse: Duration = Duration::from_millis(30);
    const tSinkDelay: Duration = Duration::from_millis(5);
    pub(crate) const tSinkRequest: Duration = Duration::from_millis(100);
    const tSinkTx: Duration = Duration::from_millis(18);
    const tSoftReset: Duration = Duration::from_millis(15);
    const tSrcHoldsBus: Duration = Duration::from_millis(50);
    const tSwapSinkReady: Duration = Duration::from_millis(15);
    const tSwapSourceStart: Duration = Duration::from_millis(20);
    const tTransmit: Duration = Duration::from_micros(195);
    pub(crate) const tTypeCSendSourceCap: Duration = Duration::from_millis(150);
    pub(crate) const tTypeCSinkWaitCap: Duration = Duration::from_millis(465);
    const tVCONNSourceDischarge: Duration = Duration::from_millis(200);
    const tVCONNSourceOff: Duration = Duration::from_millis(25);
    const tVCONNSourceOn: Duration = Duration::from_millis(50);
    const tVCONNSourceTimeout: Duration = Duration::from_millis(150);
    const tVCONNSwapWait: Duration = Duration::from_millis(100);
    const tVDMBusy: Duration = Duration::from_millis(50);
    const tVDMEnterMode: Duration = Duration::from_millis(25);
    const tVDMExitMode: Duration = Duration::from_millis(25);
    const tVDMReceiverResponse: Duration = Duration::from_millis(15);
    const tVDMSenderResponse: Duration = Duration::from_millis(27);
    const tVDMWaitModeEntry: Duration = Duration::from_millis(45);
    const tVDMWaitModeExit: Duration = Duration::from_millis(45);
}

//! EPR-specific timers.
//!
//! EPR timer names, parameters and values per PD 3.2 Tab. 6.69 and Tab. 6.68.

/// Types of EPR timers that are used for timeouts.
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum EprTimerType {
    /// PSSourceOffTimer, EPR Mode (tPSSourceOff, 1120–1400 ms).
    PSSourceOff,
    /// PSTransitionTimer, EPR Mode (tPSTransition, 830–1020 ms).
    PSTransition,
    /// SinkEPREnterTimer (tEnterEPR, 450–550 ms).
    SinkEPREnter,
    /// SinkEPRKeepAliveTimer (tSinkEPRKeepAlive, 0.25–0.5 s).
    SinkEPRKeepAlive,
    /// SourceEPRKeepAliveTimer (tSourceEPRKeepAlive, 0.75–1.0 s).
    SourceEPRKeepAlive,
}

impl EprTimerType {
    /// The timeout in milliseconds.
    ///
    /// Per PD 3.2 Tab. 6.68, the timeout is a fixed value within the specified range.
    pub fn millis(timer_type: EprTimerType) -> u64 {
        match timer_type {
            EprTimerType::PSSourceOff => 1260,
            EprTimerType::PSTransition => 925,
            EprTimerType::SinkEPREnter => 500,
            EprTimerType::SinkEPRKeepAlive => 375,
            EprTimerType::SourceEPRKeepAlive => 875,
        }
    }
}

//! Policy engine for the implementation of a sink.
use core::marker::PhantomData;

use defmt::{debug, error, trace, Format};

use crate::protocol_layer::message::header::{
    ControlMessageType, DataMessageType, Header, MessageType, SpecificationRevision,
};
use crate::protocol_layer::{Error as ProtocolError, ProtocolLayer};
use crate::timers::{Timer, TimerType};
use crate::{DataRole, Driver, PowerRole};

/// Sink states.
#[derive(Debug, Clone, Copy, Format)]
enum State {
    /// Default state at startup.
    Startup,
    Discovery,
    WaitForCapabilities,
    EvaluateCapability,
    SelectCapability,
    TransitionSink,
    Ready,
    HardReset,
    TransitionToDefault,
    GiveSinkCap,
    GetSourceCap,
    EPRKeepAlive,
}

#[derive(Debug)]
pub struct Sink<DRIVER: Driver, TIMER: Timer> {
    protocol_layer: ProtocolLayer<DRIVER, TIMER>,
    state: State,

    _timer: PhantomData<TIMER>,
}

#[derive(Debug, Format)]
enum Error {
    Startup,
    Timeout,
    Protocol(ProtocolError),
}

impl From<ProtocolError> for Error {
    fn from(protocol_error: ProtocolError) -> Self {
        Error::Protocol(protocol_error)
    }
}

impl<DRIVER: Driver, TIMER: Timer> Sink<DRIVER, TIMER> {
    fn new_protocol_layer(driver: DRIVER) -> ProtocolLayer<DRIVER, TIMER> {
        let header = Header::new_template(DataRole::Ufp, PowerRole::Sink, SpecificationRevision::R3_0);
        ProtocolLayer::new(driver, header)
    }

    /// Create a new sink policy engine with a given `driver`.
    pub fn new(driver: DRIVER) -> Self {
        Self {
            protocol_layer: Self::new_protocol_layer(driver),
            state: State::Startup,

            _timer: PhantomData,
        }
    }

    /// Run the state machine.
    pub async fn run(&mut self) -> Result<(), ()> {
        loop {
            let result = self.update_state().await;

            match result {
                Err(Error::Protocol(error)) => {
                    error!("Protocol error {} in sink state transition", error);
                    self.state = State::HardReset;
                    panic!();
                }
                _ => (),
            }
        }
    }

    async fn update_state(&mut self) -> Result<(), Error> {
        trace!("Handle sink state: {:?}", self.state);

        let new_state = match self.state {
            State::Startup => {
                // Reset protocol layer
                self.protocol_layer.reset();

                State::Discovery
            }
            State::Discovery => {
                self.protocol_layer.wait_for_vbus().await;

                State::WaitForCapabilities
            }
            State::WaitForCapabilities => {
                let message = self.protocol_layer.wait_for_source_capabilities().await?;

                debug!("Capabilities: {}", message);

                // Transition to EvaluateCapability if
                // - SPR mode & source_capabilities message, or
                // - EPR mode & epr_source_capabilities message received

                // FIXME: Actually read message
                State::EvaluateCapability
            }
            State::EvaluateCapability => {
                // Entry: Reset HardResetCounter
                // self.hard_reset_count.reset();

                // Evaluate capabilities, and:
                // - Select suitable one, or
                // - Respond with capability mismatch
                // Transition to SelectCapability after capabilities are
                // evaluated
                State::SelectCapability
            }
            State::SelectCapability => {
                self.protocol_layer.request_power(50, 1).await?;

                // Entry: Send request, as above
                // Entry: Initialize and run SenderResponseTimer
                // Transition to
                // - HardReset, if SenderResponseTimer timeout
                // - WaitForCapabilities, if no explicit contract and reject/wait message
                //   received
                // - Ready, if explicit contract and reject/wait message received
                // - TransitionSink if accept message received

                self.protocol_layer
                    .wait_for_message(
                        MessageType::Control(ControlMessageType::Accept),
                        ProtocolLayer::<DRIVER, TIMER>::get_timer(TimerType::SenderResponse),
                    )
                    .await?;

                State::TransitionSink
            }
            State::TransitionSink => {
                // Entry: Initialize and run PSTransitionTimer
                // Exit: Request device policy manager transitions sink power
                // supply to new power (if required)
                // Transition to
                // - HardReset on protocol error??
                // - Ready after PS_RDY message received

                self.protocol_layer
                    .wait_for_message(
                        MessageType::Control(ControlMessageType::PsRdy),
                        ProtocolLayer::<DRIVER, TIMER>::get_timer(TimerType::PSTransitionSpr),
                    )
                    .await?;

                State::Ready
            }
            State::Ready => {
                // Entry: Init. and run SinkRequestTimer(2) on receiving wait
                // Entry: Init. and run DiscoverIdentityTimer(4)
                // Entry: Init. and run SinkPPSPeriodicTimer(5)
                // Entry: Init. and run SinkEPRKeppAlivaTimer(6) in EPR mode
                // Entry: Send GetSinkCap message if sink supports fast role
                // swap Exit: If initiating an AMS, notify
                // protocol layer??? Transition to
                // - EPRKeepAlive on SinkEPRKeepAliveTimer timeout
                // - EvalueCapability on SPR mode and source_capabilities message, or EPR mode
                //   and EPR_source_capabilities message
                TIMER::after_millis(2000).await;

                State::Ready
            }
            State::HardReset => {
                // Signal hard reset, increment hard reset counter
                // In protocol layer?
                // self.hard_reset_count.increment()?;

                // Other causes of entry:
                // - SinkWaitCapTimer timeout or PSTransitionTimer timeout, when reset count <
                //   max
                // - Hard reset request from device policy manager
                // - EPR mode and EPR_Source_Capabilities message with EPR PDO in pos. 1..7
                // - source_capabilities message not requested by get_source_caps
                // Transition to TransitionToDefault

                State::TransitionToDefault
            }
            State::TransitionToDefault => {
                // Entry: Request power sink transition to default
                // Entry: Reset local hardware???
                // Entry: Set port data role to UFP and turn off VConn
                // Exit: Inform protocol layer that hard reset is complete
                // Transition to Startup

                State::Startup
            }
            State::GiveSinkCap => {
                // Entry: Get present sink capabilities
                // Send capabilities message (based on device policy manager
                // response):
                // - If get_sink_cap message received, send sink_capabilities message???
                // - If EPR_get_sink_kap message received, send EPR_sink_cap message
                // Transition to Ready after sink capabilities sent (what about
                // entry/exit?)

                State::Ready
            }
            State::GetSourceCap => {
                // What is this state?
                // FIXME: wrong state
                State::SelectCapability
            }
            State::EPRKeepAlive => {
                // Entry: Send EPRKeepAlive Message
                // Entry: Init. and run SenderReponseTimer
                // Transition to
                // - Ready on EPRKeepAliveAck message
                // - HardReset on SenderResponseTimerTimeout

                State::Ready
            }
        };

        // trace!("Sink state transition: {:?} -> {:?}", self.state, new_state);
        self.state = new_state;

        Ok(())
    }
}

pub trait DevicePolicyEngine {}

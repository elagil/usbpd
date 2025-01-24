//! Policy engine for the implementation of a sink.
use core::future::Future;
use core::marker::PhantomData;

use defmt::{debug, error, trace, Format};

use crate::protocol_layer::message::header::{
    ControlMessageType, DataMessageType, Header, MessageType, SpecificationRevision,
};
use crate::protocol_layer::message::pdo::SourceCapabilities;
use crate::protocol_layer::message::{Data, Message};
use crate::protocol_layer::{message, Error as ProtocolError, ProtocolLayer};
use crate::timers::{Timer, TimerType};
use crate::{DataRole, Driver, PowerRole};

use super::device_policy_manager::DevicePolicyManager;
use super::PowerSourceRequest;

/// Sink capability
enum Mode {
    /// The classic mode of PD operation where explicit contracts are negotiaged using SPR (A)PDOs.
    Spr,
    /// A Power Delivery mode of operation where maximum allowable voltage is 48V.
    Epr,
}

/// Sink states.
#[derive(Debug, Clone, Copy, Format)]
enum State {
    /// Default state at startup.
    Startup,
    Discovery,
    WaitForCapabilities,
    EvaluateCapabilities,
    SelectCapability,
    TransitionSink,
    Ready,
    SendNotSupported,
    SendSoftReset,
    SoftReset,
    HardReset,
    TransitionToDefault,
    GiveSinkCap,
    GetSourceCap,
    EPRKeepAlive,
}

#[derive(Debug)]
pub struct Sink<DRIVER: Driver, TIMER: Timer, DPM: DevicePolicyManager> {
    device_policy_manager: DPM,
    protocol_layer: Option<ProtocolLayer<DRIVER, TIMER>>,
    state: State,
    source_capabilities: Option<SourceCapabilities>,
    power_source_request: Option<PowerSourceRequest>,
    has_explicit_contract: bool,

    _timer: PhantomData<TIMER>,
}

#[derive(Debug, Format)]
enum Error {
    Protocol(ProtocolError),
}

impl From<ProtocolError> for Error {
    fn from(protocol_error: ProtocolError) -> Self {
        Error::Protocol(protocol_error)
    }
}

impl<DRIVER: Driver, TIMER: Timer, DPM: DevicePolicyManager> Sink<DRIVER, TIMER, DPM> {
    fn new_protocol_layer(driver: DRIVER) -> ProtocolLayer<DRIVER, TIMER> {
        let header = Header::new_template(DataRole::Ufp, PowerRole::Sink, SpecificationRevision::R3_0);
        ProtocolLayer::new(driver, header)
    }

    fn get_timer(timer_type: TimerType) -> impl Future<Output = ()> {
        ProtocolLayer::<DRIVER, TIMER>::get_timer(timer_type)
    }

    /// Create a new sink policy engine with a given `driver`.
    pub fn new(driver: DRIVER, device_policy_manager: DPM) -> Self {
        Self {
            device_policy_manager,
            protocol_layer: Some(Self::new_protocol_layer(driver)),
            state: State::Discovery,
            source_capabilities: None,
            power_source_request: None,
            has_explicit_contract: false,

            _timer: PhantomData,
        }
    }

    fn protocol_layer(&mut self) -> &mut ProtocolLayer<DRIVER, TIMER> {
        self.protocol_layer.as_mut().unwrap()
    }

    fn reset_protocol_layer(&mut self) {
        trace!("Reset protocol layer");

        let driver = self.protocol_layer.take().unwrap().reset();
        self.protocol_layer = Some(Self::new_protocol_layer(driver));
    }

    /// Run the sink's state machine.
    pub async fn run(&mut self) {
        loop {
            let result = self.update_state().await;
            if result.is_ok() {
                continue;
            }

            if let Err(Error::Protocol(protocol_error)) = result {
                self.state = match (self.state, protocol_error) {
                    // Handle when hard reset is signaled by the driver itself.
                    (
                        _,
                        ProtocolError::RxError(crate::RxError::HardReset)
                        | ProtocolError::TxError(crate::TxError::HardReset),
                    ) => State::HardReset,

                    // Handle when soft reset is signaled by the driver itself.
                    (_, ProtocolError::SoftReset) => State::SoftReset,

                    // Unexpected messages indicate a protocol error and demand a soft reset.
                    // See spec, [6.8.1]
                    (_, ProtocolError::UnexpectedMessage) => State::SendSoftReset,

                    // Fall back to hard reset, after soft reset accept failed to be sent.
                    (State::SoftReset, ProtocolError::TxError(_)) => State::HardReset,

                    // Fall back to hard reset, after sending soft reset failed.
                    (State::SendSoftReset, ProtocolError::Timeout | ProtocolError::TxError(_)) => State::HardReset,

                    (State::SelectCapability, ProtocolError::Timeout) => State::HardReset,

                    // Timeouts increase the retry count within the protocol layer.
                    (_, ProtocolError::Timeout) => self.state,

                    (State::Ready, ProtocolError::UnsupportedMessage) => State::SendNotSupported,

                    (_, ProtocolError::Counter(crate::counters::Error::Overrun)) => State::SendSoftReset,

                    // Attempt to recover protocol errors with a soft reset.
                    (_, error) => {
                        error!("Protocol error {} in sink state transition", error);
                        self.state
                    }
                };
            } else {
                debug!("Result {} in sink state transition", result);
                unimplemented!()
            }
        }
    }

    async fn update_state(&mut self) -> Result<(), Error> {
        trace!("Handle sink state: {:?}", self.state);

        let new_state = match self.state {
            State::Startup => {
                // Reset protocol layer
                self.reset_protocol_layer();

                State::Discovery
            }
            State::Discovery => {
                self.protocol_layer().wait_for_vbus().await;

                State::WaitForCapabilities
            }
            State::WaitForCapabilities => {
                // FIXME: Add support for EPR sources?
                let message = self.protocol_layer().wait_for_source_capabilities().await?;
                trace!("Capabilities: {}", message);

                let Some(Data::SourceCapabilities(capabilities)) = message.data else {
                    unreachable!()
                };
                self.source_capabilities = Some(capabilities);

                State::EvaluateCapabilities
            }
            State::EvaluateCapabilities => {
                // FIXME: Reset HardResetCounter
                // self.hard_reset_count.reset();

                // Evaluate capabilities, and:
                // - Select suitable one, or
                // - Respond with capability mismatch
                // Transition to SelectCapability after capabilities are
                // evaluated
                self.power_source_request = self
                    .device_policy_manager
                    .request(self.source_capabilities.take().unwrap());

                State::SelectCapability
            }
            State::SelectCapability => {
                let request = self.power_source_request.take().unwrap();
                self.protocol_layer().request_supply(request).await?;

                let message_type = self
                    .protocol_layer()
                    .wait_for_any_message(
                        &[
                            MessageType::Control(ControlMessageType::Accept),
                            MessageType::Control(ControlMessageType::Wait),
                            MessageType::Control(ControlMessageType::Reject),
                        ],
                        Self::get_timer(TimerType::SenderResponse),
                    )
                    .await?
                    .header
                    .message_type();

                let MessageType::Control(control_message_type) = message_type else {
                    unreachable!()
                };

                match (self.has_explicit_contract, control_message_type) {
                    (_, ControlMessageType::Accept) => State::TransitionSink,
                    (false, ControlMessageType::Wait | ControlMessageType::Reject) => State::WaitForCapabilities,
                    (true, ControlMessageType::Wait | ControlMessageType::Reject) => State::Ready,
                    _ => unreachable!(),
                }
            }
            State::TransitionSink => {
                // Entry: Initialize and run PSTransitionTimer
                // Exit: Request device policy manager transitions sink power
                // supply to new power (if required)
                // Transition to
                // - HardReset on protocol error??
                // - Ready after PS_RDY message received

                self.protocol_layer()
                    .wait_for_any_message(
                        &[MessageType::Control(ControlMessageType::PsRdy)],
                        Self::get_timer(TimerType::PSTransitionSpr),
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
                let message = self.protocol_layer().receive().await?;

                match message.header.message_type() {
                    MessageType::Data(DataMessageType::SourceCapabilities) => {
                        let Some(Data::SourceCapabilities(capabilities)) = message.data else {
                            unreachable!()
                        };
                        self.source_capabilities = Some(capabilities);
                        State::EvaluateCapabilities
                    }
                    _ => State::SendNotSupported,
                }
            }
            State::SendNotSupported => {
                self.protocol_layer()
                    .transmit_control_message(ControlMessageType::NotSupported)
                    .await?;

                State::Ready
            }
            State::SendSoftReset => {
                self.reset_protocol_layer();

                self.protocol_layer()
                    .transmit_control_message(ControlMessageType::SoftReset)
                    .await?;

                self.protocol_layer()
                    .wait_for_any_message(
                        &[MessageType::Control(ControlMessageType::Accept)],
                        Self::get_timer(TimerType::SenderResponse),
                    )
                    .await?;

                State::WaitForCapabilities
            }
            State::SoftReset => {
                self.protocol_layer()
                    .transmit_control_message(ControlMessageType::Accept)
                    .await?;

                self.reset_protocol_layer();

                State::WaitForCapabilities
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

//! Policy engine for the implementation of a sink.
use core::marker::PhantomData;

use futures::future::{select, Either};
use futures::pin_mut;

use super::device_policy_manager::DevicePolicyManager;
use super::PowerSourceRequest;
use crate::counters::{Counter, Error as CounterError};
use crate::protocol_layer::message::header::{
    ControlMessageType, DataMessageType, Header, MessageType, SpecificationRevision,
};
use crate::protocol_layer::message::pdo::SourceCapabilities;
use crate::protocol_layer::message::Data;
use crate::protocol_layer::{Error as ProtocolError, ProtocolLayer};
use crate::timers::{Timer, TimerType};
use crate::{DataRole, Driver, PowerRole};

/// Sink capability
///
/// FIXME: Support EPR.
enum _Mode {
    /// The classic mode of PD operation where explicit contracts are negotiaged using SPR (A)PDOs.
    Spr,
    /// A Power Delivery mode of operation where maximum allowable voltage is 48V.
    Epr,
}

#[derive(Debug, Clone, Copy)]
enum Contract {
    Safe5V,
    _Implicit, // FIXME: When does an implicit contract exist?
    Transition,
    Explicit,
}

impl Default for Contract {
    fn default() -> Self {
        Contract::Safe5V
    }
}

/// Sink states.
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
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
    _GetSourceCap, // FIXME: no EPR support
    _EPRKeepAlive, // FIXME: no EPR support
}

/// Implementation of the sink policy engine.
/// See spec, [8.3.3.3]
#[derive(Debug)]
pub struct Sink<DRIVER: Driver, TIMER: Timer, DPM: DevicePolicyManager> {
    device_policy_manager: DPM,
    protocol_layer: ProtocolLayer<DRIVER, TIMER>,

    source_capabilities: Option<SourceCapabilities>,
    power_source_request: Option<PowerSourceRequest>,
    contract: Contract,
    hard_reset_counter: Counter,

    state: State,

    _timer: PhantomData<TIMER>,
}

#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
enum Error {
    PortPartnerUnresponsive,
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

    /// Create a new sink policy engine with a given `driver`.
    pub fn new(driver: DRIVER, device_policy_manager: DPM) -> Self {
        Self {
            device_policy_manager,
            protocol_layer: Self::new_protocol_layer(driver),
            state: State::Discovery,
            source_capabilities: None,
            power_source_request: None,
            contract: Default::default(),
            hard_reset_counter: Counter::new(crate::counters::CounterType::HardReset),

            _timer: PhantomData,
        }
    }

    /// Set a new driver when re-attached.
    pub fn re_attach(&mut self, driver: DRIVER) {
        self.protocol_layer = Self::new_protocol_layer(driver);
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
                    (_, ProtocolError::HardReset) => State::TransitionToDefault,

                    // Handle when soft reset is signaled by the driver itself.
                    (_, ProtocolError::SoftReset) => State::SoftReset,

                    // Unexpected messages indicate a protocol error and demand a soft reset.
                    // See spec, [6.8.1]
                    (_, ProtocolError::UnexpectedMessage) => State::SendSoftReset,

                    // Fall back to hard reset
                    // - after soft reset accept failed to be sent, or
                    // - after sending soft reset failed.
                    (State::SoftReset | State::SendSoftReset, ProtocolError::TransmitRetriesExceeded) => {
                        State::HardReset
                    }

                    // See spec, [8.3.3.3.3]
                    (State::WaitForCapabilities, ProtocolError::ReceiveTimeout) => State::HardReset,

                    // See spec, [8.3.3.3.5]
                    (State::SelectCapability, ProtocolError::ReceiveTimeout) => State::HardReset,

                    // See spec, [8.3.3.3.6]
                    (State::TransitionSink, _) => State::HardReset,

                    (State::Ready, ProtocolError::UnsupportedMessage) => State::SendNotSupported,

                    (_, ProtocolError::TransmitRetriesExceeded) => State::SendSoftReset,

                    // Attempt to recover protocol errors with a soft reset.
                    (_, error) => {
                        error!("Protocol error {} in sink state transition", error);
                        self.state
                    }
                };
            } else {
                debug!("Result {} in sink state transition", result);
            }
        }
    }

    async fn get_source_capabilities(&mut self) -> Result<(), Error> {
        let message = self.protocol_layer.wait_for_source_capabilities().await?;
        trace!("Source capabilities: {}", message);

        let Some(Data::SourceCapabilities(capabilities)) = message.data else {
            unreachable!()
        };
        self.source_capabilities = Some(capabilities);

        Ok(())
    }

    async fn update_state(&mut self) -> Result<(), Error> {
        trace!("Handle sink state: {:?}", self.state);

        let new_state = match self.state {
            State::Startup => {
                self.contract = Default::default();
                self.protocol_layer.reset();

                State::Discovery
            }
            State::Discovery => {
                self.protocol_layer.wait_for_vbus().await;

                State::WaitForCapabilities
            }
            State::WaitForCapabilities => {
                self.get_source_capabilities().await?;

                State::EvaluateCapabilities
            }
            State::EvaluateCapabilities => {
                // Sink now knows that it is attached.

                self.hard_reset_counter.reset();

                self.power_source_request = self
                    .device_policy_manager
                    .request(self.source_capabilities.take().unwrap())
                    .await;

                State::SelectCapability
            }
            State::SelectCapability => {
                match self.power_source_request.take() {
                    Some(request) => self.protocol_layer.request_power(request).await?,
                    None => {
                        // FIXME: Send capability mismatch
                        unimplemented!()
                    }
                }

                let message_type = self
                    .protocol_layer
                    .receive_message_type(
                        &[
                            MessageType::Control(ControlMessageType::Accept),
                            MessageType::Control(ControlMessageType::Wait),
                            MessageType::Control(ControlMessageType::Reject),
                        ],
                        TimerType::SenderResponse,
                    )
                    .await?
                    .header
                    .message_type();

                let MessageType::Control(control_message_type) = message_type else {
                    unreachable!()
                };

                match (self.contract, control_message_type) {
                    (_, ControlMessageType::Accept) => State::TransitionSink,
                    (Contract::Safe5V, ControlMessageType::Wait | ControlMessageType::Reject) => {
                        State::WaitForCapabilities
                    }
                    (Contract::Explicit, ControlMessageType::Reject | ControlMessageType::Wait) => State::Ready,
                    _ => unreachable!(),
                }
            }
            State::TransitionSink => {
                self.protocol_layer
                    .receive_message_type(
                        &[MessageType::Control(ControlMessageType::PsRdy)],
                        TimerType::PSTransitionSpr,
                    )
                    .await?;

                self.contract = Contract::Transition;
                self.device_policy_manager.transition_power().await;
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
                self.contract = Contract::Explicit;

                let receive_fut = self.protocol_layer.receive_message();
                let dpm_event_fut = self.device_policy_manager.get_event();

                pin_mut!(receive_fut);
                pin_mut!(dpm_event_fut);

                match select(receive_fut, dpm_event_fut).await {
                    Either::Left((message, _)) => {
                        let message = message?;

                        match message.header.message_type() {
                            MessageType::Data(DataMessageType::SourceCapabilities) => {
                                let Some(Data::SourceCapabilities(capabilities)) = message.data else {
                                    unreachable!()
                                };
                                self.source_capabilities = Some(capabilities);
                                State::EvaluateCapabilities
                            }
                            MessageType::Control(ControlMessageType::GetSinkCap) => State::GiveSinkCap,
                            _ => State::SendNotSupported,
                        }
                    }
                    Either::Right((_event, _)) => {
                        debug!("Got event from DPM: {}", _event);
                        // FIXME: Evaluate events from DPM.
                        // E.g. request source cap, select capability, PPS
                        State::Ready
                    }
                }
            }
            State::SendNotSupported => {
                self.protocol_layer
                    .transmit_control_message(ControlMessageType::NotSupported)
                    .await?;

                State::Ready
            }
            State::SendSoftReset => {
                self.protocol_layer.reset();

                self.protocol_layer
                    .transmit_control_message(ControlMessageType::SoftReset)
                    .await?;

                self.protocol_layer
                    .receive_message_type(
                        &[MessageType::Control(ControlMessageType::Accept)],
                        TimerType::SenderResponse,
                    )
                    .await?;

                State::WaitForCapabilities
            }
            State::SoftReset => {
                self.protocol_layer
                    .transmit_control_message(ControlMessageType::Accept)
                    .await?;

                self.protocol_layer.reset();

                State::WaitForCapabilities
            }
            State::HardReset => {
                // Other causes of entry:
                // - SinkWaitCapTimer timeout or PSTransitionTimer timeout, when reset count <
                //   max
                // - Hard reset request from device policy manager
                // - EPR mode and EPR_Source_Capabilities message with EPR PDO in pos. 1..7
                // - source_capabilities message not requested by get_source_caps
                // Transition to TransitionToDefault
                match self.hard_reset_counter.increment() {
                    Ok(_) => self.protocol_layer.hard_reset().await?,

                    // FIXME: Only unresponsive if WaitCapTimer timed out
                    Err(CounterError::Exceeded) => return Err(Error::PortPartnerUnresponsive),
                }

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
                // FIXME: Send sink capabilities, as provided by device policy manager.
                // Sending NotSupported is not to spec.
                // See spec, [6.4.1.6]
                self.protocol_layer
                    .transmit_control_message(ControlMessageType::NotSupported)
                    .await?;

                State::Ready
            }
            State::_GetSourceCap => {
                // Commonly used for switching between EPR and SPR mode.
                // FIXME: EPR is not supported.

                self.protocol_layer
                    .transmit_control_message(ControlMessageType::GetSourceCap)
                    .await?;

                self.get_source_capabilities().await?;

                // Return to `Ready` state instead, when
                // - in EPR mode, and SPR capabilities requested, or
                // - in SPR mode, and EPR capabilities requested.
                // In other words, in case of a switch.
                State::EvaluateCapabilities
            }
            State::_EPRKeepAlive => {
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

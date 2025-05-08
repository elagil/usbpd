//! Policy engine for the implementation of a sink.
use core::marker::PhantomData;

use futures::future::{select, Either};
use futures::pin_mut;

use super::device_policy_manager::DevicePolicyManager;
use crate::counters::{Counter, Error as CounterError};
use crate::protocol_layer::message::header::{
    ControlMessageType, DataMessageType, Header, MessageType, SpecificationRevision,
};
use crate::protocol_layer::message::pdo::SourceCapabilities;
use crate::protocol_layer::message::{request, Data};
use crate::protocol_layer::{Error as ProtocolError, ProtocolLayer};
use crate::sink::device_policy_manager::Event;
use crate::timers::{Timer, TimerType};
use crate::{DataRole, PowerRole};
use usbpd_traits::Driver;

/// Sink capability
///
/// FIXME: Support EPR.
enum _Mode {
    /// The classic mode of PD operation where explicit contracts are negotiaged using SPR (A)PDOs.
    Spr,
    /// A Power Delivery mode of operation where maximum allowable voltage is 48V.
    Epr,
}

#[derive(Debug, Clone, Copy, Default)]
enum Contract {
    #[default]
    Safe5V,
    _Implicit, // FIXME: Only present after fast role swap, yet unsupported. Limited to max. type C current.
    TransitionToExplicit,
    Explicit,
}

/// Sink states.
#[derive(Debug, Clone)]
enum State {
    /// Default state at startup.
    Startup,
    Discovery,
    WaitForCapabilities,
    EvaluateCapabilities(SourceCapabilities),
    SelectCapability(request::PowerSource),
    TransitionSink(request::PowerSource),
    Ready,
    SendNotSupported,
    SendSoftReset,
    SoftReset,
    HardReset,
    TransitionToDefault,
    GiveSinkCap,
    _GetSourceCap, // FIXME: no EPR support
    _EPRKeepAlive, // FIXME: no EPR support

    // States for reacting to DPM events
    EventRequestSourceCapabilities,
}

/// Implementation of the sink policy engine.
/// See spec, [8.3.3.3]
#[derive(Debug)]
pub struct Sink<DRIVER: Driver, TIMER: Timer, DPM: DevicePolicyManager> {
    device_policy_manager: DPM,
    protocol_layer: ProtocolLayer<DRIVER, TIMER>,
    contract: Contract,
    hard_reset_counter: Counter,
    source_capabilities: Option<SourceCapabilities>,

    state: State,

    _timer: PhantomData<TIMER>,
}

/// Errors that can occur in the sink policy engine state machine.
#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Error {
    /// The port partner is unresponsive.
    PortPartnerUnresponsive,
    /// A protocol error has occured.
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
            contract: Default::default(),
            hard_reset_counter: Counter::new(crate::counters::CounterType::HardReset),
            source_capabilities: None,

            _timer: PhantomData,
        }
    }

    /// Set a new driver when re-attached.
    pub fn re_attach(&mut self, driver: DRIVER) {
        self.protocol_layer = Self::new_protocol_layer(driver);
    }

    /// Run a single step in the policy engine state machine.
    async fn run_step(&mut self) -> Result<(), Error> {
        let result = self.update_state().await;
        if result.is_ok() {
            return Ok(());
        }

        if let Err(Error::Protocol(protocol_error)) = result {
            let new_state = match (&self.state, protocol_error) {
                // Handle when hard reset is signaled by the driver itself.
                (_, ProtocolError::HardReset) => Some(State::TransitionToDefault),

                // Handle when soft reset is signaled by the driver itself.
                (_, ProtocolError::SoftReset) => Some(State::SoftReset),

                // Unexpected messages indicate a protocol error and demand a soft reset.
                // See spec, [6.8.1]
                (_, ProtocolError::UnexpectedMessage) => Some(State::SendSoftReset),

                // FIXME: Unexpected message in power transition -> hard reset?

                // Fall back to hard reset
                // - after soft reset accept failed to be sent, or
                // - after sending soft reset failed.
                (State::SoftReset | State::SendSoftReset, ProtocolError::TransmitRetriesExceeded) => {
                    Some(State::HardReset)
                }

                // See spec, [8.3.3.3.3]
                (State::WaitForCapabilities, ProtocolError::ReceiveTimeout) => Some(State::HardReset),

                // See spec, [8.3.3.3.5]
                (State::SelectCapability(_), ProtocolError::ReceiveTimeout) => Some(State::HardReset),

                // See spec, [8.3.3.3.6]
                (State::TransitionSink(_), _) => Some(State::HardReset),

                (State::Ready, ProtocolError::UnsupportedMessage) => Some(State::SendNotSupported),

                (_, ProtocolError::TransmitRetriesExceeded) => Some(State::SendSoftReset),

                // Attempt to recover protocol errors with a soft reset.
                (_, error) => {
                    error!("Protocol error {} in sink state transition", error);
                    None
                }
            };

            if let Some(state) = new_state {
                self.state = state
            }

            Ok(())
        } else {
            error!("Unrecoverable result {} in sink state transition", result);
            result
        }
    }

    /// Run the sink's state machine continuously.
    ///
    /// The loop is only broken for unrecoverable errors, for example if the port partner is unresponsive.
    pub async fn run(&mut self) -> Result<(), Error> {
        loop {
            self.run_step().await?;
        }
    }

    async fn wait_for_source_capabilities(&mut self) -> Result<SourceCapabilities, Error> {
        let message = self.protocol_layer.wait_for_source_capabilities().await?;
        trace!("Source capabilities: {}", message);

        let Some(Data::SourceCapabilities(capabilities)) = message.data else {
            unreachable!()
        };

        Ok(capabilities)
    }

    async fn update_state(&mut self) -> Result<(), Error> {
        let new_state = match &self.state {
            State::Startup => {
                self.contract = Default::default();
                self.protocol_layer.reset();

                State::Discovery
            }
            State::Discovery => {
                self.protocol_layer.wait_for_vbus().await;
                self.source_capabilities = None;

                State::WaitForCapabilities
            }
            State::WaitForCapabilities => State::EvaluateCapabilities(self.wait_for_source_capabilities().await?),
            State::EvaluateCapabilities(capabilities) => {
                // Sink now knows that it is attached.
                // FIXME: No clone? Size is 72 bytes.
                self.source_capabilities = Some(capabilities.clone());

                self.hard_reset_counter.reset();

                let request = self
                    .device_policy_manager
                    .request(self.source_capabilities.as_ref().unwrap())
                    .await;

                State::SelectCapability(request)
            }
            State::SelectCapability(request) => {
                self.protocol_layer.request_power(*request).await?;

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
                    (_, ControlMessageType::Accept) => State::TransitionSink(*request),
                    (Contract::Safe5V, ControlMessageType::Wait | ControlMessageType::Reject) => {
                        State::WaitForCapabilities
                    }
                    (Contract::Explicit, ControlMessageType::Reject | ControlMessageType::Wait) => State::Ready,
                    _ => unreachable!(),
                }
            }
            State::TransitionSink(accepted_request) => {
                self.protocol_layer
                    .receive_message_type(
                        &[MessageType::Control(ControlMessageType::PsRdy)],
                        TimerType::PSTransitionSpr,
                    )
                    .await?;

                self.contract = Contract::TransitionToExplicit;
                self.device_policy_manager.transition_power(accepted_request).await;
                State::Ready
            }
            State::Ready => {
                // Entry: Init. and run SinkRequestTimer(2) on receiving wait
                // Entry: Init. and run DiscoverIdentityTimer(4)
                // Entry: Init. and run SinkPPSPeriodicTimer(5)
                // Entry: Init. and run SinkEPRKeppAlivaTimer(6) in EPR mode
                // Entry: Send GetSinkCap message if sink supports fast role swap
                // Exit: If initiating an AMS, notify protocol layer??? Transition to
                // - EPRKeepAlive on SinkEPRKeepAliveTimer timeout
                self.contract = Contract::Explicit;

                let receive_fut = self.protocol_layer.receive_message();

                let dpm_event_fut = self
                    .device_policy_manager
                    .get_event(self.source_capabilities.as_ref().unwrap());

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
                                State::EvaluateCapabilities(capabilities)
                            }
                            MessageType::Control(ControlMessageType::GetSinkCap) => State::GiveSinkCap,
                            _ => State::SendNotSupported,
                        }
                    }
                    Either::Right((event, _)) => match event {
                        Event::RequestSourceCapabilities => State::EventRequestSourceCapabilities,
                        Event::RequestPower(power) => State::SelectCapability(power),
                        Event::None => State::Ready,
                    },
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

                // Return to `Ready` state instead, when
                // - in EPR mode, and SPR capabilities requested, or
                // - in SPR mode, and EPR capabilities requested.
                // In other words, in case of a switch.
                State::EvaluateCapabilities(self.wait_for_source_capabilities().await?)
            }
            State::_EPRKeepAlive => {
                // Entry: Send EPRKeepAlive Message
                // Entry: Init. and run SenderReponseTimer
                // Transition to
                // - Ready on EPRKeepAliveAck message
                // - HardReset on SenderResponseTimerTimeout

                State::Ready
            }
            State::EventRequestSourceCapabilities => {
                self.protocol_layer
                    .transmit_control_message(ControlMessageType::GetSourceCap)
                    .await?;
                State::WaitForCapabilities
            }
        };

        self.state = new_state;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::Sink;
    use crate::counters::{Counter, CounterType};
    use crate::dummy::{DummyDriver, DummySinkDevice, DummyTimer, DUMMY_CAPABILITIES};
    use crate::protocol_layer::message::header::{ControlMessageType, DataMessageType, Header, MessageType};
    use crate::protocol_layer::message::Message;
    use crate::sink::policy_engine::State;

    fn get_policy_engine() -> Sink<DummyDriver<30>, DummyTimer, DummySinkDevice> {
        Sink::new(DummyDriver::new(), DummySinkDevice {})
    }

    fn simulate_source_control_message(
        policy_engine: &mut Sink<DummyDriver<30>, DummyTimer, DummySinkDevice>,
        control_message_type: ControlMessageType,
        message_id: u8,
    ) {
        let header = *policy_engine.protocol_layer.header();
        let mut buf = [0u8; 30];

        Message::new(Header::new_control(
            header,
            Counter::new_from_value(CounterType::MessageId, message_id),
            control_message_type,
        ))
        .to_bytes(&mut buf);
        policy_engine.protocol_layer.driver().inject_received_data(&buf);
    }

    #[tokio::test]
    async fn test_negotiation() {
        // Instantiated in `Discovery` state
        let mut policy_engine = get_policy_engine();

        // Provide capabilities
        policy_engine
            .protocol_layer
            .driver()
            .inject_received_data(&DUMMY_CAPABILITIES);

        // `Discovery` -> `WaitForCapabilities`
        policy_engine.run_step().await.unwrap();

        // `WaitForCapabilities` -> `EvaluateCapabilities`
        policy_engine.run_step().await.unwrap();

        let good_crc = Message::from_bytes(&policy_engine.protocol_layer.driver().probe_transmitted_data());
        assert!(matches!(
            good_crc.header.message_type(),
            MessageType::Control(ControlMessageType::GoodCRC)
        ));

        // Simulate `GoodCrc` with ID 0.
        simulate_source_control_message(&mut policy_engine, ControlMessageType::GoodCRC, 0);

        // `EvaluateCapabilities` -> `SelectCapability`
        policy_engine.run_step().await.unwrap();

        // Simulate `Accept` message.
        simulate_source_control_message(&mut policy_engine, ControlMessageType::Accept, 1);

        // `SelectCapability` -> `TransitionSink`
        policy_engine.run_step().await.unwrap();

        let request_capabilities = Message::from_bytes(&policy_engine.protocol_layer.driver().probe_transmitted_data());
        assert!(matches!(
            request_capabilities.header.message_type(),
            MessageType::Data(DataMessageType::Request)
        ));

        // Simulate `PsRdy` message.
        simulate_source_control_message(&mut policy_engine, ControlMessageType::PsRdy, 2);

        // `TransitionSink` -> `Ready`
        policy_engine.run_step().await.unwrap();

        assert!(matches!(policy_engine.state, State::Ready));

        let good_crc = Message::from_bytes(&policy_engine.protocol_layer.driver().probe_transmitted_data());
        assert!(matches!(
            good_crc.header.message_type(),
            MessageType::Control(ControlMessageType::GoodCRC)
        ));
    }
}

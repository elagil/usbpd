//! Policy engine for the implementation of a sink.
use core::marker::PhantomData;

use embassy_futures::select::{Either3, select3};
use usbpd_traits::Driver;

use super::device_policy_manager::DevicePolicyManager;
use crate::counters::{Counter, Error as CounterError};
use crate::protocol_layer::message::header::{
    ControlMessageType, DataMessageType, ExtendedControlMessageType, Header, MessageType, SpecificationRevision,
};
use crate::protocol_layer::message::request::PowerSource;
use crate::protocol_layer::message::source_capabilities::SourceCapabilities;
use crate::protocol_layer::message::{Data, request};
use crate::protocol_layer::{ProtocolError, ProtocolLayer, RxError, TxError};
use crate::sink::device_policy_manager::Event;
use crate::timers::{Timer, TimerType};
use crate::{DataRole, PowerRole};

/// Sink capability
#[derive(Debug, Clone, Copy, PartialEq)]
enum Mode {
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
    // States of the policy engine as given by the specification.
    /// Default state at startup.
    Startup,
    Discovery,
    WaitForCapabilities,
    EvaluateCapabilities(SourceCapabilities),
    SelectCapability(request::PowerSource),
    TransitionSink(request::PowerSource),
    Ready(request::PowerSource),
    SendNotSupported(request::PowerSource),
    SendSoftReset,
    SoftReset,
    HardReset,
    TransitionToDefault,
    GiveSinkCap(request::PowerSource),
    GetSourceCap(Mode, request::PowerSource),

    // EPR states
    _EprSendEntry,
    _EprSendExit,
    _EprEntryWaitForResponse,
    _EprExitReceived,
    _EprKeepAlive(request::PowerSource),
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
    mode: Mode,
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
    /// Create a fresh protocol layer with initial state.
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
            mode: Mode::Spr,
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
            let new_state = match (&self.mode, &self.state, protocol_error) {
                // Handle when hard reset is signaled by the driver itself.
                (_, _, ProtocolError::RxError(RxError::HardReset) | ProtocolError::TxError(TxError::HardReset)) => {
                    Some(State::TransitionToDefault)
                }

                // Handle when soft reset is signaled by the driver itself.
                (_, _, ProtocolError::RxError(RxError::SoftReset)) => Some(State::SoftReset),

                // Unexpected messages indicate a protocol error and demand a soft reset.
                // See spec, [6.8.1]
                (_, _, ProtocolError::UnexpectedMessage) => Some(State::SendSoftReset),

                // FIXME: Unexpected message in power transition -> hard reset?
                // FIXME: Source cap. message not requested by get_source_caps
                // FIXME: EPR mode and EPR source cap. message with EPR PDO in positions 1..7
                // (Mode::Epr, _, _) => Some(State::HardReset),

                // Fall back to hard reset
                // - after soft reset accept failed to be sent, or
                // - after sending soft reset failed.
                (_, State::SoftReset | State::SendSoftReset, ProtocolError::TransmitRetriesExceeded(_)) => {
                    Some(State::HardReset)
                }

                // See spec, [8.3.3.3.3]
                (_, State::WaitForCapabilities, ProtocolError::RxError(RxError::ReceiveTimeout)) => {
                    Some(State::HardReset)
                }

                // See spec, [8.3.3.3.5]
                (_, State::SelectCapability(_), ProtocolError::RxError(RxError::ReceiveTimeout)) => {
                    Some(State::HardReset)
                }

                // See spec, [8.3.3.3.6]
                (_, State::TransitionSink(_), _) => Some(State::HardReset),

                (_, State::Ready(power_source), ProtocolError::RxError(RxError::UnsupportedMessage)) => {
                    Some(State::SendNotSupported(*power_source))
                }

                (_, _, ProtocolError::TransmitRetriesExceeded(_)) => Some(State::SendSoftReset),

                // Attempt to recover protocol errors with a soft reset.
                (_, _, error) => {
                    error!("Protocol error {:?} in sink state transition", error);
                    None
                }
            };

            if let Some(state) = new_state {
                self.state = state
            }

            Ok(())
        } else {
            error!("Unrecoverable result {:?} in sink state transition", result);
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

    async fn wait_for_source_capabilities(
        protocol_layer: &mut ProtocolLayer<DRIVER, TIMER>,
    ) -> Result<SourceCapabilities, Error> {
        let message = protocol_layer.wait_for_source_capabilities().await?;
        trace!("Source capabilities: {:?}", message);

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
            State::WaitForCapabilities => {
                State::EvaluateCapabilities(Self::wait_for_source_capabilities(&mut self.protocol_layer).await?)
            }
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
            State::SelectCapability(power_source) => {
                self.protocol_layer.request_power(*power_source).await?;

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
                    (_, ControlMessageType::Accept) => State::TransitionSink(*power_source),
                    (Contract::Safe5V, ControlMessageType::Wait | ControlMessageType::Reject) => {
                        // TODO: If a `Wait` message is received, the sink may request again, after a timeout of `tSinkRequest`.
                        State::WaitForCapabilities
                    }
                    (Contract::Explicit, ControlMessageType::Reject | ControlMessageType::Wait) => {
                        State::Ready(*power_source)
                    }
                    _ => unreachable!(),
                }
            }
            State::TransitionSink(power_source) => {
                self.protocol_layer
                    .receive_message_type(
                        &[MessageType::Control(ControlMessageType::PsRdy)],
                        TimerType::PSTransitionSpr,
                    )
                    .await?;

                self.contract = Contract::TransitionToExplicit;
                self.device_policy_manager.transition_power(power_source).await;
                State::Ready(*power_source)
            }
            State::Ready(power_source) => {
                // TODO: Entry: Init. and run SinkRequestTimer(2) on receiving `Wait`
                // TODO: Entry: Init. and run DiscoverIdentityTimer(4)
                // Entry: Init. and run SinkPPSPeriodicTimer(5) in SPR PPS mode
                // TODO: Entry: Init. and run SinkEPRKeepAliveTimer(6) in EPR mode
                // TODO: Entry: Send GetSinkCap message if sink supports fast role swap
                // TODO: Exit: If initiating an AMS, notify protocol layer??? Transition to
                // - EPRKeepAlive on SinkEPRKeepAliveTimer timeout
                self.contract = Contract::Explicit;

                let receive_fut = self.protocol_layer.receive_message();
                let event_fut = self
                    .device_policy_manager
                    .get_event(self.source_capabilities.as_ref().unwrap());
                let pps_periodic_fut = async {
                    match power_source {
                        PowerSource::Pps(_) => TimerType::get_timer::<TIMER>(TimerType::SinkPPSPeriodic).await,
                        _ => core::future::pending().await,
                    }
                };

                match select3(receive_fut, event_fut, pps_periodic_fut).await {
                    // A message was received.
                    Either3::First(message) => {
                        let message = message?;

                        match message.header.message_type() {
                            MessageType::Data(DataMessageType::SourceCapabilities) => {
                                let Some(Data::SourceCapabilities(capabilities)) = message.data else {
                                    unreachable!()
                                };
                                State::EvaluateCapabilities(capabilities)
                            }
                            MessageType::Control(ControlMessageType::GetSinkCap) => State::GiveSinkCap(*power_source),
                            _ => State::SendNotSupported(*power_source),
                        }
                    }
                    // Event from device policy manager.
                    Either3::Second(event) => match event {
                        Event::RequestSprSourceCapabilities => State::GetSourceCap(Mode::Spr, *power_source),
                        Event::RequestEprSourceCapabilities => State::GetSourceCap(Mode::Epr, *power_source),
                        Event::RequestPower(power_source) => State::SelectCapability(power_source),
                        Event::None => State::Ready(*power_source),
                    },
                    // PPS periodic timeout -> select capability again as keep-alive.
                    Either3::Third(_) => State::SelectCapability(*power_source),
                }
            }
            State::SendNotSupported(power_source) => {
                self.protocol_layer
                    .transmit_control_message(ControlMessageType::NotSupported)
                    .await?;

                State::Ready(*power_source)
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
            State::GiveSinkCap(power_source) => {
                // FIXME: Send sink capabilities, as provided by device policy manager.
                // Sending NotSupported is not to spec.
                // See spec, [6.4.1.6]
                self.protocol_layer
                    .transmit_control_message(ControlMessageType::NotSupported)
                    .await?;

                State::Ready(*power_source)
            }
            State::GetSourceCap(requested_mode, power_source) => {
                // Commonly used for switching between EPR and SPR mode, depending on requested mode.
                match requested_mode {
                    Mode::Spr => {
                        self.protocol_layer
                            .transmit_control_message(ControlMessageType::GetSourceCap)
                            .await?;
                    }
                    Mode::Epr => {
                        self.protocol_layer
                            .transmit_extended_control_message(ExtendedControlMessageType::EprGetSourceCap)
                            .await?;
                    }
                };

                let caps = Self::wait_for_source_capabilities(&mut self.protocol_layer).await?;
                self.device_policy_manager.inform(&caps).await;

                State::Ready(*power_source)
            }
            State::_EprSendEntry => unimplemented!(),
            State::_EprEntryWaitForResponse => unimplemented!(),
            State::_EprSendExit => unimplemented!(),
            State::_EprExitReceived => unimplemented!(),
            State::_EprKeepAlive(_power_source) => {
                // Entry: Send EPRKeepAlive Message
                // Entry: Init. and run SenderReponseTimer
                // Transition to
                // - Ready on EPRKeepAliveAck message
                // - HardReset on SenderResponseTimerTimeout
                unimplemented!();
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
    use crate::dummy::{DUMMY_CAPABILITIES, DummyDriver, DummySinkDevice, DummyTimer};
    use crate::protocol_layer::message::Message;
    use crate::protocol_layer::message::header::{ControlMessageType, DataMessageType, Header, MessageType};
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

        let good_crc = Message::from_bytes(&policy_engine.protocol_layer.driver().probe_transmitted_data()).unwrap();
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

        let request_capabilities =
            Message::from_bytes(&policy_engine.protocol_layer.driver().probe_transmitted_data()).unwrap();
        assert!(matches!(
            request_capabilities.header.message_type(),
            MessageType::Data(DataMessageType::Request)
        ));

        // Simulate `PsRdy` message.
        simulate_source_control_message(&mut policy_engine, ControlMessageType::PsRdy, 2);

        // `TransitionSink` -> `Ready`
        policy_engine.run_step().await.unwrap();

        assert!(matches!(policy_engine.state, State::Ready(_)));

        let good_crc = Message::from_bytes(&policy_engine.protocol_layer.driver().probe_transmitted_data()).unwrap();
        assert!(matches!(
            good_crc.header.message_type(),
            MessageType::Control(ControlMessageType::GoodCRC)
        ));
    }
}

//! Policy engine for the implementation of a sink.
use core::marker::PhantomData;

use embassy_futures::select::{Either, Either3, select, select3};
use uom::si::power::watt;
use usbpd_traits::Driver;

use super::device_policy_manager::DevicePolicyManager;
use crate::counters::Counter;
use crate::protocol_layer::message::data::epr_mode::Action;
use crate::protocol_layer::message::data::request::PowerSource;
use crate::protocol_layer::message::data::source_capabilities::SourceCapabilities;
use crate::protocol_layer::message::data::{Data, request};
use crate::protocol_layer::message::extended::extended_control::ExtendedControlMessageType;
use crate::protocol_layer::message::header::{
    ControlMessageType, DataMessageType, ExtendedMessageType, Header, MessageType, SpecificationRevision,
};
use crate::protocol_layer::message::{Payload, extended};
use crate::protocol_layer::{ProtocolError, ProtocolLayer, RxError, TxError};
use crate::sink::device_policy_manager::Event;
use crate::timers::{Timer, TimerType};
use crate::{DataRole, PowerRole, units};

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
    /// Ready state. The bool indicates if we entered due to receiving a Wait message,
    /// which requires running SinkRequestTimer before allowing re-request.
    Ready(request::PowerSource, bool),
    SendNotSupported(request::PowerSource),
    SendSoftReset,
    SoftReset,
    HardReset,
    TransitionToDefault,
    /// Give sink capabilities. The Mode indicates whether to send Sink_Capabilities (Spr)
    /// or EPR_Sink_Capabilities (Epr) per spec 8.3.3.3.10.
    GiveSinkCap(Mode, request::PowerSource),
    GetSourceCap(Mode, request::PowerSource),

    // EPR states
    EprModeEntry(request::PowerSource, units::Power),
    EprEntryWaitForResponse(request::PowerSource),
    EprWaitForCapabilities(request::PowerSource),
    EprSendExit,
    EprExitReceived(request::PowerSource),
    EprKeepAlive(request::PowerSource),
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
    /// Tracks whether a Get_Source_Cap request is pending.
    /// Per USB PD Spec R3.2 Section 8.3.3.3.8, in EPR mode, receiving a
    /// Source_Capabilities message that was not requested via Get_Source_Cap
    /// shall trigger a Hard Reset.
    get_source_cap_pending: bool,

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
            get_source_cap_pending: false,
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

                // Per spec 6.3.13: If the Soft_Reset Message fails, a Hard Reset shall be initiated.
                // This handles the case where we're trying to send/receive a soft reset and it fails.
                (_, State::SoftReset | State::SendSoftReset, ProtocolError::TransmitRetriesExceeded(_)) => {
                    Some(State::HardReset)
                }

                // Per spec 8.3.3.3.3: SinkWaitCapTimer timeout triggers Hard Reset.
                (_, State::WaitForCapabilities, ProtocolError::RxError(RxError::ReceiveTimeout)) => {
                    Some(State::HardReset)
                }

                // Per spec 8.3.3.3.5: SenderResponseTimer timeout triggers Hard Reset.
                (_, State::SelectCapability(_), ProtocolError::RxError(RxError::ReceiveTimeout)) => {
                    Some(State::HardReset)
                }

                // Per USB PD Spec R3.2 Section 8.3.3.3.6 and Table 6.72:
                // Any Protocol Error during power transition (PE_SNK_Transition_Sink state)
                // shall trigger a Hard Reset, not a Soft Reset.
                (_, State::TransitionSink(_), _) => Some(State::HardReset),

                // Unexpected messages indicate a protocol error and demand a soft reset.
                // Per spec 6.8.1 Table 6.72 (for non-power-transitioning states).
                // Note: This must come AFTER TransitionSink check above.
                (_, _, ProtocolError::UnexpectedMessage) => Some(State::SendSoftReset),

                // Per spec Table 6.72: Unsupported messages in Ready state get Not_Supported response.
                (_, State::Ready(power_source, _), ProtocolError::RxError(RxError::UnsupportedMessage)) => {
                    Some(State::SendNotSupported(*power_source))
                }

                // Per spec 6.6.9.1: Transmission failure (no GoodCRC after retries) triggers Soft Reset.
                // Note: If we're in SoftReset/SendSoftReset state, this is caught above and escalates to Hard Reset.
                (_, _, ProtocolError::TransmitRetriesExceeded(_)) => Some(State::SendSoftReset),

                // Unhandled protocol errors - log and continue.
                // Note: Unrequested Source_Capabilities in EPR mode is handled in Ready state
                // by checking get_source_cap_pending flag (per spec 8.3.3.3.8).
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

    /// Wait for source capabilities message (either Source_Capabilities or EPR_Source_Capabilities).
    ///
    /// Per USB PD Spec R3.2 Section 8.3.3.3.3 (PE_SNK_Wait_for_Capabilities):
    /// - In SPR Mode: Source_Capabilities Message is received
    /// - In EPR Mode: EPR_Source_Capabilities Message is received
    ///
    /// EPR Mode persists through Soft Reset (unlike Hard Reset which exits EPR per spec 6.8.3.2).
    /// Per spec section 6.4.1.2.2, after a Soft Reset while in EPR Mode, the source sends
    /// EPR_Source_Capabilities. Therefore this function must handle both message types.
    async fn wait_for_source_capabilities(
        protocol_layer: &mut ProtocolLayer<DRIVER, TIMER>,
    ) -> Result<SourceCapabilities, Error> {
        let message = protocol_layer.wait_for_source_capabilities().await?;
        trace!("Source capabilities: {:?}", message);

        let capabilities = match message.payload {
            Some(Payload::Data(Data::SourceCapabilities(caps))) => caps,
            Some(Payload::Extended(extended::Extended::EprSourceCapabilities(pdos))) => SourceCapabilities(pdos),
            _ => unreachable!(),
        };

        Ok(capabilities)
    }

    async fn update_state(&mut self) -> Result<(), Error> {
        let new_state = match &self.state {
            State::Startup => {
                self.contract = Default::default();
                self.protocol_layer.reset();
                self.mode = Mode::Spr;

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
                        State::WaitForCapabilities
                    }
                    (Contract::Explicit, ControlMessageType::Reject) => State::Ready(*power_source, false),
                    (Contract::Explicit, ControlMessageType::Wait) => {
                        // Per spec 8.3.3.3.7: On entry to Ready as result of Wait,
                        // initialize and run SinkRequestTimer.
                        State::Ready(*power_source, true)
                    }
                    _ => unreachable!(),
                }
            }
            State::TransitionSink(power_source) => {
                self.protocol_layer
                    .receive_message_type(
                        &[MessageType::Control(ControlMessageType::PsRdy)],
                        match self.mode {
                            Mode::Epr => TimerType::PSTransitionEpr,
                            Mode::Spr => TimerType::PSTransitionSpr,
                        },
                    )
                    .await?;

                self.contract = Contract::TransitionToExplicit;
                self.device_policy_manager.transition_power(power_source).await;
                State::Ready(*power_source, false)
            }
            State::Ready(power_source, after_wait) => {
                // TODO: Entry: Init. and run DiscoverIdentityTimer(4)
                // TODO: Entry: Send GetSinkCap message if sink supports fast role swap
                // TODO: Exit: If initiating an AMS, notify protocol layer
                //
                // Timers implemented:
                // - SinkRequestTimer: Per spec 8.3.3.3.7, after receiving Wait, wait tSinkRequest
                //   before allowing re-request. On timeout, transition to SelectCapability.
                // - SinkPPSPeriodicTimer: triggers SelectCapability in SPR PPS mode
                // - SinkEPRKeepAliveTimer: triggers EprKeepAlive in EPR mode
                self.contract = Contract::Explicit;

                // Per spec 6.6.4.1: SinkRequestTimer ensures minimum tSinkRequest (100ms) delay
                // after receiving Wait before re-requesting. Timer is only active if we entered
                // Ready due to a Wait message.
                if *after_wait {
                    TimerType::get_timer::<TIMER>(TimerType::SinkRequest).await;
                    // Per spec 8.3.3.3.7: SinkRequestTimer timeout → SelectCapability
                    self.state = State::SelectCapability(*power_source);
                    return Ok(());
                }

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
                let epr_keep_alive_fut = async {
                    match self.mode {
                        Mode::Epr => TimerType::get_timer::<TIMER>(TimerType::SinkEPRKeepAlive).await,
                        Mode::Spr => core::future::pending().await,
                    }
                };
                let timers_fut = async { select(pps_periodic_fut, epr_keep_alive_fut).await };

                match select3(receive_fut, event_fut, timers_fut).await {
                    // A message was received.
                    Either3::First(message) => {
                        let message = message?;

                        match message.header.message_type() {
                            MessageType::Data(DataMessageType::SourceCapabilities) => {
                                // Per USB PD Spec R3.2 Section 8.3.3.3.8:
                                // In EPR Mode, if a Source_Capabilities Message is received that
                                // has not been requested using a Get_Source_Cap Message, trigger Hard Reset.
                                if self.mode == Mode::Epr && !self.get_source_cap_pending {
                                    State::HardReset
                                } else {
                                    let Some(Payload::Data(Data::SourceCapabilities(capabilities))) = message.payload
                                    else {
                                        unreachable!()
                                    };
                                    self.get_source_cap_pending = false;
                                    State::EvaluateCapabilities(capabilities)
                                }
                            }
                            MessageType::Extended(ExtendedMessageType::EprSourceCapabilities) => {
                                if let Some(Payload::Extended(extended::Extended::EprSourceCapabilities(pdos))) =
                                    message.payload
                                {
                                    self.get_source_cap_pending = false;
                                    let caps = SourceCapabilities(pdos);

                                    // Per spec 8.3.3.3.8: In EPR Mode, if EPR_Source_Capabilities
                                    // contains an EPR (A)PDO in positions 1-7 → Hard Reset
                                    if self.mode == Mode::Epr && caps.has_epr_pdo_in_spr_positions() {
                                        State::HardReset
                                    } else {
                                        State::EvaluateCapabilities(caps)
                                    }
                                } else {
                                    unreachable!()
                                }
                            }
                            MessageType::Data(DataMessageType::EprMode) => {
                                // Handle source exit notification.
                                State::EprExitReceived(*power_source)
                            }
                            // Per spec 8.3.3.3.7: Get_Sink_Cap → GiveSinkCap (send Sink_Capabilities)
                            MessageType::Control(ControlMessageType::GetSinkCap) => {
                                State::GiveSinkCap(Mode::Spr, *power_source)
                            }
                            // Per spec 8.3.3.3.7: EPR_Get_Sink_Cap → GiveSinkCap (send EPR_Sink_Capabilities)
                            MessageType::Extended(ExtendedMessageType::ExtendedControl) => {
                                if let Some(Payload::Extended(extended::Extended::ExtendedControl(ctrl))) =
                                    &message.payload
                                {
                                    if ctrl.message_type() == ExtendedControlMessageType::EprGetSinkCap {
                                        State::GiveSinkCap(Mode::Epr, *power_source)
                                    } else {
                                        State::SendNotSupported(*power_source)
                                    }
                                } else {
                                    State::SendNotSupported(*power_source)
                                }
                            }
                            _ => State::SendNotSupported(*power_source),
                        }
                    }
                    // Event from device policy manager.
                    Either3::Second(event) => match event {
                        Event::RequestSprSourceCapabilities => State::GetSourceCap(Mode::Spr, *power_source),
                        Event::RequestEprSourceCapabilities => State::GetSourceCap(Mode::Epr, *power_source),
                        Event::EnterEprMode(pdp) => State::EprModeEntry(*power_source, pdp),
                        Event::ExitEprMode => State::EprSendExit,
                        Event::RequestPower(power_source) => State::SelectCapability(power_source),
                        Event::None => State::Ready(*power_source, false),
                    },
                    // PPS periodic timeout -> select capability again as keep-alive.
                    Either3::Third(timeout_source) => match timeout_source {
                        Either::First(_) => State::SelectCapability(*power_source),
                        Either::Second(_) => State::EprKeepAlive(*power_source),
                    },
                }
            }
            State::SendNotSupported(power_source) => {
                self.protocol_layer
                    .transmit_control_message(ControlMessageType::NotSupported)
                    .await?;

                State::Ready(*power_source, false)
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
                // Per USB PD Spec R3.2 Section 8.3.3.3.8 (PE_SNK_Hard_Reset):
                // Entry conditions:
                // - PSTransitionTimer timeout (when HardResetCounter <= nHardResetCount)
                // - Hard reset request from Device Policy Manager
                // - EPR mode and EPR_Source_Capabilities message with EPR PDO in pos. 1..7
                // - Source_Capabilities message not requested by Get_Source_Cap
                // - SinkWaitCapTimer timeout (May transition)
                //
                // On entry: Request Hard Reset Signaling AND increment HardResetCounter

                // Check if we've exceeded the hard reset count before attempting
                if self.hard_reset_counter.increment().is_err() {
                    // Per spec: If SinkWaitCapTimer times out and HardResetCounter > nHardResetCount
                    // the Sink shall assume that the Source is non-responsive.
                    return Err(Error::PortPartnerUnresponsive);
                }

                // Transmit Hard Reset Signaling
                self.protocol_layer.hard_reset().await?;

                State::TransitionToDefault
            }
            State::TransitionToDefault => {
                // Per USB PD Spec R3.2 Section 8.3.3.3.9 (PE_SNK_Transition_to_default):
                // This state is entered when:
                // - Hard Reset Signaling is detected (received or transmitted)
                // - From PE_SNK_Hard_Reset after hard reset is complete
                //
                // On entry:
                // - Indicate to DPM that Sink shall transition to default
                // - Request reset of local hardware
                // - Request DPM that Port Data Role is set to UFP
                //
                // Transition to PE_SNK_Startup when:
                // - DPM indicates Sink has reached default level

                // Notify DPM about hard reset (DPM should transition to default power level)
                self.device_policy_manager.hard_reset().await;

                // Reset protocol layer (per spec 6.8.3: "Protocol Layers shall be reset as for Soft Reset")
                self.protocol_layer.reset();

                // Reset EPR mode (per spec 6.8.3.2: "Hard Reset shall cause EPR Mode to be exited")
                self.mode = Mode::Spr;

                // Reset contract to default
                self.contract = Contract::Safe5V;

                // Clear cached source capabilities
                self.source_capabilities = None;

                State::Startup
            }
            State::GiveSinkCap(response_mode, power_source) => {
                // Per USB PD Spec R3.2 Section 8.3.3.3.10:
                // - Send Sink_Capabilities when Get_Sink_Cap was received
                // - Send EPR_Sink_Capabilities when EPR_Get_Sink_Cap was received
                let sink_caps = self.device_policy_manager.sink_capabilities();
                match response_mode {
                    Mode::Spr => {
                        self.protocol_layer.transmit_sink_capabilities(sink_caps).await?;
                    }
                    Mode::Epr => {
                        self.protocol_layer.transmit_epr_sink_capabilities(sink_caps).await?;
                    }
                }

                State::Ready(*power_source, false)
            }
            State::GetSourceCap(requested_mode, power_source) => {
                // Commonly used for switching between EPR and SPR mode, depending on requested mode.
                // Set flag before sending to track that we requested source capabilities.
                // Per USB PD Spec R3.2 Section 8.3.3.3.8, in EPR mode, receiving an unrequested
                // Source_Capabilities message triggers a Hard Reset.
                self.get_source_cap_pending = true;

                match requested_mode {
                    Mode::Spr => {
                        self.protocol_layer
                            .transmit_control_message(ControlMessageType::GetSourceCap)
                            .await?;
                    }
                    Mode::Epr => {
                        self.protocol_layer
                            .transmit_extended_control_message(
                                crate::protocol_layer::message::extended::extended_control::ExtendedControlMessageType::EprGetSourceCap,
                            )
                            .await?;
                    }
                };

                // Use protocol layer directly to get the full Message (not the helper that only returns SourceCapabilities)
                let message = self.protocol_layer.wait_for_source_capabilities().await?;
                self.get_source_cap_pending = false;

                // Per spec 8.3.3.3.12:
                // - In SPR mode + SPR caps requested + Source_Capabilities received → EvaluateCapabilities
                // - In EPR mode + EPR caps requested + EPR_Source_Capabilities received → EvaluateCapabilities
                // - Mode mismatch (e.g., EPR mode but SPR caps requested) → Ready
                let received_spr = matches!(
                    message.header.message_type(),
                    MessageType::Data(DataMessageType::SourceCapabilities)
                );
                let received_epr = matches!(
                    message.header.message_type(),
                    MessageType::Extended(ExtendedMessageType::EprSourceCapabilities)
                );

                let mode_matches = (*requested_mode == Mode::Spr && self.mode == Mode::Spr && received_spr)
                    || (*requested_mode == Mode::Epr && self.mode == Mode::Epr && received_epr);

                // Extract capabilities from the message
                let capabilities = match message.payload {
                    Some(Payload::Data(Data::SourceCapabilities(caps))) => caps,
                    Some(Payload::Extended(extended::Extended::EprSourceCapabilities(pdos))) => {
                        SourceCapabilities(pdos)
                    }
                    _ => unreachable!(),
                };

                self.device_policy_manager.inform(&capabilities).await;

                if mode_matches {
                    State::EvaluateCapabilities(capabilities)
                } else {
                    State::Ready(*power_source, false)
                }
            }
            State::EprModeEntry(power_source, operational_pdp) => {
                // Request entry into EPR mode.
                // Per spec 8.3.3.26.2.1 (PE_SNK_Send_EPR_Mode_Entry), sink sends EPR_Mode (Enter)
                // and starts SenderResponseTimer and SinkEPREnterTimer.
                //
                // Per spec 6.4.10, the Data field shall be set to the EPR Sink Operational PDP.
                //
                // Note: The spec says SinkEPREnterTimer (500ms) should run continuously across
                // both EprModeEntry and EprEntryWaitForResponse states until stopped or timeout.
                // Our implementation uses SenderResponseTimer (30ms) here and a fresh
                // SinkEPREnterTimer (500ms) in EprEntryWaitForResponse. This means the total
                // timeout could be ~530ms instead of 500ms in edge cases. However, this is
                // within the spec's allowed range (tEnterEPR max = 550ms per Table 6.71).
                let pdp_watts: u8 = operational_pdp.get::<watt>() as u8;
                self.protocol_layer.transmit_epr_mode(Action::Enter, pdp_watts).await?;

                // Wait for EnterAcknowledged with SenderResponseTimer (spec step 9-14)
                let message = self
                    .protocol_layer
                    .receive_message_type(
                        &[MessageType::Data(DataMessageType::EprMode)],
                        TimerType::SenderResponse,
                    )
                    .await?;

                let Some(Payload::Data(Data::EprMode(epr_mode))) = message.payload else {
                    unreachable!()
                };

                match epr_mode.action() {
                    Action::EnterAcknowledged => {
                        // Source acknowledged, now wait for EnterSucceeded
                        State::EprEntryWaitForResponse(*power_source)
                    }
                    Action::EnterSucceeded => {
                        // Source skipped EnterAcknowledged and went directly to EnterSucceeded
                        self.mode = Mode::Epr;
                        State::EprWaitForCapabilities(*power_source)
                    }
                    Action::Exit => State::EprExitReceived(*power_source),
                    // Per spec 8.3.3.26.2.1: EPR_Mode message not Enter Succeeded → Soft Reset
                    _ => State::SendSoftReset,
                }
            }
            State::EprEntryWaitForResponse(power_source) => {
                // Wait for EnterSucceeded after receiving EnterAcknowledged.
                // Per spec 8.3.3.26.2.2 (PE_SNK_EPR_Mode_Wait_For_Response), use SinkEPREnterTimer
                // for the overall timeout while source performs cable discovery.
                let message = self
                    .protocol_layer
                    .receive_message_type(&[MessageType::Data(DataMessageType::EprMode)], TimerType::SinkEPREnter)
                    .await?;

                let Some(Payload::Data(Data::EprMode(epr_mode))) = message.payload else {
                    unreachable!()
                };

                match epr_mode.action() {
                    Action::EnterSucceeded => {
                        // EPR mode entry succeeded. Per spec Table 8.39 step 21-29,
                        // source will automatically send EPR_Source_Capabilities after this.
                        self.mode = Mode::Epr;
                        State::EprWaitForCapabilities(*power_source)
                    }
                    Action::Exit => State::EprExitReceived(*power_source),
                    // Per spec 8.3.3.26.2.2: EPR_Mode message not Enter Succeeded → Soft Reset
                    _ => State::SendSoftReset,
                }
            }
            State::EprWaitForCapabilities(_power_source) => {
                // After successful EPR mode entry, source automatically sends EPR_Source_Capabilities.
                // This may be a chunked extended message that requires assembly.
                // Wait for the capabilities and evaluate them.
                let message = self.protocol_layer.wait_for_source_capabilities().await?;

                match message.payload {
                    Some(Payload::Data(Data::SourceCapabilities(capabilities))) => {
                        State::EvaluateCapabilities(capabilities)
                    }
                    Some(Payload::Extended(extended::Extended::EprSourceCapabilities(pdos))) => {
                        State::EvaluateCapabilities(SourceCapabilities(pdos))
                    }
                    _ => {
                        error!("Expected source capabilities after EPR mode entry");
                        State::HardReset
                    }
                }
            }
            State::EprSendExit => {
                // Inform partner we are exiting EPR.
                self.protocol_layer.transmit_epr_mode(Action::Exit, 0).await?;
                self.mode = Mode::Spr;
                State::WaitForCapabilities
            }
            State::EprExitReceived(power_source) => {
                // Per USB PD Spec R3.2 Section 8.3.3.26.4.2 (PE_SNK_EPR_Mode_Exit_Received):
                // - If in an Explicit Contract with an SPR (A)PDO → WaitForCapabilities
                // - If NOT in an Explicit Contract with an SPR (A)PDO → HardReset
                //
                // SPR PDOs are in object positions 1-7, EPR PDOs are in positions 8+.
                // In EPR mode, requests use EprRequest which contains the RDO with object position.
                self.mode = Mode::Spr;

                let is_epr_pdo_contract = match power_source {
                    PowerSource::EprRequest { rdo, .. } => {
                        // Extract object position from RDO (bits 28-31)
                        let object_position = request::RawDataObject(*rdo).object_position();
                        object_position >= 8
                    }
                    // Non-EprRequest variants are only used in SPR mode, so always SPR PDOs
                    _ => false,
                };

                if is_epr_pdo_contract {
                    State::HardReset
                } else {
                    State::WaitForCapabilities
                }
            }
            State::EprKeepAlive(power_source) => {
                // Per spec 8.3.3.3.11 (PE_SNK_EPR_Keep_Alive):
                // - Entry: Send EPR_KeepAlive message, start SenderResponseTimer
                // - On EPR_KeepAlive_Ack: transition to Ready (which restarts SinkEPRKeepAliveTimer)
                // - On timeout: transition to HardReset
                self.protocol_layer
                    .transmit_extended_control_message(
                        crate::protocol_layer::message::extended::extended_control::ExtendedControlMessageType::EprKeepAlive,
                    )
                    .await?;
                match self
                    .protocol_layer
                    .receive_message_type(
                        &[MessageType::Extended(ExtendedMessageType::ExtendedControl)],
                        TimerType::SenderResponse,
                    )
                    .await
                {
                    Ok(message) => {
                        if let Some(Payload::Extended(extended::Extended::ExtendedControl(control))) = message.payload {
                            if control.message_type()
                                == crate::protocol_layer::message::extended::extended_control::ExtendedControlMessageType::EprKeepAliveAck
                            {
                                self.mode = Mode::Epr;
                                State::Ready(*power_source, false)
                            } else {
                                State::SendNotSupported(*power_source)
                            }
                        } else {
                            State::SendNotSupported(*power_source)
                        }
                    }
                    Err(_) => State::HardReset,
                }
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
    use crate::protocol_layer::message::data::Data;
    use crate::protocol_layer::message::data::epr_mode::Action;
    use crate::protocol_layer::message::data::request::PowerSource;
    use crate::protocol_layer::message::data::source_capabilities::PowerDataObject;
    use crate::protocol_layer::message::header::{
        ControlMessageType, DataMessageType, ExtendedMessageType, Header, MessageType,
    };
    use crate::protocol_layer::message::{Message, Payload};
    use crate::sink::policy_engine::State;

    fn get_policy_engine() -> Sink<DummyDriver<30>, DummyTimer, DummySinkDevice> {
        Sink::new(DummyDriver::new(), DummySinkDevice {})
    }

    fn simulate_source_control_message<DPM: crate::sink::device_policy_manager::DevicePolicyManager>(
        policy_engine: &mut Sink<DummyDriver<30>, DummyTimer, DPM>,
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

    /// Get a header template for simulating source messages (Source/Dfp roles).
    /// This flips the roles from the sink's perspective to simulate messages from the source.
    fn get_source_header_template() -> Header {
        use crate::protocol_layer::message::header::SpecificationRevision;
        use crate::{DataRole, PowerRole};

        // Source messages have Source/Dfp roles (opposite of sink's Sink/Ufp)
        Header::new_template(DataRole::Dfp, PowerRole::Source, SpecificationRevision::R3_0)
    }

    /// Simulate an EPR Mode data message from the source with proper API.
    /// Returns the serialized bytes for assertion.
    fn simulate_source_epr_mode_message<DPM: crate::sink::device_policy_manager::DevicePolicyManager>(
        policy_engine: &mut Sink<DummyDriver<30>, DummyTimer, DPM>,
        action: Action,
        message_id: u8,
    ) -> heapless::Vec<u8, 30> {
        use crate::protocol_layer::message::data::epr_mode::EprModeDataObject;

        let source_header = get_source_header_template();
        let header = Header::new_data(
            source_header,
            Counter::new_from_value(CounterType::MessageId, message_id),
            DataMessageType::EprMode,
            1, // 1 data object (the EprModeDataObject)
        );

        let epr_mode = EprModeDataObject::default().with_action(action);
        let message = Message::new_with_data(header, Data::EprMode(epr_mode));

        let mut buf = [0u8; 30];
        let len = message.to_bytes(&mut buf);
        policy_engine.protocol_layer.driver().inject_received_data(&buf[..len]);

        let mut result = heapless::Vec::new();
        result.extend_from_slice(&buf[..len]).unwrap();
        result
    }

    /// Simulate an EprKeepAliveAck extended control message from the source.
    /// Returns the serialized bytes for assertion.
    fn simulate_epr_keep_alive_ack<DPM: crate::sink::device_policy_manager::DevicePolicyManager>(
        policy_engine: &mut Sink<DummyDriver<30>, DummyTimer, DPM>,
        message_id: u8,
    ) -> heapless::Vec<u8, 30> {
        use crate::protocol_layer::message::Payload;
        use crate::protocol_layer::message::extended::Extended;
        use crate::protocol_layer::message::extended::extended_control::{ExtendedControl, ExtendedControlMessageType};

        let source_header = get_source_header_template();
        // Create extended message header (num_objects=0 as used in transmit_extended_control_message)
        let header = Header::new_extended(
            source_header,
            Counter::new_from_value(CounterType::MessageId, message_id),
            ExtendedMessageType::ExtendedControl,
            0,
        );

        // Create the message with proper payload
        let mut message = Message::new(header);
        message.payload = Some(Payload::Extended(Extended::ExtendedControl(
            ExtendedControl::default().with_message_type(ExtendedControlMessageType::EprKeepAliveAck),
        )));

        // Serialize and inject
        let mut buf = [0u8; 30];
        let len = message.to_bytes(&mut buf);
        policy_engine.protocol_layer.driver().inject_received_data(&buf[..len]);

        let mut result = heapless::Vec::new();
        result.extend_from_slice(&buf[..len]).unwrap();
        result
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
        assert!(matches!(policy_engine.state, State::Ready(..)));

        let good_crc = Message::from_bytes(&policy_engine.protocol_layer.driver().probe_transmitted_data()).unwrap();
        assert!(matches!(
            good_crc.header.message_type(),
            MessageType::Control(ControlMessageType::GoodCRC)
        ));
    }

    #[tokio::test]
    async fn test_epr_negotiation() {
        use crate::dummy::{DUMMY_SPR_CAPS_EPR_CAPABLE, DummySinkEprDevice};

        // Create policy engine with EPR-capable DPM
        let mut policy_engine: Sink<DummyDriver<30>, DummyTimer, DummySinkEprDevice> =
            Sink::new(DummyDriver::new(), DummySinkEprDevice::new());

        // === Phase 1: Initial SPR Negotiation ===
        // Using same flow as test_negotiation
        eprintln!("Starting test");

        policy_engine
            .protocol_layer
            .driver()
            .inject_received_data(&DUMMY_SPR_CAPS_EPR_CAPABLE);

        // Discovery -> WaitForCapabilities
        eprintln!("run_step 1");
        policy_engine.run_step().await.unwrap();

        // WaitForCapabilities -> EvaluateCapabilities
        eprintln!("run_step 2");
        policy_engine.run_step().await.unwrap();

        eprintln!("Probing first GoodCRC");
        let good_crc = Message::from_bytes(&policy_engine.protocol_layer.driver().probe_transmitted_data()).unwrap();
        eprintln!("Got first GoodCRC");
        assert!(matches!(
            good_crc.header.message_type(),
            MessageType::Control(ControlMessageType::GoodCRC)
        ));

        // Simulate GoodCRC with ID 0
        simulate_source_control_message(&mut policy_engine, ControlMessageType::GoodCRC, 0);

        // EvaluateCapabilities -> SelectCapability
        policy_engine.run_step().await.unwrap();

        // Simulate Accept
        simulate_source_control_message(&mut policy_engine, ControlMessageType::Accept, 1);

        // SelectCapability -> TransitionSink
        policy_engine.run_step().await.unwrap();

        let request_capabilities =
            Message::from_bytes(&policy_engine.protocol_layer.driver().probe_transmitted_data()).unwrap();
        assert!(matches!(
            request_capabilities.header.message_type(),
            MessageType::Data(DataMessageType::Request)
        ));

        // Simulate PsRdy
        simulate_source_control_message(&mut policy_engine, ControlMessageType::PsRdy, 2);

        // TransitionSink -> Ready
        policy_engine.run_step().await.unwrap();
        eprintln!("State after last run_step: {:?}", policy_engine.state);
        assert!(matches!(policy_engine.state, State::Ready(..)));

        eprintln!(
            "Has transmitted data: {}",
            policy_engine.protocol_layer.driver().has_transmitted_data()
        );
        let good_crc = Message::from_bytes(&policy_engine.protocol_layer.driver().probe_transmitted_data()).unwrap();
        eprintln!("Got first GoodCRC: {:?}", good_crc.header.message_type());
        assert!(matches!(
            good_crc.header.message_type(),
            MessageType::Control(ControlMessageType::GoodCRC)
        ));

        // Probe any remaining messages from Phase 1
        while policy_engine.protocol_layer.driver().has_transmitted_data() {
            let msg = Message::from_bytes(&policy_engine.protocol_layer.driver().probe_transmitted_data()).unwrap();
            eprintln!("Draining leftover message: {:?}", msg.header.message_type());
        }

        eprintln!("\n=== Phase 1 Complete: SPR negotiation at 20V ===\n");

        // === Phase 2: EPR Mode Entry ===
        // Per spec 8.3.3.26.2, EPR mode entry flow:
        // 1. Sink sends EPR_Mode (Enter), starts SenderResponseTimer
        // 2. Source sends EnterAcknowledged
        // 3. Source performs cable discovery (we skip this in test)
        // 4. Source sends EnterSucceeded
        eprintln!("=== Phase 2: EPR Mode Entry ===");

        // Ready -> EprModeEntry (DPM triggers EnterEprMode event)
        policy_engine.run_step().await.unwrap();
        eprintln!("State after DPM event: {:?}", policy_engine.state);

        // Inject GoodCRC for EPR_Mode (Enter) that will be transmitted
        simulate_source_control_message(&mut policy_engine, ControlMessageType::GoodCRC, 1);

        // Inject EPR_Mode (EnterAcknowledged) using proper API
        // From capture line 78-81: RAW[6]: AA 19 00 00 00 02 (Source, msg_id=4)
        let epr_enter_ack_bytes = simulate_source_epr_mode_message(
            &mut policy_engine,
            Action::EnterAcknowledged,
            4, // message_id from capture
        );
        // Assert bytes match the real capture
        assert_eq!(
            &epr_enter_ack_bytes[..],
            &[0xAA, 0x19, 0x00, 0x00, 0x00, 0x02],
            "EPR EnterAcknowledged bytes should match capture"
        );

        // EprModeEntry: sends EPR_Mode (Enter), receives EnterAcknowledged -> EprEntryWaitForResponse
        match policy_engine.run_step().await {
            Ok(_) => eprintln!("EprModeEntry run_step succeeded"),
            Err(e) => eprintln!("EprModeEntry run_step failed: {:?}", e),
        }
        eprintln!("State after EprModeEntry: {:?}", policy_engine.state);

        // Probe EPR_Mode (Enter) message
        let epr_enter_bytes = policy_engine.protocol_layer.driver().probe_transmitted_data();
        let epr_enter = Message::from_bytes(&epr_enter_bytes).unwrap();
        eprintln!("Probed message type: {:?}", epr_enter.header.message_type());
        assert!(matches!(
            epr_enter.header.message_type(),
            MessageType::Data(DataMessageType::EprMode)
        ));
        if let Some(Payload::Data(Data::EprMode(mode))) = epr_enter.payload {
            assert_eq!(mode.action(), Action::Enter);
        } else {
            panic!("Expected EprMode Enter payload");
        }
        // Assert EPR_Mode Enter bytes match capture line 71-74: RAW[6]: 8A 14 00 00 00 01
        // Note: Our test starts from different state so message_id may differ
        eprintln!("EPR_Mode Enter bytes: {:02X?}", &epr_enter_bytes[..]);

        // Probe GoodCRC for EnterAcknowledged
        let good_crc = Message::from_bytes(&policy_engine.protocol_layer.driver().probe_transmitted_data()).unwrap();
        eprintln!("Probed GoodCRC for EnterAck: {:?}", good_crc.header.message_type());
        assert!(matches!(
            good_crc.header.message_type(),
            MessageType::Control(ControlMessageType::GoodCRC)
        ));

        // Inject EPR_Mode (EnterSucceeded) using proper API
        // From capture line 85-88: RAW[6]: AA 1B 00 00 00 03 (Source, msg_id=5)
        let epr_enter_succeeded_bytes = simulate_source_epr_mode_message(
            &mut policy_engine,
            Action::EnterSucceeded,
            5, // message_id from capture
        );
        // Assert bytes match the real capture
        assert_eq!(
            &epr_enter_succeeded_bytes[..],
            &[0xAA, 0x1B, 0x00, 0x00, 0x00, 0x03],
            "EPR EnterSucceeded bytes should match capture"
        );

        // EprEntryWaitForResponse receives EnterSucceeded -> EprWaitForCapabilities
        policy_engine.run_step().await.unwrap();
        eprintln!("State after EnterSucceeded: {:?}", policy_engine.state);

        // Probe GoodCRC for EnterSucceeded
        let good_crc = Message::from_bytes(&policy_engine.protocol_layer.driver().probe_transmitted_data()).unwrap();
        assert!(matches!(
            good_crc.header.message_type(),
            MessageType::Control(ControlMessageType::GoodCRC)
        ));

        eprintln!("=== Phase 2 Complete: EPR mode entry succeeded ===\n");

        // === Phase 3: Chunked EPR Source Capabilities ===
        // This follows the real-world capture flow per USB PD spec 6.12.2.1.2:
        // 1. Source sends chunk 0 -> Sink sends GoodCRC
        // 2. Sink sends Chunk Request (chunk=1) -> Source sends GoodCRC
        // 3. Source sends chunk 1 -> Sink sends GoodCRC
        use crate::dummy::{DUMMY_EPR_SOURCE_CAPS_CHUNK_0, DUMMY_EPR_SOURCE_CAPS_CHUNK_1};

        eprintln!("=== Phase 3: Chunked EPR Source Capabilities ===");

        // Source sends EPR_Source_Capabilities chunk 0
        policy_engine
            .protocol_layer
            .driver()
            .inject_received_data(&DUMMY_EPR_SOURCE_CAPS_CHUNK_0);

        // Inject GoodCRC for the Chunk Request that sink will send after receiving chunk 0
        // The chunk request message ID will be based on tx_message counter
        simulate_source_control_message(&mut policy_engine, ControlMessageType::GoodCRC, 2);

        // Source sends chunk 1 after receiving the chunk request
        policy_engine
            .protocol_layer
            .driver()
            .inject_received_data(&DUMMY_EPR_SOURCE_CAPS_CHUNK_1);

        // EprWaitForCapabilities -> Protocol layer:
        // - receives chunk 0, sends GoodCRC
        // - sends chunk request, waits for GoodCRC
        // - receives chunk 1, sends GoodCRC
        // - assembles message -> EvaluateCapabilities
        policy_engine.run_step().await.unwrap();

        // Probe GoodCRC for chunk 0
        let good_crc_0 = Message::from_bytes(&policy_engine.protocol_layer.driver().probe_transmitted_data()).unwrap();
        eprintln!("Chunk 0 GoodCRC: {:?}", good_crc_0.header.message_type());
        assert!(matches!(
            good_crc_0.header.message_type(),
            MessageType::Control(ControlMessageType::GoodCRC)
        ));

        // Probe the Chunk Request message (per spec 6.12.2.1.2.4)
        // Chunk requests are parsed as ChunkedExtendedMessage error, so we use parse_extended_chunk
        let chunk_req_data = policy_engine.protocol_layer.driver().probe_transmitted_data();
        let (chunk_req_header, chunk_req_ext_header, _chunk_data) =
            Message::parse_extended_chunk(&chunk_req_data).unwrap();
        eprintln!(
            "Chunk Request: type={:?}, chunk_number={}, request_chunk={}",
            chunk_req_header.message_type(),
            chunk_req_ext_header.chunk_number(),
            chunk_req_ext_header.request_chunk()
        );
        assert!(
            chunk_req_header.extended(),
            "Chunk request should be an extended message"
        );
        assert!(matches!(
            chunk_req_header.message_type(),
            MessageType::Extended(ExtendedMessageType::EprSourceCapabilities)
        ));
        assert!(chunk_req_ext_header.request_chunk(), "Should be a chunk request");
        assert_eq!(chunk_req_ext_header.chunk_number(), 1, "Should request chunk 1");

        // Probe GoodCRC for chunk 1
        let good_crc_1 = Message::from_bytes(&policy_engine.protocol_layer.driver().probe_transmitted_data()).unwrap();
        eprintln!("Chunk 1 GoodCRC: {:?}", good_crc_1.header.message_type());
        assert!(matches!(
            good_crc_1.header.message_type(),
            MessageType::Control(ControlMessageType::GoodCRC)
        ));

        eprintln!("=== Phase 3 Complete: EPR caps assembled (with chunk request per spec) ===\n");

        // === Phase 4: EPR Power Negotiation ===
        // Selects PDO#8 (28V @ 5A = 140W)
        eprintln!("=== Phase 4: EPR Power Negotiation ===");

        // EvaluateCapabilities -> DPM.request() selects EPR PDO#8 (28V) -> SelectCapability
        policy_engine.run_step().await.unwrap();
        eprintln!("State after evaluate: {:?}", policy_engine.state);

        // Inject GoodCRC for the EprRequest that will be transmitted
        // Note: TX message counter is now 3 (after chunk request in Phase 3 incremented it from 2 to 3)
        simulate_source_control_message(&mut policy_engine, ControlMessageType::GoodCRC, 3);

        // Also inject Accept message that SelectCapability will wait for after transmitting
        simulate_source_control_message(&mut policy_engine, ControlMessageType::Accept, 0);

        // SelectCapability -> sends EprRequest, waits for Accept -> TransitionSink
        policy_engine.run_step().await.unwrap();

        // Probe the EPR Request
        let epr_request = Message::from_bytes(&policy_engine.protocol_layer.driver().probe_transmitted_data()).unwrap();
        eprintln!(
            "EPR Request: {:?} (message_id={})",
            epr_request.header.message_type(),
            epr_request.header.message_id()
        );
        assert!(matches!(
            epr_request.header.message_type(),
            MessageType::Data(DataMessageType::EprRequest)
        ));

        // Verify EPR Request selects PDO#8 (28V)
        if let Some(Payload::Data(Data::Request(PowerSource::EprRequest { rdo, pdo }))) = &epr_request.payload {
            use crate::protocol_layer::message::data::request::RawDataObject;
            let object_pos = RawDataObject(*rdo).object_position();
            eprintln!("EPR Request: PDO#{} (RDO=0x{:08X})", object_pos, rdo);
            assert_eq!(object_pos, 8, "Should request PDO#8 (28V) to match real capture");

            // Verify it's the 28V PDO
            if let PowerDataObject::FixedSupply(fixed) = pdo {
                assert_eq!(fixed.raw_voltage(), 560, "28V = 560 * 50mV");
                assert_eq!(fixed.raw_max_current(), 500, "5A = 500 * 10mA");
            }
        } else {
            panic!("Expected EprRequest payload");
        }

        // Drain any leftover messages
        eprintln!(
            "Has transmitted data before drain: {}",
            policy_engine.protocol_layer.driver().has_transmitted_data()
        );
        while policy_engine.protocol_layer.driver().has_transmitted_data() {
            let msg = Message::from_bytes(&policy_engine.protocol_layer.driver().probe_transmitted_data()).unwrap();
            eprintln!("Draining: {:?}", msg.header.message_type());
        }
        eprintln!(
            "Has transmitted data after drain: {}",
            policy_engine.protocol_layer.driver().has_transmitted_data()
        );

        // Inject PsRdy that TransitionSink will wait for
        simulate_source_control_message(&mut policy_engine, ControlMessageType::PsRdy, 1);

        // TransitionSink waits for PsRdy -> Ready
        policy_engine.run_step().await.unwrap();

        // Probe any GoodCRCs we transmitted (for Accept and PsRdy messages we received)
        while policy_engine.protocol_layer.driver().has_transmitted_data() {
            let msg = Message::from_bytes(&policy_engine.protocol_layer.driver().probe_transmitted_data()).unwrap();
            eprintln!("Phase 4 transmitted: {:?}", msg.header.message_type());
            assert!(matches!(
                msg.header.message_type(),
                MessageType::Control(ControlMessageType::GoodCRC)
            ));
        }

        // Verify we're in Ready state with EPR power
        assert!(matches!(policy_engine.state, State::Ready(..)));
        eprintln!("Final state: {:?}", policy_engine.state);

        eprintln!("=== Phase 4 Complete: EPR power negotiation at 28V/5A (140W) ===\n");

        // === Phase 5: EPR Keep-Alive ===
        // Per USB PD spec 8.3.3.3.11, sink must send EprKeepAlive periodically in EPR mode.
        // Real capture shows multiple keep-alive exchanges after EPR contract (lines 145-214).
        // We manually transition to EprKeepAlive state to test this flow, simulating multiple
        // keep-alive cycles to verify the sink continues sending them.
        eprintln!("=== Phase 5: EPR Keep-Alive (multiple cycles) ===");

        // Test multiple keep-alive cycles to verify the sink keeps sending them
        // Real capture shows 7 keep-alive exchanges - we'll test 3 to verify the pattern
        // From capture (lines 152-183), EprKeepAliveAck messages have Source/Dfp roles.
        let mut sink_tx_counter = 4u8; // Sink TX counter after EPR Request
        let mut source_tx_counter = 2u8; // Source TX counter (increments with each message source sends)

        // Expected EprKeepAliveAck bytes from capture (for first 3 cycles):
        // Cycle 1 (msg_id=2): B0 95 02 80 04 00
        // Cycle 2 (msg_id=3): B0 97 02 80 04 00
        // Cycle 3 (msg_id=4): B0 99 02 80 04 00
        // Note: Header changes based on msg_id, but extended header (02 80) and payload (04 00) stay same

        for cycle in 1..=3 {
            eprintln!("--- Keep-Alive cycle {} ---", cycle);

            // Manually set state to EprKeepAlive (normally triggered by SinkEPRKeepAliveTimer in Ready state)
            if let State::Ready(power_source, _) = policy_engine.state.clone() {
                policy_engine.state = State::EprKeepAlive(power_source);
            } else {
                panic!("Expected Ready state before keep-alive cycle {}", cycle);
            }

            // Inject GoodCRC for the EprKeepAlive message that will be transmitted
            simulate_source_control_message(&mut policy_engine, ControlMessageType::GoodCRC, sink_tx_counter);
            sink_tx_counter = sink_tx_counter.wrapping_add(1);

            // Inject EprKeepAliveAck response from source with correct message ID
            let keep_alive_ack_bytes = simulate_epr_keep_alive_ack(&mut policy_engine, source_tx_counter);
            eprintln!("  EprKeepAliveAck bytes: {:02X?}", &keep_alive_ack_bytes[..]);
            // Verify payload matches capture pattern
            // The extended header data_size=2 (low byte 0x02), and the chunked bit may or may not be set
            // (spec allows both). Real capture shows chunked=true (0x80), but our impl uses chunked=false (0x00)
            // Payload: 04 00 (ExtendedControl with EprKeepAliveAck type)
            assert_eq!(keep_alive_ack_bytes[2] & 0x1F, 0x02, "data_size should be 2");
            assert_eq!(
                &keep_alive_ack_bytes[4..],
                &[0x04, 0x00],
                "EprKeepAliveAck payload should match capture"
            );
            source_tx_counter = source_tx_counter.wrapping_add(1);

            // EprKeepAlive sends keep-alive, receives ack -> Ready
            policy_engine.run_step().await.unwrap();

            // Probe the EprKeepAlive message
            let keep_alive_bytes = policy_engine.protocol_layer.driver().probe_transmitted_data();
            let keep_alive = Message::from_bytes(&keep_alive_bytes).unwrap();
            eprintln!("  EprKeepAlive sent: {:?}", keep_alive.header.message_type());
            assert!(matches!(
                keep_alive.header.message_type(),
                MessageType::Extended(ExtendedMessageType::ExtendedControl)
            ));

            // Verify it's actually an EprKeepAlive message
            if let Some(Payload::Extended(crate::protocol_layer::message::extended::Extended::ExtendedControl(ctrl))) =
                &keep_alive.payload
            {
                assert_eq!(
                    ctrl.message_type(),
                    crate::protocol_layer::message::extended::extended_control::ExtendedControlMessageType::EprKeepAlive,
                    "Expected EprKeepAlive message type"
                );
            } else {
                panic!("Expected ExtendedControl payload with EprKeepAlive");
            }
            // Verify EprKeepAlive payload matches capture pattern
            // From capture (e.g. line 145-148): 90 9A 02 80 03 00
            // Extended header data_size=2, Payload: 03 00 (EprKeepAlive type)
            // Note: chunked bit may differ between our impl (0x00) and capture (0x80)
            assert_eq!(keep_alive_bytes[2] & 0x1F, 0x02, "data_size should be 2");
            assert_eq!(
                &keep_alive_bytes[4..],
                &[0x03, 0x00],
                "EprKeepAlive payload should match capture"
            );

            // Probe GoodCRC for EprKeepAliveAck
            let good_crc =
                Message::from_bytes(&policy_engine.protocol_layer.driver().probe_transmitted_data()).unwrap();
            assert!(matches!(
                good_crc.header.message_type(),
                MessageType::Control(ControlMessageType::GoodCRC)
            ));

            // Verify we're back in Ready state (ready for next keep-alive cycle)
            assert!(matches!(policy_engine.state, State::Ready(..)));
            eprintln!("  Returned to Ready state");
        }

        eprintln!("=== Phase 5 Complete: {} EPR keep-alive cycles succeeded ===\n", 3);
        eprintln!("=== Full EPR negotiation test PASSED ===");
    }
}

//! Policy engine for the implementation of a sink.
use core::marker::PhantomData;

use embassy_futures::select::{Either3, select3};
use uom::si::power::watt;
use usbpd_traits::Driver;

use super::device_policy_manager::DevicePolicyManager;
use crate::counters::Counter;
use crate::protocol_layer::message::data::epr_mode::{self, Action};
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

#[cfg(test)]
mod tests;

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
        let header = Header::new_template(DataRole::Ufp, PowerRole::Sink, SpecificationRevision::R3_X);
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
                // Per spec 8.3.3.3.7: SinkRequestTimer runs concurrently when re-entering
                // Ready after a Wait response. On timeout, transition to SelectCapability.
                // Per spec 6.6.4.1: Ensures minimum tSinkRequest (100ms) delay before re-request.
                let sink_request_fut = async {
                    if *after_wait {
                        TimerType::get_timer::<TIMER>(TimerType::SinkRequest).await
                    } else {
                        core::future::pending().await
                    }
                };
                let timers_fut = async { select3(pps_periodic_fut, epr_keep_alive_fut, sink_request_fut).await };

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
                    // Timer timeout handling
                    Either3::Third(timeout_source) => match timeout_source {
                        // PPS periodic timeout -> select capability again as keep-alive.
                        Either3::First(_) => State::SelectCapability(*power_source),
                        // EPR keep-alive timeout
                        Either3::Second(_) => State::EprKeepAlive(*power_source),
                        // SinkRequest timeout -> re-request power after Wait response
                        Either3::Third(_) => State::SelectCapability(*power_source),
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
                // - SinkWaitCapTimer timeout (when HardResetCounter <= nHardResetCount)
                //
                // On entry: Request Hard Reset Signaling AND increment HardResetCounter

                // Increment counter first - returns Err when counter > nHardResetCount.
                // Per spec 8.3.3.3.8: If HardResetCounter > nHardResetCount (> 2),
                // the Sink shall assume that the Source is non-responsive.
                // With counter max_value = 3, we allow 3 hard reset attempts (counter 1, 2, 3)
                // before wrap returns Err.
                if self.hard_reset_counter.increment().is_err() {
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
                // Per USB PD Spec R3.2 Section 8.3.3.3.12 (PE_SNK_Get_Source_Cap):
                // - Send Get_Source_Cap (SPR) or EPR_Get_Source_Cap (EPR)
                // - Start SenderResponseTimer
                // - On timeout or mode mismatch → Ready
                // - On matching capabilities received → EvaluateCapabilities
                //
                // Set flag before sending to track that we requested source capabilities.
                // Per spec 8.3.3.3.8, in EPR mode, receiving an unrequested
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

                // Per spec 8.3.3.3.12: Use SenderResponseTimer (not SinkWaitCap)
                let result = self
                    .protocol_layer
                    .receive_message_type(
                        &[
                            MessageType::Data(DataMessageType::SourceCapabilities),
                            MessageType::Extended(ExtendedMessageType::EprSourceCapabilities),
                        ],
                        TimerType::SenderResponse,
                    )
                    .await;

                self.get_source_cap_pending = false;

                // Per spec 8.3.3.3.12: On timeout, inform DPM and transition to Ready
                let message = match result {
                    Ok(msg) => msg,
                    Err(ProtocolError::RxError(RxError::ReceiveTimeout)) => {
                        // Inform DPM of timeout (no capabilities received)
                        warn!("Get_Source_Cap timeout, returning to Ready");
                        self.state = State::Ready(*power_source, false);
                        return Ok(());
                    }
                    Err(e) => return Err(e.into()),
                };

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
                    Action::EnterFailed => {
                        // Per spec 8.3.3.26.2.1: EnterFailed → Soft Reset
                        // Notify DPM of the failure reason before soft reset
                        let reason = epr_mode::DataEnterFailed::from(epr_mode.data());
                        self.device_policy_manager.epr_mode_entry_failed(reason).await;
                        State::SendSoftReset
                    }
                    // Per spec 8.3.3.26.2.1: any other EPR_Mode message → Soft Reset
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
                    Action::EnterFailed => {
                        // Per spec 8.3.3.26.2.2: EnterFailed → Soft Reset
                        // Notify DPM of the failure reason before soft reset
                        let reason = epr_mode::DataEnterFailed::from(epr_mode.data());
                        self.device_policy_manager.epr_mode_entry_failed(reason).await;
                        State::SendSoftReset
                    }
                    // Per spec 8.3.3.26.2.2: any other EPR_Mode message → Soft Reset
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
                    PowerSource::EprRequest(epr) => {
                        // Extract object position from RDO (bits 28-31)
                        epr.object_position() >= 8
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

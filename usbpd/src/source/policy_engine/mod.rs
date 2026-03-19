//! Policy engine for the implementation of a sink.
use core::marker::PhantomData;

use embassy_futures::select::{Either, Either3, select, select3};
use usbpd_traits::Driver;

use super::device_policy_manager::{CapabilityResponse, Event, Info, SourceDpm, SwapType};
use crate::counters::Counter;
use crate::protocol_layer::message::data::request::PowerSource;
use crate::protocol_layer::message::data::sink_capabilities::SinkCapabilities;
use crate::protocol_layer::message::data::source_capabilities::{Kind, SourceCapabilities};
use crate::protocol_layer::message::data::{Data, PdoKind, epr_mode, request};
use crate::protocol_layer::message::extended::Extended;
use crate::protocol_layer::message::extended::extended_control::ExtendedControlMessageType;
use crate::protocol_layer::message::header::{
    ControlMessageType, DataMessageType, ExtendedMessageType, Header, MessageType, SpecificationRevision,
};
use crate::protocol_layer::message::{Message, Payload};
use crate::protocol_layer::{ProtocolError, RxError, SourceProtocolLayer, TxError};
use crate::timers::{Timer, TimerType};
use crate::{DataRole, PowerRole};

#[cfg(test)]
mod tests;

/// Source capability
#[derive(Debug, Clone, Copy, Default, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
enum Mode {
    /// The classic mode of PD operation where explicit contracts are negotiaged using SPR (A)PDOs.
    #[default]
    Spr,
    /// A Power Delivery mode of operation where maximum allowable voltage is 48V.
    Epr,
}

#[derive(Debug, Clone, Copy, Default)]
enum Contract {
    #[default]
    Safe5V,
    Implicit, // Only present after fast role swap. Limited to max. type C current.
    TransitionToExplicit,
    Explicit(PowerSource),

    // FIXME: Source EPR support may use this enum
    #[allow(unused)]
    Invalid,
}

/// Source states.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
enum State {
    // States of the policy engine as given by specification.
    // 8.3.3.2 Policy Engine Source Port State Diagram
    Startup { role_swap: bool },
    Discovery,
    SendCapabilities,
    NegotiateCapability(PowerSource),
    TransitionSupply(PowerSource),
    Ready,
    Disabled,
    CapabilityResponse(CapabilityResponse),
    HardReset,
    HardResetReceived,
    TransitionToDefault,
    GetSinkCap,
    WaitNewCapabilities,
    EprKeepAlive,
    GiveSourceCap,
    // 8.3.3.4 Source Port Soft Reset
    SendSoftReset,
    SoftReset,
    // 8.3.3.6 Not Supported Message State
    SendNotSupported,
    NotSupportedReceived,
    // 8.3.3.19 Dual-Role Port (DRP) States
    DrpSwap(SwapState),
    DrpGetSourceCap(Mode),
    DrpGiveSinkCap(Mode),
    // 8.3.3.20 Vconn Swap
    VconnSwap { source: VcsSwapSource, state: VcsState },
    // 8.3.3.26 EPR States
    EprMode(EprState),
    // Custom state to signal exit out of source to sink from a power swap
    PrSwapToSinkStartup,
    ErrorRecovery,
}

#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
/// Dual Role Port Swap States
enum SwapState {
    Data(DataRoleSwap),
    Power(PowerRoleSwap),
    FastPower(FastPowerRoleSwap),
}

#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
/// State during a Data Role Swap execution (for both directions)
enum DataRoleSwap {
    Evaluate,
    Accept,
    Change,
    Send,
    Reject,
}

#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
/// State during a Power Role Swap execution
enum PowerRoleSwap {
    Evaluate,
    Accept,
    TransitionToOff,
    AssertRd,
    WaitSourceOn,
    Send,
    Reject,
}

#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
/// State during a Fast Power Role Swap execution
enum FastPowerRoleSwap {
    Evaluate,
    Accept,
    TransitionToOff,
    AssertRd,
    WaitSourceOn,
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
enum VcsSwapSource {
    Message,
    Epr,
}

#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
enum VcsState {
    SendSwap,
    EvaluateSwap,
    AcceptSwap,
    RejectSwap,
    WaitForVconn,
    TurnOffVconn,
    TurnOnVconn,
    SendPsRdy,
    // FIXME: For now, forcing a different state traversal, resulting in this being unused
    #[allow(unused)]
    VcsForceVconn,
}

#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
enum EprState {
    Entry,
    EntryAck,
    DiscoverCable,
    EvaluateCable,
    EntrySucceeded,
    EntryFailed(u8),
    SendExit,
    ExitReceived,
}

/// Implementation of the source policy engine.
/// See spec, [8.3.3.2]
#[derive(Debug)]
pub struct Source<DRIVER: Driver, TIMER: Timer, DPM: SourceDpm> {
    device_policy_manager: DPM,
    protocol_layer: SourceProtocolLayer<DRIVER, TIMER>,
    hard_reset_counter: Counter,
    caps_counter: Counter,
    state: State,

    dual_role: bool,
    vconn_source: bool,
    mode: Mode,
    contract: Contract,

    _timer: PhantomData<TIMER>,
}

/// Errors that can occur in the source policy engine state machine.
#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Error {
    /// The port partner is unresponsive.
    PortPartnerUnresponsive,
    /// Entered ErrorRecovery mode. This requests a disconnect.
    ReconnectionRequired,
    /// Easiest way to signal to device to swap to sink
    SwapToSink,
    /// A protocol error has occured.
    Protocol(ProtocolError),
}

impl From<ProtocolError> for Error {
    fn from(protocol_error: ProtocolError) -> Self {
        Error::Protocol(protocol_error)
    }
}

impl<DRIVER: Driver, TIMER: Timer, DPM: SourceDpm> Source<DRIVER, TIMER, DPM> {
    fn new_protocol_layer(driver: DRIVER) -> SourceProtocolLayer<DRIVER, TIMER> {
        let header = Header::new_template(DataRole::Dfp, PowerRole::Source, SpecificationRevision::R3_X);
        SourceProtocolLayer::new(driver, header)
    }

    /// Create a new source policy engine with a given `driver` and set of `source_capabilities`.
    pub fn new(driver: DRIVER, device_policy_manager: DPM, role_swap: bool) -> Self {
        Self {
            device_policy_manager,
            protocol_layer: Self::new_protocol_layer(driver),
            hard_reset_counter: Counter::new(crate::counters::CounterType::HardReset),
            caps_counter: Counter::new(crate::counters::CounterType::Caps),

            state: State::Startup { role_swap },
            contract: match role_swap {
                true => Contract::Implicit,
                false => Contract::default(),
            },
            mode: Mode::default(),
            dual_role: false,
            vconn_source: true,

            _timer: PhantomData,
        }
    }

    /// Create a new source policy engine with dual role capabilities,
    /// with a given `driver`, and set of `source_capabilities`, and set of `sink_capabilities`
    /// for the port
    pub fn new_dual_role(driver: DRIVER, device_policy_manager: DPM, role_swapped: bool) -> Self {
        Self {
            device_policy_manager,
            protocol_layer: Self::new_protocol_layer(driver),
            hard_reset_counter: Counter::new(crate::counters::CounterType::HardReset),
            caps_counter: Counter::new(crate::counters::CounterType::Caps),

            state: match role_swapped {
                true => State::SendCapabilities,
                false => State::Startup { role_swap: false },
            },
            contract: match role_swapped {
                true => Contract::Implicit,
                false => Contract::default(),
            },
            mode: Mode::default(),
            dual_role: true,
            vconn_source: true,

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
                (_, _, ProtocolError::RxError(RxError::HardReset)) => Some(State::HardResetReceived),

                // Handle when hard reset is signaled by the driver itself.
                (_, _, ProtocolError::TxError(TxError::HardReset)) => Some(State::HardReset),

                // Handle when soft reset is signaled by the driver itself.
                (_, _, ProtocolError::RxError(RxError::SoftReset)) => Some(State::SoftReset),

                // Per spec 6.3.13: If the Soft_Reset Message fails, a Hard Reset shall be initiated.
                // This handles the case where we're trying to send/receive a soft reset and it fails.
                (_, State::SoftReset | State::SendSoftReset, ProtocolError::TransmitRetriesExceeded(_)) => {
                    Some(State::HardReset)
                }

                // Per spec 8.3.3.2.3: No GoodCRC (NoResponseTimer times out) goes to Discovery or Disabled
                (_, State::SendCapabilities, ProtocolError::TransmitRetriesExceeded(_)) => Some(State::Discovery),

                // Per spec 8.3.3.2.3: Failure to receive a Request Message results in
                (_, State::SendCapabilities, ProtocolError::RxError(RxError::ReceiveTimeout)) => {
                    // FIXME: Detect when Port Partners have been PD Connected before this error or not.
                    // For now, using whether or not a Contract had been previously established or not
                    match self.contract {
                        Contract::Safe5V => Some(State::Discovery),
                        _ => Some(State::ErrorRecovery),
                    }
                }

                // PowerSwap:     Per spec 8.3.3.19.3.6, the Policy Engine shall transition to ErrorRecovery on RxTimeout or TxSendFail
                // FastPowerSwap: Per spec 8.3.3.19.5.6, the Policy Engine shall transition to ErrorRecovery on RxTimeout or TxSendFail
                (
                    _,
                    State::DrpSwap(SwapState::Power(PowerRoleSwap::WaitSourceOn))
                    | State::DrpSwap(SwapState::FastPower(FastPowerRoleSwap::WaitSourceOn)),
                    ProtocolError::RxError(RxError::ReceiveTimeout) | ProtocolError::TransmitRetriesExceeded(_),
                ) => Some(State::ErrorRecovery),

                // Per spec 8.3.3.2.5: When any Protocol Error occurs, transition to Hard Reset
                (_, State::TransitionSupply(_), _) => Some(State::HardReset),

                // Unexpected messages indicate a protocol error and demand a soft reset.
                // Per spec 6.8.1 Table 6.72 (for non-power-transitioning states).
                // Note: This must come AFTER TransitionSupply check above.
                (_, _, ProtocolError::UnexpectedMessage) => Some(State::SendSoftReset),

                // Per Table 6.72: Unsupported messages in Ready state get Not_Supported response.
                (_, State::Ready, ProtocolError::RxError(RxError::UnsupportedMessage)) => Some(State::SendNotSupported),

                // Per spec 6.6.9.1: Transmission failure (no GoodCRC after retries) triggers Soft Reset.
                // Note: If we're in SoftReset/SendSoftReset state, this is caught above and escalates to Hard Reset.
                (_, _, ProtocolError::TransmitRetriesExceeded(_)) => Some(State::SendSoftReset),

                // Unhandled protocol errors - log and continue.
                (_, _, error) => {
                    error!("Protocol error {:?} in source state transition", error);
                    return Err(Error::Protocol(error));
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

    /// Run the source's state machine continuously.
    ///
    /// The loop is only broken for unrecoverable errors, for example if the port partner is unresponsive.
    pub async fn run(&mut self) -> Result<(), Error> {
        loop {
            self.run_step().await?;
        }
    }

    async fn update_state(&mut self) -> Result<(), Error> {
        trace!("State: {:?}", &self.state);
        let new_state = match &self.state {
            // 8.3.3.2.1 (PE_SR_Startup):
            State::Startup { role_swap } => {
                self.contract = Default::default();
                self.protocol_layer.reset();
                self.caps_counter.reset();

                if *role_swap {
                    self.contract = Contract::Implicit;
                    TimerType::get_timer::<TIMER>(TimerType::SwapSourceStart).await;
                }

                // FIXME: Sources shall remain in the Startup state until a plug is Attached
                // For now, assume that a Source driver will only be ran after an attach occurs

                State::SendCapabilities
            }
            // 8.3.3.2.2 (PE_SRC_Discovery):
            State::Discovery => {
                // NOTE: Incrementing the CapsCounter here is not to spec,
                // but **should** have the same behavior
                if self.caps_counter.increment().is_err() {
                    // If the CapsCounter saturates, enter the Disabled state
                    State::Disabled
                } else {
                    // Else, re-enter SendCapabilities once the SourceCapability timer times out
                    TimerType::get_timer::<TIMER>(TimerType::SourceCapability).await;
                    State::SendCapabilities
                }
            }
            // 8.3.3.2.3 (PE_SRC_Send_Capabilities):
            State::SendCapabilities => {
                // Send capabilities message
                match self.mode {
                    Mode::Spr => {
                        self.protocol_layer
                            .transmit_source_capabilities(&self.device_policy_manager.source_capabilities())
                            .await?
                    }
                    Mode::Epr => {
                        self.protocol_layer
                            .transmit_epr_source_capabilities(&self.device_policy_manager.epr_source_capabilities())
                            .await?
                    }
                };

                // When Capabilities were sent successfully, reset the hard reset counter
                self.hard_reset_counter.reset();

                let request = self.wait_for_sink_request().await?;

                State::NegotiateCapability(request)
            }
            // 8.3.3.2.4 (PE_SRC_Negotiate_Capability):
            State::NegotiateCapability(power_request) => {
                // FIXME: This should be done in the protocol layer
                // If the request is Unknown, attempt to match to its PDO to determine the Kind & re-type the request
                let power_request = match *power_request {
                    PowerSource::Unknown(u) => {
                        match self
                            .device_policy_manager
                            .source_capabilities()
                            .at_object_position(u.object_position())
                        {
                            Some(q) => match q {
                                Kind::FixedSupply | Kind::VariableSupply => {
                                    request::PowerSource::FixedVariableSupply(request::FixedVariableSupply(u.0))
                                }
                                Kind::Battery => request::PowerSource::Battery(request::Battery(u.0)),
                                Kind::Pps => request::PowerSource::Pps(request::Pps(u.0)),
                                Kind::Avs => request::PowerSource::Avs(request::Avs(u.0)),
                            },
                            None => {
                                trace!("Could not match Unknown Power Request to a PDO!");
                                *power_request
                            }
                        }
                    }
                    _ => *power_request,
                };

                let response = self.device_policy_manager.evaluate_request(&power_request).await;

                match response {
                    CapabilityResponse::Accept => State::TransitionSupply(power_request),
                    _ => State::CapabilityResponse(response),
                }
            }
            // 8.3.3.2.5 (PE_SRC_Transition_Supply):
            State::TransitionSupply(power_request) => {
                self.protocol_layer
                    .transmit_control_message(ControlMessageType::Accept)
                    .await?;

                self.contract = Contract::TransitionToExplicit;
                self.device_policy_manager
                    .transition_power(power_request)
                    .await
                    .map_err(|_| Error::PortPartnerUnresponsive)?;
                self.contract = Contract::Explicit(*power_request);

                self.protocol_layer
                    .transmit_control_message(ControlMessageType::PsRdy)
                    .await?;

                State::Ready
            }
            // 8.3.3.2.6 (PE_SRC_Ready):
            State::Ready => {
                // FIXME: Entry: source shall notify the protocol layer of the end of the Atomic Message Sequence (AMS)
                // FIXME: Exit: If source is initiating an AMS, notify the protocol layer that the first message in an AMS will follow

                // FIXME: Entry: The DiscoverIdentityTimer is run when this is a Vconn Source and a PD Connection w/ a Cable Plug needs
                // to be established, i.e., no GoodCrc Message has yet been received in response to a Discover Identity Command.
                let discover_identity_fut = async { core::future::pending::<()>().await };

                // Entry: If current Explicit Contract is for an SPR PPS APDO, then run the SourcePPSCommTimer
                let pps_periodic_fut = async {
                    let power_source = match self.contract {
                        Contract::Explicit(power_source) => Some(power_source),
                        _ => None,
                    };

                    match power_source {
                        Some(PowerSource::Pps(_)) => TimerType::get_timer::<TIMER>(TimerType::SourcePPSComm).await,
                        _ => core::future::pending().await,
                    }
                };

                // Entry: If current Explicit Contract is for EPR Mode, then run the SourceEPRKeepAliveTimer
                let epr_keep_alive_fut = async {
                    match self.mode {
                        Mode::Epr => TimerType::get_timer::<TIMER>(TimerType::SourceEPRKeepAlive).await,
                        Mode::Spr => core::future::pending().await,
                    }
                };

                let timers_fut = async { select3(discover_identity_fut, pps_periodic_fut, epr_keep_alive_fut).await };

                match select3(
                    self.protocol_layer.receive_message(),
                    self.device_policy_manager.get_event(),
                    timers_fut,
                )
                .await
                {
                    Either3::First(message) => {
                        let message = message?;
                        self.match_message_to_state(message).await?
                    }

                    Either3::Second(dpm_event) => match dpm_event {
                        Event::None => State::Ready,
                        Event::UpdatedSourceCapabilities => State::SendCapabilities,
                        Event::RequestSinkCapabilities => State::GetSinkCap,
                        Event::RequestSourceCapabilities => State::DrpGetSourceCap(Mode::Spr),
                        Event::RequestEprSourceCapabilities => State::DrpGetSourceCap(Mode::Epr),
                        Event::ExitEprMode => State::EprMode(EprState::SendExit),
                        Event::RequestVconnSwap => State::VconnSwap {
                            source: VcsSwapSource::Message,
                            state: VcsState::SendSwap,
                        },
                        Event::RequestDataRoleSwap => State::DrpSwap(SwapState::Data(DataRoleSwap::Send)),
                        Event::RequestPowerRoleSwap => State::DrpSwap(SwapState::Power(PowerRoleSwap::Send)),
                    },

                    Either3::Third(timeout_source) => match timeout_source {
                        // DiscoverIdentity Timeout
                        Either3::First(_) => State::HardReset,
                        // PPS Periodic Timeout
                        Either3::Second(_) => State::HardReset,
                        // EPR Keep Alive Timeout
                        Either3::Third(_) => State::HardReset,
                    },
                }
            }
            // 8.3.3.2.7 (PE_SRC_Disabled):
            State::Disabled => {
                // This **SHOULD** put the device in a vSafe5V default power mode
                let source_capabilities = self.device_policy_manager.source_capabilities();
                self.device_policy_manager
                    .transition_power(
                        &PowerSource::new_fixed(
                            request::CurrentRequest::Highest,
                            request::VoltageRequest::Safe5V,
                            &source_capabilities,
                        )
                        .unwrap(),
                    )
                    .await
                    .map_err(|_| ProtocolError::RxError(RxError::HardReset))?;
                // Only respond to `HardReset` Signaling
                loop {
                    if let Err(ProtocolError::RxError(RxError::HardReset)) = self.protocol_layer.receive_message().await
                    {
                        break;
                    }
                }
                State::HardReset
            }
            // 8.3.3.2.8 (PE_SRC_Capability_Response):
            State::CapabilityResponse(response) => {
                let message_type = match response {
                    CapabilityResponse::Reject => ControlMessageType::Reject,
                    CapabilityResponse::Wait => ControlMessageType::Wait,
                    _ => unreachable!(), // `Accept` does not go to this state
                };
                self.protocol_layer.transmit_control_message(message_type).await?;

                match self.contract {
                    Contract::Invalid => {
                        if message_type == ControlMessageType::Reject {
                            State::HardReset
                        } else {
                            State::WaitNewCapabilities
                        }
                    }
                    Contract::Explicit(_) => State::Ready,
                    _ => State::WaitNewCapabilities,
                }
            }
            // 8.3.3.2.9 (PE_SRC_Hard_Reset):
            State::HardReset => {
                // Increment HardResetCounter
                self.hard_reset_counter
                    .increment()
                    .map_err(|_| Error::PortPartnerUnresponsive)?;

                // Transmit Hard Reset Signaling
                self.protocol_layer.hard_reset().await?;

                // Transition to TransitionToDefault when PSHardResetTimer times out
                TimerType::get_timer::<TIMER>(TimerType::PSHardReset).await;

                State::TransitionToDefault
            }
            // 8.3.3.2.10 (PE_SRC_Hard_Reset_Received):
            State::HardResetReceived => {
                // Transition to TransitionToDefault when PSHardResetTimer times out
                TimerType::get_timer::<TIMER>(TimerType::PSHardReset).await;

                State::TransitionToDefault
            }
            // 8.3.3.2.11 (PE_SRC_Transition_to_default):
            State::TransitionToDefault => {
                // Notify DPM about hard reset & turn off Vconn
                self.device_policy_manager
                    .drive_vconn(false)
                    .await
                    .map_err(|_| Error::PortPartnerUnresponsive)?;
                self.device_policy_manager
                    .hard_reset()
                    .await
                    .map_err(|_| Error::PortPartnerUnresponsive)?;
                self.device_policy_manager
                    .drive_vconn(true)
                    .await
                    .map_err(|_| Error::PortPartnerUnresponsive)?;

                // Hard Reset shall cause EPR Mode to be exited
                self.mode = Mode::Spr;

                // Reset contract to default
                self.contract = Contract::Safe5V;

                State::Startup { role_swap: false }
            }
            // 8.3.3.2.12 (PE_SRC_Get_Sink_Cap):
            State::GetSinkCap => {
                // Due to request from DPM, request capabilities from Attached Sink
                match self.mode {
                    Mode::Spr => {
                        self.protocol_layer
                            .transmit_control_message(ControlMessageType::GetSinkCap)
                            .await?
                    }
                    Mode::Epr => {
                        self.protocol_layer
                            .transmit_extended_control_message(ExtendedControlMessageType::EprGetSinkCap)
                            .await?
                    }
                }

                // Inform the DPM of the outcome
                let message = match self.mode {
                    Mode::Spr => {
                        self.protocol_layer
                            .receive_message_type(
                                &[MessageType::Data(DataMessageType::SinkCapabilities)],
                                TimerType::SenderResponse,
                            )
                            .await
                    }
                    Mode::Epr => {
                        self.protocol_layer
                            .receive_message_type(
                                &[MessageType::Extended(ExtendedMessageType::EprSinkCapabilities)],
                                TimerType::SenderResponse,
                            )
                            .await
                    }
                };

                let response = if let Ok(message) = message {
                    // Message success, deal with payload:
                    let capabilities = match message.payload {
                        Some(Payload::Data(Data::SinkCapabilities(caps))) => caps,
                        Some(Payload::Extended(Extended::EprSinkCapabilities(pdos))) => SinkCapabilities(pdos),
                        _ => unreachable!(),
                    };
                    Some(capabilities)
                } else if let Err(ProtocolError::RxError(RxError::ReceiveTimeout)) = message {
                    // Did not receive a capability, which should be handled explicitly here
                    None
                } else {
                    // Propogate the error if it wasn't the receive timeout
                    message?;
                    unreachable!()
                };

                let info = match self.mode {
                    Mode::Spr => Info::SprSinkCapabilities(response),
                    Mode::Epr => Info::EprSinkCapabilities(response),
                };
                self.device_policy_manager.inform(info).await;

                State::Ready
            }
            // 8.3.3.2.13 (PE_SRC_Wait_New_Capabilities):
            State::WaitNewCapabilities => {
                // Transition to SendCapabilities only when the DPM indicates the source capabilities have changed
                let mut wait_time: usize = 0;
                loop {
                    match select(self.device_policy_manager.get_event(), TIMER::after_millis(5000)).await {
                        Either::First(event) => {
                            if event == Event::UpdatedSourceCapabilities {
                                break;
                            }
                        }
                        Either::Second(_timeout) => {
                            wait_time += 5;
                            warn!(
                                "{} seconds have passed waiting for updated source capabilities!",
                                wait_time
                            );
                        }
                    }
                }
                State::SendCapabilities
            }
            // 8.3.3.2.14 (PE_SRC_EPR_Keep_Alive):
            State::EprKeepAlive => {
                self.protocol_layer
                    .transmit_extended_control_message(ExtendedControlMessageType::EprKeepAlive)
                    .await?;
                State::Ready
            }
            // 8.3.3.2.15 (PE_SRC_Give_Source_Cap):
            State::GiveSourceCap => {
                match self.mode {
                    Mode::Spr => {
                        self.protocol_layer
                            .transmit_source_capabilities(&self.device_policy_manager.source_capabilities())
                            .await?
                    }
                    Mode::Epr => {
                        self.protocol_layer
                            .transmit_epr_source_capabilities(&self.device_policy_manager.epr_source_capabilities())
                            .await?
                    }
                }
                State::Ready
            }

            // 8.3.3.4 SOP Soft Reset & Protocol Error
            // 8.3.3.4.1.1 (PE_SRC_Send_Soft_Reset)
            State::SendSoftReset => {
                // Soft reset the protocol layer and send a SoftReset message
                self.protocol_layer.reset();
                self.protocol_layer
                    .transmit_control_message(ControlMessageType::SoftReset)
                    .await?;

                // Wait for an Accept message
                self.protocol_layer
                    .receive_message_type(
                        &[MessageType::Control(ControlMessageType::Accept)],
                        TimerType::SenderResponse,
                    )
                    .await?;

                State::SendCapabilities
            }
            // 8.3.3.4.1.2 (PE_SRC_Soft_Reset)
            State::SoftReset => {
                self.protocol_layer.reset();
                self.protocol_layer
                    .transmit_control_message(ControlMessageType::Accept)
                    .await?;

                State::SendCapabilities
            }

            // 8.3.3.6 Not Supported Message
            // 8.3.3.6.1.1 (PE_SRC_Not_Supported):
            State::SendNotSupported => {
                self.protocol_layer
                    .transmit_control_message(ControlMessageType::NotSupported)
                    .await?;

                State::Ready
            }
            // 8.3.3.6.1.2 (PE_SRC_Not_Supported_Received):
            State::NotSupportedReceived => {
                // FIXME: Entry: Inform the Device Policy Manager
                State::Ready
            }

            // 8.3.3.19 Dual-Role Port States
            State::DrpSwap(swap_state) => match swap_state {
                SwapState::Data(dr) => self.execute_data_role_swap_state(*dr).await?,
                SwapState::Power(pr) => self.execute_power_role_swap_state(*pr).await?,
                SwapState::FastPower(fpr) => self.execute_fast_power_role_swap_state(*fpr).await?,
            },
            // 8.3.3.19.7.1 (PE_DR_SRC_Get_Source_Cap):
            State::DrpGetSourceCap(mode) => {
                let result = match mode {
                    Mode::Spr => self.get_source_capabilities().await,
                    Mode::Epr => self.get_epr_source_capabilities().await,
                };

                let caps = match result {
                    Ok(caps) => Some(caps),
                    Err(err) => match err {
                        Error::Protocol(ProtocolError::RxError(RxError::ReceiveTimeout)) => None,
                        _ => return Err(err),
                    },
                };

                self.device_policy_manager
                    .inform(Info::RemoteSourceCapabilities(caps))
                    .await;
                State::Ready
            }
            // 8.3.3.19.8.1 (PE_DR_SRC_Give_Sink_Cap):
            State::DrpGiveSinkCap(mode) => {
                let sink_caps = self.device_policy_manager.sink_capabilities().await;

                match mode {
                    Mode::Spr => {
                        self.protocol_layer.transmit_sink_capabilities(sink_caps).await?;
                    }
                    Mode::Epr => {
                        self.protocol_layer.transmit_epr_sink_capabilities(sink_caps).await?;
                    }
                }
                State::Ready
            }

            // Custom State - Exit source running and signal to program to begin Sink
            State::PrSwapToSinkStartup => {
                // FIXME: Switch to sink policy manager due to power swap
                Err(Error::SwapToSink)?
            }

            // 8.3.3.20 Source Vconn Swap
            State::VconnSwap { source, state } => self.execute_vconn_swap_state(*source, *state).await?,

            // 8.3.3.26 EPR States
            // FIXME: Source EPR
            State::EprMode(state) => self.execute_epr_state(*state).await?,

            // 8.3.3.28.1
            State::ErrorRecovery => {
                error!("Entered Error Recovery state! Reconnection required.");
                Err(Error::ReconnectionRequired)?
            }
        };

        self.state = new_state;

        Ok(())
    }

    /// 8.3.3.19.1 DFP to UFP Data Role Swap, 8.3.3.19.2 UFP to DFP Data Role Swap
    async fn execute_data_role_swap_state(&mut self, state: DataRoleSwap) -> Result<State, Error> {
        match state {
            // 8.3.3.19.1.2 (PE_DRS_DFP_UFP_Evaluate_Swap, PE_DRS_UFP_DFP_Evaluate_Swap):
            DataRoleSwap::Evaluate => match self.device_policy_manager.evaluate_swap_request(SwapType::Data).await {
                true => Ok(State::DrpSwap(SwapState::Data(DataRoleSwap::Accept))),
                false => Ok(State::DrpSwap(SwapState::Data(DataRoleSwap::Reject))),
            },
            // 8.3.3.19.1.3 (PE_DRS_DFP_UFP_Accept_Swap, PE_DRS_UFP_DFP_Accept_Swap):
            DataRoleSwap::Accept => {
                self.protocol_layer
                    .transmit_control_message(ControlMessageType::Accept)
                    .await?;
                Ok(State::DrpSwap(SwapState::Data(DataRoleSwap::Change)))
            }
            // 8.3.3.19.1.4 (PE_DRS_DFP_UFP_Change_to_UFP_Swap, PE_DRS_UFP_DFP_Change_to_DFP_Swap):
            DataRoleSwap::Change => {
                let new_role = DataRole::from(!bool::from(self.protocol_layer.header().port_data_role()));
                self.device_policy_manager.swap_data_role(new_role).await;
                Ok(State::Ready)
            }
            // 8.3.3.19.1.5 (PE_DRS_DFP_UFP_Send_Swap, PE_DRS_UFP_DFP_Send_Swap):
            DataRoleSwap::Send => {
                self.protocol_layer
                    .transmit_control_message(ControlMessageType::DrSwap)
                    .await?;

                let message = self
                    .protocol_layer
                    .receive_message_type(
                        &[
                            MessageType::Control(ControlMessageType::Accept),
                            MessageType::Control(ControlMessageType::Reject),
                            MessageType::Control(ControlMessageType::Wait),
                        ],
                        TimerType::SenderResponse,
                    )
                    .await?;

                match message.header.message_type() {
                    MessageType::Control(ControlMessageType::Accept) => {
                        Ok(State::DrpSwap(SwapState::Data(DataRoleSwap::Change)))
                    }

                    MessageType::Control(ControlMessageType::Reject)
                    | MessageType::Control(ControlMessageType::Wait) => Ok(State::Ready),

                    _ => Err(Error::Protocol(ProtocolError::UnexpectedMessage)),
                }
            }
            // 8.3.3.19.1.6 (PE_DRS_DFP_UFP_Reject_Swap, PE_DRS_UFP_DFP_Reject_Swap):
            DataRoleSwap::Reject => {
                // FIXME: Wait Message logic
                self.protocol_layer
                    .transmit_control_message(ControlMessageType::Reject)
                    .await?;
                Ok(State::Ready)
            }
        }
    }

    /// 8.3.3.19.3 Source to Sink Power Role Swap
    async fn execute_power_role_swap_state(&mut self, state: PowerRoleSwap) -> Result<State, Error> {
        match state {
            // 8.3.3.19.3.2 (PE_PRS_SRC_SNK_Evaluate_Swap):
            PowerRoleSwap::Evaluate => match self.device_policy_manager.evaluate_swap_request(SwapType::Power).await {
                true => Ok(State::DrpSwap(SwapState::Power(PowerRoleSwap::Accept))),
                false => Ok(State::DrpSwap(SwapState::Power(PowerRoleSwap::Reject))),
            },
            // 8.3.3.19.3.3 (PE_PRS_SRC_SNK_Accept_Swap):
            PowerRoleSwap::Accept => {
                self.protocol_layer
                    .transmit_control_message(ControlMessageType::Accept)
                    .await?;
                Ok(State::DrpSwap(SwapState::Power(PowerRoleSwap::TransitionToOff)))
            }
            // 8.3.3.19.3.4 (PE_PRS_SRC_SNK_Transition_to_off):
            PowerRoleSwap::TransitionToOff => {
                self.device_policy_manager.disable_source().await;
                Ok(State::DrpSwap(SwapState::Power(PowerRoleSwap::AssertRd)))
            }
            // 8.3.3.19.3.5 (PE_PRS_SRC_SNK_Assert_Rd):
            PowerRoleSwap::AssertRd => {
                self.device_policy_manager.cc_sink().await;
                Ok(State::DrpSwap(SwapState::Power(PowerRoleSwap::WaitSourceOn)))
            }
            // 8.3.3.19.3.6 (PE_PRS_SRC_SNK_Wait_Source_on):
            PowerRoleSwap::WaitSourceOn => {
                self.protocol_layer
                    .transmit_control_message(ControlMessageType::PsRdy)
                    .await?;

                self.protocol_layer
                    .receive_message_type(
                        &[MessageType::Control(ControlMessageType::PsRdy)],
                        TimerType::PSSourceOnSpr,
                    )
                    .await?;

                Ok(State::PrSwapToSinkStartup)
            }
            // 8.3.3.19.3.7 (PE_PRS_SRC_SNK_Send_Swap):
            PowerRoleSwap::Send => {
                self.protocol_layer
                    .transmit_control_message(ControlMessageType::PrSwap)
                    .await?;

                let message = self
                    .protocol_layer
                    .receive_message_type(
                        &[
                            MessageType::Control(ControlMessageType::Accept),
                            MessageType::Control(ControlMessageType::Reject),
                            MessageType::Control(ControlMessageType::Wait),
                        ],
                        TimerType::SenderResponse,
                    )
                    .await?;

                match message.header.message_type() {
                    MessageType::Control(ControlMessageType::Accept) => {
                        Ok(State::DrpSwap(SwapState::Power(PowerRoleSwap::TransitionToOff)))
                    }

                    MessageType::Control(ControlMessageType::Reject)
                    | MessageType::Control(ControlMessageType::Wait) => Ok(State::Ready),

                    _ => Err(Error::Protocol(ProtocolError::UnexpectedMessage)),
                }
            }
            // 8.3.3.18.3.8 (PE_PRS_SRC_SNK_Reject_Swap):
            PowerRoleSwap::Reject => {
                // FIXME: Wait Message logic
                self.protocol_layer
                    .transmit_control_message(ControlMessageType::Reject)
                    .await?;
                Ok(State::Ready)
            }
        }
    }

    /// 8.3.3.19.5 Source to Sink Fast Role Swap
    async fn execute_fast_power_role_swap_state(&mut self, state: FastPowerRoleSwap) -> Result<State, Error> {
        match state {
            // 8.3.3.19.5.2 (PE_FRS_SRC_SNK_Evaluate_Swap):
            FastPowerRoleSwap::Evaluate => match self.device_policy_manager.fr_swap_signaled().await {
                true => Ok(State::DrpSwap(SwapState::FastPower(FastPowerRoleSwap::Accept))),
                false => Ok(State::HardReset),
            },
            // 8.3.3.19.5.3 (PE_FRS_SRC_Accept):
            FastPowerRoleSwap::Accept => {
                match self
                    .protocol_layer
                    .transmit_control_message(ControlMessageType::Accept)
                    .await
                {
                    Ok(_) => Ok(State::DrpSwap(SwapState::FastPower(FastPowerRoleSwap::TransitionToOff))),
                    _ => Ok(State::HardReset), // Soft Reset shall **not** be initiated in this case
                }
            }
            // 8.3.3.19.5.4 (PE_FRS_SRC_Transition_to_off):
            FastPowerRoleSwap::TransitionToOff => {
                self.device_policy_manager.discharge_vbus().await;
                Ok(State::DrpSwap(SwapState::FastPower(FastPowerRoleSwap::AssertRd)))
            }
            // 8.3.3.19.5.5 (PE_FRS_SRC_Assert_Rd):
            FastPowerRoleSwap::AssertRd => {
                self.device_policy_manager.cc_sink().await;
                Ok(State::DrpSwap(SwapState::FastPower(FastPowerRoleSwap::WaitSourceOn)))
            }
            // 8.3.3.19.5.6 (PE_FRS_SRC_Wait_Source_on):
            FastPowerRoleSwap::WaitSourceOn => {
                self.protocol_layer
                    .transmit_control_message(ControlMessageType::PsRdy)
                    .await?;

                self.protocol_layer
                    .receive_message_type(
                        &[MessageType::Control(ControlMessageType::PsRdy)],
                        TimerType::PSSourceOnSpr,
                    )
                    .await?;

                Ok(State::PrSwapToSinkStartup)
            }
        }
    }

    // 8.3.3.20 Vconn Swap
    async fn execute_vconn_swap_state(&mut self, source: VcsSwapSource, state: VcsState) -> Result<State, Error> {
        match state {
            // 8.3.3.20.1 (PE_VCS_Send_Swap):
            VcsState::SendSwap => {
                self.protocol_layer
                    .transmit_control_message(ControlMessageType::VconnSwap)
                    .await?;

                let message = self
                    .protocol_layer
                    .receive_message_type(
                        &[
                            MessageType::Control(ControlMessageType::Accept),
                            MessageType::Control(ControlMessageType::Reject),
                            MessageType::Control(ControlMessageType::Wait),
                            MessageType::Control(ControlMessageType::NotSupported),
                        ],
                        TimerType::SenderResponse,
                    )
                    .await;

                if let Err(err) = message {
                    match err {
                        ProtocolError::RxError(RxError::ReceiveTimeout) => Ok(State::Ready),
                        _ => Err(err)?,
                    }
                } else {
                    let message = message.unwrap();
                    match message.header.message_type() {
                        MessageType::Control(ControlMessageType::Accept) => match self.vconn_source {
                            true => Ok(State::VconnSwap {
                                source,
                                state: VcsState::WaitForVconn,
                            }),
                            false => Ok(State::VconnSwap {
                                source,
                                state: VcsState::TurnOnVconn,
                            }),
                        },
                        MessageType::Control(ControlMessageType::Reject)
                        | MessageType::Control(ControlMessageType::Wait) => Ok(State::Ready),
                        // May also transition to ForceVconn if NotSupported message and port presently not vconn source
                        MessageType::Control(ControlMessageType::NotSupported) => Ok(State::NotSupportedReceived),
                        _ => unreachable!(),
                    }
                }
            }
            // 8.3.3.20.2 (PE_VCS_Evaluate_Swap):
            VcsState::EvaluateSwap => match self.device_policy_manager.evaluate_vconn_swap_request().await {
                true => Ok(State::VconnSwap {
                    source,
                    state: VcsState::AcceptSwap,
                }),
                false => Ok(State::VconnSwap {
                    source,
                    state: VcsState::RejectSwap,
                }),
            },
            // 8.3.3.20.3 (PE_VCS_Accept_Swap):
            VcsState::AcceptSwap => {
                self.protocol_layer
                    .transmit_control_message(ControlMessageType::Accept)
                    .await?;
                match self.vconn_source {
                    true => Ok(State::VconnSwap {
                        source,
                        state: VcsState::WaitForVconn,
                    }),
                    false => Ok(State::VconnSwap {
                        source,
                        state: VcsState::TurnOnVconn,
                    }),
                }
            }
            // 8.3.3.20.4 (PE_VCS_Reject_Swap):
            VcsState::RejectSwap => {
                // FIXME: Wait Message logic
                self.protocol_layer
                    .transmit_control_message(ControlMessageType::Reject)
                    .await?;
                match source {
                    VcsSwapSource::Message => Ok(State::Ready),
                    VcsSwapSource::Epr => Ok(State::EprMode(EprState::DiscoverCable)),
                }
            }
            // 8.3.3.20.5 (PE_VCS_Wait_for_Vconn):
            VcsState::WaitForVconn => {
                self.protocol_layer
                    .receive_message_type(&[MessageType::Control(ControlMessageType::PsRdy)], TimerType::VCONNOn)
                    .await?;

                Ok(State::VconnSwap {
                    source,
                    state: VcsState::TurnOffVconn,
                })
            }
            // 8.3.3.20.6 (PE_VCS_Turn_Off_Vconn):
            VcsState::TurnOffVconn => {
                self.device_policy_manager
                    .drive_vconn(false)
                    .await
                    .map_err(|_| Error::ReconnectionRequired)?;
                match source {
                    VcsSwapSource::Message => Ok(State::Ready),
                    VcsSwapSource::Epr => Ok(State::EprMode(EprState::DiscoverCable)),
                }
            }
            // 8.3.3.20.7 (PE_VCS_Turn_On_Vconn):
            VcsState::TurnOnVconn => {
                self.device_policy_manager
                    .drive_vconn(true)
                    .await
                    .map_err(|_| Error::ReconnectionRequired)?;
                Ok(State::VconnSwap {
                    source,
                    state: VcsState::SendPsRdy,
                })
            }
            // 8.3.3.20.8 (PE_VCS_Send_PS_Rdy):
            VcsState::SendPsRdy => {
                self.protocol_layer
                    .transmit_control_message(ControlMessageType::PsRdy)
                    .await?;
                match source {
                    VcsSwapSource::Message => Ok(State::Ready),
                    VcsSwapSource::Epr => Ok(State::EprMode(EprState::DiscoverCable)),
                }
            }
            // 8.3.3.20.9 (PE_VCS_Force_Vconn):
            VcsState::VcsForceVconn => {
                self.device_policy_manager
                    .drive_vconn(true)
                    .await
                    .map_err(|_| Error::ReconnectionRequired)?;
                match source {
                    VcsSwapSource::Message => Ok(State::Ready),
                    VcsSwapSource::Epr => Ok(State::EprMode(EprState::DiscoverCable)),
                }
            }
        }
    }

    // 8.3.3.26 EPR States
    async fn execute_epr_state(&mut self, state: EprState) -> Result<State, Error> {
        match state {
            // 8.3.3.26.1.1 (PE_SRC_Evaluate_EPR_Mode_Entry):
            EprState::Entry => match self.device_policy_manager.epr_capable() {
                true => Ok(State::EprMode(EprState::EntryAck)),
                false => Ok(State::EprMode(EprState::EntryFailed(
                    epr_mode::DataEnterFailed::SourceUnableToEnterEprMode.into(),
                ))),
            },
            // 8.3.3.26.1.2 (PE_SRC_EPR_Mode_Entry_Ack):
            EprState::EntryAck => {
                self.protocol_layer
                    .transmit_epr_mode(epr_mode::Action::EnterAcknowledged, 0)
                    .await?;

                match (self.device_policy_manager.epr_cable_good(), self.vconn_source) {
                    (false, true) => Ok(State::VconnSwap {
                        source: VcsSwapSource::Epr,
                        state: VcsState::SendSwap,
                    }),
                    (false, false) => Ok(State::EprMode(EprState::DiscoverCable)),
                    (true, _) => Ok(State::EprMode(EprState::EvaluateCable)),
                }
            }
            // 8.3.3.26.1.3 (PE_SRC_EPR_Mode_Discover_Cable):
            EprState::DiscoverCable => {
                match self.vconn_source {
                    // FIXME: Discovery is done implicitly through DPM right now,
                    // switch to using PE_INIT_PORT_VDM_Identity_Request
                    true => Ok(State::EprMode(EprState::EvaluateCable)),
                    false => Ok(State::EprMode(EprState::EntryFailed(
                        epr_mode::DataEnterFailed::SourceFailedToBecomeVconnSource.into(),
                    ))),
                }
            }
            // 8.3.3.26.1.4 (PE_SRC_EPR_Mode_Evaluate_Cable_EPR):
            EprState::EvaluateCable => match self.device_policy_manager.epr_cable_good() {
                true => Ok(State::EprMode(EprState::EntrySucceeded)),
                false => Ok(State::EprMode(EprState::EntryFailed(
                    epr_mode::DataEnterFailed::CableNotEprCapable.into(),
                ))),
            },
            // 8.3.3.26.1.5 (PE_SRC_EPR_Mode_Entry_Succeeded):
            EprState::EntrySucceeded => {
                self.protocol_layer
                    .transmit_epr_mode(epr_mode::Action::EnterSucceeded, 0)
                    .await?;
                // FIXME: Set EPR headers
                self.mode = Mode::Epr;
                Ok(State::SendCapabilities)
            }
            // 8.3.3.26.1.6 (PE_SRC_EPR_Mode_Entry_Failed):
            EprState::EntryFailed(data) => {
                self.protocol_layer
                    .transmit_epr_mode(epr_mode::Action::EnterFailed, data)
                    .await?;
                Ok(State::Ready)
            }
            // 8.3.3.26.3.1 (PE_SRC_Send_EPR_Mode_Exit):
            EprState::SendExit => {
                self.protocol_layer.transmit_epr_mode(epr_mode::Action::Exit, 0).await?;
                // FIXME: Clear EPR headers
                self.mode = Mode::Spr;
                Ok(State::SendCapabilities)
            }
            // 8.3.3.26.3.2 (PE_SRC_EPR_Mode_Exit_Received):
            EprState::ExitReceived => {
                self.mode = Mode::Spr;
                // FIXME: Clear EPR headers
                match self.contract {
                    Contract::Explicit(power_source) => {
                        let epr_capabilities = self.device_policy_manager.epr_source_capabilities();

                        if epr_capabilities.has_epr_pdo_in_spr_positions() {
                            Ok(State::HardReset)
                        } else if epr_capabilities
                            .spr_pdos()
                            .any(|(pos, _pdo)| pos == power_source.object_position())
                        {
                            Ok(State::SendCapabilities)
                        } else {
                            Ok(State::HardReset)
                        }
                    }
                    Contract::Safe5V | Contract::Implicit => Ok(State::Ready),
                    _ => Ok(State::HardReset),
                }
            }
        }
    }

    async fn match_message_to_state(&mut self, message: Message) -> Result<State, Error> {
        let state = match message.header.message_type() {
            MessageType::Data(DataMessageType::Request) => {
                if self.mode != Mode::Spr {
                    return Err(Error::Protocol(ProtocolError::RxError(RxError::HardReset)));
                }

                let request = match message.payload {
                    Some(Payload::Data(Data::Request(power_source))) => power_source,
                    _ => unreachable!(),
                };

                State::NegotiateCapability(request)
            }

            MessageType::Data(DataMessageType::EprRequest) => {
                if self.mode != Mode::Epr {
                    return Err(Error::Protocol(ProtocolError::RxError(RxError::HardReset)));
                }

                let request = match message.payload {
                    Some(Payload::Data(Data::Request(power_source))) => power_source,
                    _ => unreachable!(),
                };

                State::NegotiateCapability(request)
            }

            MessageType::Data(DataMessageType::EprMode) => {
                if let Some(Payload::Data(Data::EprMode(epr_data))) = message.payload {
                    match epr_data.action() {
                        epr_mode::Action::Enter => match self.mode {
                            Mode::Spr => State::EprMode(EprState::Entry),
                            Mode::Epr => State::HardReset,
                        },
                        epr_mode::Action::Exit => match self.mode {
                            Mode::Spr => State::HardReset,
                            Mode::Epr => State::EprMode(EprState::ExitReceived),
                        },
                        _ => State::SendNotSupported,
                    }
                } else {
                    State::SendNotSupported
                }
            }

            MessageType::Control(ControlMessageType::SoftReset) => State::SoftReset,

            MessageType::Control(ControlMessageType::GetSourceCap) => match self.mode {
                Mode::Spr => State::SendCapabilities,
                Mode::Epr => State::GiveSourceCap,
            },

            MessageType::Control(ControlMessageType::VconnSwap) => State::VconnSwap {
                source: VcsSwapSource::Message,
                state: VcsState::EvaluateSwap,
            },

            MessageType::Control(ControlMessageType::DrSwap) => {
                if !self.dual_role {
                    State::SendNotSupported
                } else if self.mode == Mode::Epr {
                    State::HardReset
                } else {
                    State::DrpSwap(SwapState::Data(DataRoleSwap::Evaluate))
                }
            }

            // 8.3.3.19.3.1
            MessageType::Control(ControlMessageType::PrSwap) => match self.dual_role {
                true => State::DrpSwap(SwapState::Power(PowerRoleSwap::Evaluate)),
                false => State::SendNotSupported,
            },

            // 8.3.3.19.5.1
            MessageType::Control(ControlMessageType::FrSwap) => match self.dual_role {
                true => State::DrpSwap(SwapState::FastPower(FastPowerRoleSwap::Evaluate)),
                false => State::SendNotSupported,
            },

            MessageType::Control(ControlMessageType::GetSinkCap) => match self.dual_role {
                true => State::DrpGiveSinkCap(Mode::Spr),
                false => State::SendNotSupported,
            },

            MessageType::Control(ControlMessageType::NotSupported) => State::NotSupportedReceived,

            MessageType::Extended(ExtendedMessageType::ExtendedControl) => {
                if let Some(Payload::Extended(Extended::ExtendedControl(ctrl))) = &message.payload {
                    match ctrl.message_type() {
                        ExtendedControlMessageType::EprGetSourceCap => match self.mode {
                            Mode::Spr => State::GiveSourceCap,
                            Mode::Epr => State::SendCapabilities,
                        },
                        ExtendedControlMessageType::EprGetSinkCap => match self.dual_role {
                            true => State::DrpGiveSinkCap(Mode::Epr),
                            false => State::SendNotSupported,
                        },
                        ExtendedControlMessageType::EprKeepAlive => State::EprKeepAlive,
                        ExtendedControlMessageType::EprKeepAliveAck => State::SendNotSupported, // FIXME: Source EPR
                    }
                } else {
                    State::SendNotSupported
                }
            }

            _ => State::SendNotSupported,
        };

        Ok(state)
    }

    /// Wait for the sink to request a capability after the source has published all capabilities.
    /// Handles both SPR & EPR Modes.
    async fn wait_for_sink_request(&mut self) -> Result<request::PowerSource, Error> {
        // Receive a Request or EPR Request, depending on the current mode
        let message = match self.mode {
            Mode::Spr => {
                let message = self.protocol_layer.wait_for_request().await?;
                trace!("Sink Request {:?}", message);

                if message.header.message_type() != MessageType::Data(DataMessageType::Request) {
                    unreachable!()
                }

                message
            }
            Mode::Epr => {
                let message = self.protocol_layer.wait_for_epr_request().await?;
                trace!("Sink EPR Request {:?}", message);

                if message.header.message_type() != MessageType::Data(DataMessageType::EprRequest) {
                    unreachable!()
                }

                message
            }
        };

        // Extract the power source from the request
        let request = match message.payload {
            Some(Payload::Data(Data::Request(power_source))) => power_source,
            _ => unreachable!(),
        };

        Ok(request)
    }

    /// GetSourceCap
    async fn get_source_capabilities(&mut self) -> Result<SourceCapabilities, Error> {
        self.protocol_layer
            .transmit_control_message(ControlMessageType::GetSourceCap)
            .await?;

        let message = self
            .protocol_layer
            .receive_message_type(
                &[MessageType::Data(DataMessageType::SourceCapabilities)],
                TimerType::SenderResponse,
            )
            .await?;

        match message.payload {
            Some(Payload::Data(Data::SourceCapabilities(caps))) => Ok(caps),
            _ => unreachable!(),
        }
    }

    /// GetSourceCap
    async fn get_epr_source_capabilities(&mut self) -> Result<SourceCapabilities, Error> {
        self.protocol_layer
            .transmit_extended_control_message(ExtendedControlMessageType::EprGetSourceCap)
            .await?;

        let message = self
            .protocol_layer
            .receive_message_type(
                &[MessageType::Extended(ExtendedMessageType::EprSourceCapabilities)],
                TimerType::SenderResponse,
            )
            .await?;

        match message.payload {
            Some(Payload::Extended(Extended::EprSourceCapabilities(cap))) => Ok(SourceCapabilities(cap)),
            _ => unreachable!(),
        }
    }
}

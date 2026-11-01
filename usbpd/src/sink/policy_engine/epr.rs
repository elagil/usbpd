//! Sink EPR policy engine states.
//!
//! Per PD 3.2 Sec. 8.3.3.26 (EPR Mode Entry/Exit) and Sec. 8.3.3.3.11 (EPR Keep-Alive).
//! Everything EPR-specific in the sink policy engine lives here.
//! The shared state machine only references this module through gated arms.

use uom::si::power::watt;
use usbpd_traits::Driver;

use super::super::device_policy_manager::SinkDpm;
use super::{Error, Mode, Sink, State};
use crate::Contract;
use crate::protocol_layer::message::data::Data;
use crate::protocol_layer::message::data::epr_mode::{self, Action};
use crate::protocol_layer::message::data::request::PowerSource;
use crate::protocol_layer::message::data::sink_capabilities::SinkCapabilities;
use crate::protocol_layer::message::data::source_capabilities::SourceCapabilities;
use crate::protocol_layer::message::extended::extended_control::ExtendedControlMessageType;
use crate::protocol_layer::message::header::{DataMessageType, ExtendedMessageType, MessageType};
use crate::protocol_layer::message::{Payload, extended};
use crate::protocol_layer::{ProtocolError, SinkProtocolLayer};
use crate::timers::{EprTimerType, Timer, TimerType};
use crate::units;

/// EPR policy engine states.
///
/// PD 3.2 Sec. 8.3.3.26.2/4 Sink EPR Mode Entry/Exit, Sec. 8.3.3.3.11 Keep-Alive.
#[derive(Debug, Clone, Copy)]
pub(crate) enum EprState {
    /// Send EPR_Mode (Enter) with the EPR Sink Operational PDP.
    Entry(units::Power),
    /// Wait for EnterSucceeded after EnterAcknowledged.
    EntryWaitForResponse,
    /// Wait for the source's EPR_Source_Capabilities after entry.
    WaitForCapabilities,
    /// Send EPR_Mode (Exit).
    SendExit,
    /// EPR_Mode (Exit) received from the source.
    ExitReceived,
    /// Send EPR_KeepAlive.
    KeepAlive,
}

/// Execute an EPR state of the sink policy engine.
pub(super) async fn execute_epr_state<DRIVER: Driver, TIMER: Timer, DPM: SinkDpm>(
    sink: &mut Sink<DRIVER, TIMER, DPM>,
    state: EprState,
) -> Result<State, Error> {
    let new_state = match state {
        // PD 3.2 Sec. 8.3.3.26.2.1 (PE_SNK_Send_EPR_Mode_Entry):
        EprState::Entry(operational_pdp) => {
            // Request entry into EPR mode.
            // Per PD 3.2 Sec. 8.3.3.26.2.1 (PE_SNK_Send_EPR_Mode_Entry), sink sends EPR_Mode (Enter)
            // and starts SenderResponseTimer and SinkEPREnterTimer.
            //
            // Per PD 3.2 Sec. 6.4.10, the Data field shall be set to the EPR Sink Operational PDP.
            //
            // Note: The spec says SinkEPREnterTimer (500 ms) should run continuously across
            // both EprModeEntry and EprEntryWaitForResponse states until stopped or timeout.
            // Our implementation uses SenderResponseTimer (30 ms) here and a fresh
            // SinkEPREnterTimer (500 ms) in EprEntryWaitForResponse. This means the total
            // timeout could be ~530 ms instead of 500 ms in edge cases. However, this is
            // within the spec's allowed range (tEnterEPR max = 550 ms per PD 3.2 Tab. 6.71).
            let pdp_watts: u8 = operational_pdp.get::<watt>() as u8;
            sink.protocol_layer.transmit_epr_mode(Action::Enter, pdp_watts).await?;

            // Wait for EnterAcknowledged with SenderResponseTimer (spec step 9-14)
            let message = sink
                .protocol_layer
                .receive_message_type(
                    &[MessageType::Data(DataMessageType::EprMode)],
                    TimerType::SenderResponse,
                )
                .await?;

            // A partner-supplied frame may parse as `Data::Unknown` (e.g. wrong payload length).
            // Per PD 3.2 Sec. 8.3.3.26.2.1 any invalid `EPR_Mode` message leads to Soft Reset.
            let Some(Payload::Data(Data::EprMode(epr_mode))) = message.payload else {
                return Ok(State::SendSoftReset);
            };

            match epr_mode.action() {
                Action::EnterAcknowledged => {
                    // Source acknowledged, now wait for EnterSucceeded
                    State::EprMode(EprState::EntryWaitForResponse)
                }
                Action::EnterSucceeded => {
                    // Source skipped EnterAcknowledged and went directly to EnterSucceeded
                    sink.mode = Mode::Epr;
                    State::EprMode(EprState::WaitForCapabilities)
                }
                Action::Exit => State::EprMode(EprState::ExitReceived),
                Action::EnterFailed => {
                    // Per PD 3.2 Sec. 8.3.3.26.2.1: EnterFailed -> Soft Reset
                    // Notify DPM of the failure reason before soft reset
                    let reason = epr_mode::DataEnterFailed::from(epr_mode.data());
                    sink.device_policy_manager.epr_mode_entry_failed(reason).await;
                    State::SendSoftReset
                }
                // Per PD 3.2 Sec. 8.3.3.26.2.1: any other EPR_Mode message -> Soft Reset
                _ => State::SendSoftReset,
            }
        }
        // PD 3.2 Sec. 8.3.3.26.2.2 (PE_SNK_EPR_Mode_Wait_For_Response):
        EprState::EntryWaitForResponse => {
            // Wait for EnterSucceeded after receiving EnterAcknowledged.
            // Per PD 3.2 Sec. 8.3.3.26.2.2 (PE_SNK_EPR_Mode_Wait_For_Response), use SinkEPREnterTimer
            // for the overall timeout while source performs cable discovery.
            let message = sink
                .protocol_layer
                .receive_message_type(
                    &[MessageType::Data(DataMessageType::EprMode)],
                    TimerType::Epr(EprTimerType::SinkEPREnter),
                )
                .await?;

            // A partner-supplied frame may parse as `Data::Unknown` (e.g. wrong payload length).
            // Per PD 3.2 Sec. 8.3.3.26.2.2 any invalid EPR_Mode message leads to Soft Reset.
            let Some(Payload::Data(Data::EprMode(epr_mode))) = message.payload else {
                return Ok(State::SendSoftReset);
            };

            match epr_mode.action() {
                Action::EnterSucceeded => {
                    // EPR mode entry succeeded. Per PD 3.2 Tab. 8.39 step 21-29,
                    // source will automatically send EPR_Source_Capabilities after this.
                    sink.mode = Mode::Epr;
                    State::EprMode(EprState::WaitForCapabilities)
                }
                Action::Exit => State::EprMode(EprState::ExitReceived),
                Action::EnterFailed => {
                    // Per PD 3.2 Sec. 8.3.3.26.2.2: EnterFailed -> Soft Reset
                    // Notify DPM of the failure reason before soft reset
                    let reason = epr_mode::DataEnterFailed::from(epr_mode.data());
                    sink.device_policy_manager.epr_mode_entry_failed(reason).await;
                    State::SendSoftReset
                }
                // Per PD 3.2 Sec. 8.3.3.26.2.2: any other EPR_Mode message -> Soft Reset
                _ => State::SendSoftReset,
            }
        }
        // EPR Mode PE_SNK_Wait_for_Capabilities
        EprState::WaitForCapabilities => {
            // After successful EPR mode entry, source automatically sends EPR_Source_Capabilities.
            // This may be a chunked extended message that requires assembly.
            // Wait for the capabilities and evaluate them.
            let message = sink.protocol_layer.wait_for_source_capabilities().await?;

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
        // PD 3.2 Sec. 8.3.3.26.4.1 (PE_SNK_Send_EPR_Mode_Exit):
        EprState::SendExit => {
            // Inform partner we are exiting EPR.
            sink.protocol_layer.transmit_epr_mode(Action::Exit, 0).await?;
            sink.mode = Mode::Spr;
            State::WaitForCapabilities
        }
        // PD 3.2 Sec. 8.3.3.26.4.2 (PE_SNK_EPR_Mode_Exit_Received):
        EprState::ExitReceived => {
            // Per PD 3.2 Sec. 8.3.3.26.4.2 (PE_SNK_EPR_Mode_Exit_Received):
            // - If in an Explicit Contract with an SPR (A)PDO -> WaitForCapabilities
            // - If NOT in an Explicit Contract with an SPR (A)PDO -> HardReset
            //
            // SPR PDOs are in object positions 1-7, EPR PDOs are in positions 8+.
            // In EPR mode, requests use EprRequest which contains the RDO with object position.
            sink.mode = Mode::Spr;

            let is_epr_pdo_contract = match sink.contract {
                Contract::Explicit(PowerSource::EprRequest(epr)) => {
                    // Extract object position from RDO (bits 28-31)
                    epr.object_position() >= 8
                }
                _ => false,
            };

            if is_epr_pdo_contract {
                State::HardReset
            } else {
                State::WaitForCapabilities
            }
        }
        // PD 3.2 Sec. 8.3.3.3.11 (PE_SNK_EPR_Keep_Alive):
        EprState::KeepAlive => {
            // Per PD 3.2 Sec. 8.3.3.3.11 (PE_SNK_EPR_Keep_Alive):
            // - Entry: Send EPR_KeepAlive message, start SenderResponseTimer
            // - On EPR_KeepAlive_Ack: transition to Ready (which restarts SinkEPRKeepAliveTimer)
            // - On timeout: transition to HardReset
            sink.protocol_layer
                .transmit_extended_control_message(ExtendedControlMessageType::EprKeepAlive)
                .await?;
            match sink
                .protocol_layer
                .receive_message_type(
                    &[MessageType::Extended(ExtendedMessageType::ExtendedControl)],
                    TimerType::SenderResponse,
                )
                .await
            {
                Ok(message) => {
                    if let Some(Payload::Extended(extended::Extended::ExtendedControl(control))) = message.payload {
                        // Per PD 3.2 Tab. 6.67, values not explicitly
                        // defined are Reserved; answer Not_Supported.
                        if control.try_message_type() == Ok(ExtendedControlMessageType::EprKeepAliveAck) {
                            sink.mode = Mode::Epr;
                            State::Ready(false)
                        } else {
                            State::SendNotSupported
                        }
                    } else {
                        State::SendNotSupported
                    }
                }
                Err(_) => State::HardReset,
            }
        }
    };

    Ok(new_state)
}

/// Whether the received message is an `EPR_Source_Capabilities` message.
///
/// Always `false` when compiled without the `epr` feature.
pub(super) fn received_is_epr(message: &crate::protocol_layer::message::Message) -> bool {
    matches!(
        message.header.message_type(),
        MessageType::Extended(ExtendedMessageType::EprSourceCapabilities)
    )
}

/// Transmit sink capabilities in the appropriate mode.
///
/// `Mode::Epr` only exists when the `epr` feature is enabled, so only the EPR branch lives here.
pub(super) async fn transmit_sink_capabilities<DRIVER: Driver, TIMER: Timer>(
    protocol_layer: &mut SinkProtocolLayer<DRIVER, TIMER>,
    capabilities: SinkCapabilities,
) -> Result<(), ProtocolError> {
    protocol_layer.transmit_epr_sink_capabilities(capabilities).await
}

/// Send the EPR_Get_Source_Cap request message.
pub(super) async fn transmit_get_source_cap<DRIVER: Driver, TIMER: Timer>(
    protocol_layer: &mut SinkProtocolLayer<DRIVER, TIMER>,
) -> Result<(), ProtocolError> {
    protocol_layer.0.transmit_epr_source_cap_request().await
}

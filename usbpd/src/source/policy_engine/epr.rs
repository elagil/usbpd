//! Source EPR policy engine states.
//!
//! Per PD 3.2 Sec. 8.3.3.26 (Source EPR Mode Entry/Exit).
//! Everything EPR-specific in the source policy engine lives here.
//! The shared state machine only references this module through gated arms.

use usbpd_traits::Driver;

use super::super::device_policy_manager::SourceDpm;
use super::{Contract, Error, Mode, Source, State};
use crate::protocol_layer::message::Payload;
use crate::protocol_layer::message::data::epr_mode;
use crate::protocol_layer::message::data::source_capabilities::SourceCapabilities;
use crate::protocol_layer::message::extended::Extended;
use crate::protocol_layer::message::extended::extended_control::ExtendedControlMessageType;
use crate::protocol_layer::message::header::{ExtendedMessageType, MessageType};
use crate::protocol_layer::{ProtocolError, SourceProtocolLayer};
use crate::timers::Timer;

/// Source EPR policy engine states.
///
/// PD 3.2 Sec. 8.3.3.26.1/3 Source EPR Mode Entry/Exit.
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) enum EprState {
    /// Evaluate EPR mode entry request from the sink.
    Entry,
    /// Acknowledge entry, discover cable.
    EntryAck,
    /// Discover whether the cable is EPR-capable.
    DiscoverCable,
    /// Evaluate the cable discovery result.
    EvaluateCable,
    /// Send EnterSucceeded.
    EntrySucceeded,
    /// Send EnterFailed with a reason.
    EntryFailed(u8),
    /// Send EPR_Mode (Exit).
    SendExit,
    /// EPR_Mode (Exit) received from the sink.
    ExitReceived,
}

/// Execute an EPR state of the source policy engine.
pub(super) async fn execute_epr_state<DRIVER: Driver, TIMER: Timer, DPM: SourceDpm>(
    source: &mut Source<DRIVER, TIMER, DPM>,
    state: EprState,
) -> Result<State, Error> {
    match state {
        // PD 3.2 Sec. 8.3.3.26.1.1 (PE_SRC_Evaluate_EPR_Mode_Entry):
        EprState::Entry => match source.device_policy_manager.epr_capable() {
            true => Ok(State::EprMode(EprState::EntryAck)),
            false => Ok(State::EprMode(EprState::EntryFailed(
                epr_mode::DataEnterFailed::SourceUnableToEnterEprMode.into(),
            ))),
        },
        // PD 3.2 Sec. 8.3.3.26.1.2 (PE_SRC_EPR_Mode_Entry_Ack):
        EprState::EntryAck => {
            source
                .protocol_layer
                .transmit_epr_mode(epr_mode::Action::EnterAcknowledged, 0)
                .await?;

            match (source.device_policy_manager.epr_cable_good(), source.vconn_source) {
                (false, true) => Ok(State::VconnSwap {
                    source: super::VcsSwapSource::Epr,
                    state: super::VcsState::SendSwap,
                }),
                (false, false) => Ok(State::EprMode(EprState::DiscoverCable)),
                (true, _) => Ok(State::EprMode(EprState::EvaluateCable)),
            }
        }
        // PD 3.2 Sec. 8.3.3.26.1.3 (PE_SRC_EPR_Mode_Discover_Cable):
        EprState::DiscoverCable => {
            if source.vconn_source {
                // FIXME: Discovery is done implicitly through DPM right now,
                // switch to using PE_INIT_PORT_VDM_Identity_Request
                Ok(State::EprMode(EprState::EvaluateCable))
            } else {
                Ok(State::EprMode(EprState::EntryFailed(
                    epr_mode::DataEnterFailed::SourceFailedToBecomeVconnSource.into(),
                )))
            }
        }
        // PD 3.2 Sec. 8.3.3.26.1.4 (PE_SRC_EPR_Mode_Evaluate_Cable_EPR):
        EprState::EvaluateCable => {
            if source.device_policy_manager.epr_cable_good() {
                Ok(State::EprMode(EprState::EntrySucceeded))
            } else {
                Ok(State::EprMode(EprState::EntryFailed(
                    epr_mode::DataEnterFailed::CableNotEprCapable.into(),
                )))
            }
        }
        // PD 3.2 Sec. 8.3.3.26.1.5 (PE_SRC_EPR_Mode_Entry_Succeeded):
        EprState::EntrySucceeded => {
            source
                .protocol_layer
                .transmit_epr_mode(epr_mode::Action::EnterSucceeded, 0)
                .await?;
            // FIXME: Set EPR headers
            source.mode = Mode::Epr;
            Ok(State::SendCapabilities)
        }
        // PD 3.2 Sec. 8.3.3.26.1.6 (PE_SRC_EPR_Mode_Entry_Failed):
        EprState::EntryFailed(data) => {
            source
                .protocol_layer
                .transmit_epr_mode(epr_mode::Action::EnterFailed, data)
                .await?;
            Ok(State::Ready)
        }
        // PD 3.2 Sec. 8.3.3.26.3.1 (PE_SRC_Send_EPR_Mode_Exit):
        EprState::SendExit => {
            source
                .protocol_layer
                .transmit_epr_mode(epr_mode::Action::Exit, 0)
                .await?;
            // FIXME: Clear EPR headers
            source.mode = Mode::Spr;
            Ok(State::SendCapabilities)
        }
        // PD 3.2 Sec. 8.3.3.26.3.2 (PE_SRC_EPR_Mode_Exit_Received):
        EprState::ExitReceived => {
            source.mode = Mode::Spr;
            // FIXME: Clear EPR headers
            match source.contract {
                Contract::Explicit(power_source) => {
                    let epr_capabilities = source.device_policy_manager.epr_source_capabilities();

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

/// Get the partner's EPR source capabilities (EPR_Get_SourceCap).
pub(super) async fn get_epr_source_capabilities<DRIVER: Driver, TIMER: Timer>(
    source_protocol_layer: &mut SourceProtocolLayer<DRIVER, TIMER>,
) -> Result<SourceCapabilities, Error> {
    source_protocol_layer
        .0
        .transmit_extended_control_message(ExtendedControlMessageType::EprGetSourceCap)
        .await?;

    let message = source_protocol_layer
        .receive_message_type(
            &[MessageType::Extended(ExtendedMessageType::EprSourceCapabilities)],
            crate::timers::TimerType::SenderResponse,
        )
        .await?;

    match message.payload {
        Some(Payload::Extended(Extended::EprSourceCapabilities(cap))) => Ok(SourceCapabilities(cap)),
        // A malformed payload is a protocol violation, not a bug; the caller's
        // UnexpectedMessage handler (PD 3.2 Sec. 6.6.2) transitions to Soft Reset.
        _ => Err(Error::Protocol(ProtocolError::UnexpectedMessage)),
    }
}

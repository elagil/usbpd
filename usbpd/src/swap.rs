//! Shared dual-role-port (DRP) state types and shared swap executors.
//!
//! Used by both the sink and source policy engines, which implement the two
//! sides of the same PD 3.2 Sec. 8.3.3.19 and 8.3.3.20 state machines.
//!
//! Power Role Swap and Fast Role Swap states are deliberately not shared:
//! per PD 3.2, each role runs its own state machine family with different states on each side:
//! - sink: PE_PRS_SNK_SRC, PE_FRS_SNK_SRC
//! - source: PE_PRS_SRC_SNK, PE_FRS_SRC_SNK
//!
//! The policy engine [`crate::Error`] enum moves with the state machines and lives
//! at the crate root.

use usbpd_traits::Driver;

use crate::dpm::{DrpSwap, VconnSwap};
use crate::protocol_layer::message::header::{ControlMessageType, MessageType};
use crate::protocol_layer::{ProtocolError, ProtocolLayer, RxError};
use crate::swap_reply::{SwapReply, await_swap_reply};
use crate::timers::{Timer, TimerType};
use crate::{Error, SwapType};

/// State during a Data Role Swap execution (for both directions),
/// per PD 3.2 Sec. 8.3.3.19.1 and 8.3.3.19.2.
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) enum DataRoleSwap {
    Evaluate,
    Accept,
    Change,
    Send,
    Reject,
}

/// PD 3.2 Sec. 8.3.3.20 Vconn Swap. State during a Vconn Swap execution
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) enum VcsState {
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
    ForceVconn,
}

/// The port state a finished shared executor step returns to.
///
/// Parameterizes the shared executors so each role maps the single exit point
/// onto its own ready state (and the source additionally onto its EPR return
/// path for the Vconn Swap executor).
#[derive(Debug, Clone, Copy)]
pub(crate) enum Exit {
    /// Return to the ready state.
    Ready,
}

/// Outcome of one shared executor step.
pub(crate) enum Step<STATE> {
    /// The swap continues with the given next state.
    Continue(STATE),
    /// The swap finished and the port shall return to the mapped exit state.
    Done(Exit),
}

/// Execute one state of the Vconn Swap state machine
/// (PD 3.2 Sec. 8.3.3.20, `PE_VCS`), shared by sink and source.
///
/// The `SendSwap` reply handling differs per role: a `Not_Supported` reply and
/// the reply timeouts surface in `VcsOutcome`, which the caller maps onto its
/// role-specific states.
pub(crate) async fn execute_vconn_swap_state<DRIVER: Driver, TIMER: Timer, H: VconnSwap>(
    protocol_layer: &mut ProtocolLayer<DRIVER, TIMER>,
    dpm: &mut H,
    vconn_source: bool,
    state: VcsState,
) -> Result<VcsOutcome, Error> {
    if let VcsState::SendSwap = state {
        protocol_layer
            .transmit_control_message(ControlMessageType::VconnSwap)
            .await?;

        // Note: The DPM is not informed of the Wait. Proper Wait handling for the
        // VCS initiating side is a known limitation (see CHANGELOG).
        // May also transition to ForceVconn if NotSupported message and port presently not vconn source
        let reply = await_swap_reply(protocol_layer, SwapReply::AcceptRejectWaitNotSupported).await;

        return match reply {
            Ok(ControlMessageType::Accept) => Ok(VcsOutcome::Start(if vconn_source {
                VcsState::WaitForVconn
            } else {
                VcsState::TurnOnVconn
            })),
            // May also transition to ForceVconn if NotSupported message and port presently not vconn source
            Ok(ControlMessageType::NotSupported) => Ok(VcsOutcome::NotSupported),
            // Note: DPM is not informed of a `Wait`; known limitation (see CHANGELOG).
            Ok(_) => Ok(VcsOutcome::Done(Exit::Ready)),
            // Timeout on a Vconn_Swap request is treated as a rejection.
            Err(ProtocolError::RxError(RxError::ReceiveTimeout)) => Ok(VcsOutcome::Done(Exit::Ready)),
            Err(err) => Err(Error::Protocol(err)),
        };
    }

    let next = match state {
        VcsState::SendSwap => unreachable!("handled above"),
        // PD 3.2 Sec. 8.3.3.20.2 (PE_VCS_Evaluate_Swap):
        VcsState::EvaluateSwap => {
            if dpm.evaluate_vconn_swap_request().await {
                VcsState::AcceptSwap
            } else {
                VcsState::RejectSwap
            }
        }
        // PD 3.2 Sec. 8.3.3.20.3 (PE_VCS_Accept_Swap):
        VcsState::AcceptSwap => {
            protocol_layer
                .transmit_control_message(ControlMessageType::Accept)
                .await?;

            if vconn_source {
                VcsState::WaitForVconn
            } else {
                VcsState::TurnOnVconn
            }
        }
        // PD 3.2 Sec. 8.3.3.20.4 (PE_VCS_Reject_Swap):
        VcsState::RejectSwap => {
            // Note: A `Wait` response would require a DPM tri-state decision. Not
            // supported yet (see CHANGELOG Known limitations).
            protocol_layer
                .transmit_control_message(ControlMessageType::Reject)
                .await?;

            return Ok(VcsOutcome::Done(Exit::Ready));
        }
        // PD 3.2 Sec. 8.3.3.20.5 (PE_VCS_Wait_for_Vconn):
        VcsState::WaitForVconn => {
            protocol_layer
                .receive_message_type(&[MessageType::Control(ControlMessageType::PsRdy)], TimerType::VCONNOn)
                .await?;

            VcsState::TurnOffVconn
        }
        // PD 3.2 Sec. 8.3.3.20.6 (PE_VCS_Turn_Off_Vconn):
        VcsState::TurnOffVconn => {
            dpm.drive_vconn(false).await.map_err(|_| Error::ReconnectionRequired)?;

            return Ok(VcsOutcome::Done(Exit::Ready));
        }
        // PD 3.2 Sec. 8.3.3.20.7 (PE_VCS_Turn_On_Vconn):
        VcsState::TurnOnVconn => {
            dpm.drive_vconn(true).await.map_err(|_| Error::ReconnectionRequired)?;
            VcsState::SendPsRdy
        }
        // PD 3.2 Sec. 8.3.3.20.8 (PE_VCS_Send_PS_Rdy):
        VcsState::SendPsRdy => {
            protocol_layer
                .transmit_control_message(ControlMessageType::PsRdy)
                .await?;
            return Ok(VcsOutcome::Done(Exit::Ready));
        }
        // PD 3.2 Sec. 8.3.3.20.9 (PE_VCS_Force_Vconn):
        VcsState::ForceVconn => {
            dpm.drive_vconn(true).await.map_err(|_| Error::ReconnectionRequired)?;
            return Ok(VcsOutcome::Done(Exit::Ready));
        }
    };

    Ok(VcsOutcome::Continue(next))
}

/// Result of one shared Vconn Swap executor step.
pub(crate) enum VcsOutcome {
    /// The swap continues with the given next state.
    Continue(VcsState),
    /// The swap starts over at the given state (reply to a transmitted request).
    Start(VcsState),
    /// The partner answered `Not_Supported` to an initiated swap.
    NotSupported,
    /// The swap finished and the port shall return to the mapped exit state.
    Done(Exit),
}

/// Execute one state of the Data Role Swap state machine
/// (PD 3.2 Sec. 8.3.3.19.1/2, `PE_DRS`), shared by sink and source.
pub(crate) async fn execute_data_role_swap_state<DRIVER: Driver, TIMER: Timer, H: DrpSwap>(
    protocol_layer: &mut ProtocolLayer<DRIVER, TIMER>,
    dpm: &mut H,
    state: DataRoleSwap,
) -> Result<Step<DataRoleSwap>, Error> {
    let next = match state {
        // PD 3.2 Sec. 8.3.3.19.1.2 / 8.3.3.19.2.1 (PE_DRS_*_Evaluate_Swap):
        DataRoleSwap::Evaluate => {
            if dpm.evaluate_swap_request(SwapType::Data).await {
                DataRoleSwap::Accept
            } else {
                DataRoleSwap::Reject
            }
        }
        // PD 3.2 Sec. 8.3.3.19.1.3 / 8.3.3.19.2.2 (PE_DRS_*_Accept_Swap):
        DataRoleSwap::Accept => {
            protocol_layer
                .transmit_control_message(ControlMessageType::Accept)
                .await?;
            DataRoleSwap::Change
        }
        // PD 3.2 Sec. 8.3.3.19.1.4 / 8.3.3.19.2.3 (PE_DRS_*_Change_*_Swap):
        DataRoleSwap::Change => {
            let new_role = crate::DataRole::from(!bool::from(protocol_layer.header().port_data_role()));
            dpm.swap_data_role(new_role).await;
            return Ok(Step::Done(Exit::Ready));
        }
        // PD 3.2 Sec. 8.3.3.19.1.5 / 8.3.3.19.2.4 (PE_DRS_*_Send_Swap):
        DataRoleSwap::Send => {
            protocol_layer
                .transmit_control_message(ControlMessageType::DrSwap)
                .await?;

            // Per PD 3.2 Sec. 6.6.4.3: after a DR_Swap Wait, only a new DR_Swap is
            // gated (tDRSwapWait). A power Request must not be armed, so no
            // SinkRequestTimer. Re-initiating the DR_Swap is left to the DPM.
            match await_swap_reply(protocol_layer, SwapReply::AcceptRejectWait).await? {
                ControlMessageType::Accept => DataRoleSwap::Change,
                _ => return Ok(Step::Done(Exit::Ready)),
            }
        }
        // PD 3.2 Sec. 8.3.3.19.1.6 / 8.3.3.19.2.5 (PE_DRS_*_Reject_Swap):
        DataRoleSwap::Reject => {
            // Note: A `Wait` response would require a DPM tri-state decision. Not
            // supported yet (see CHANGELOG Known limitations).
            protocol_layer
                .transmit_control_message(ControlMessageType::Reject)
                .await?;
            return Ok(Step::Done(Exit::Ready));
        }
    };

    Ok(Step::Continue(next))
}

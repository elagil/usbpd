//! Shared sender-response await for swap requests.
//!
//! Per PD 3.2 Sec. 6.8.4, a sender of a `Swap` request waits on the
//! `SenderResponseTimer` for an `Accept`, `Reject` or `Wait` reply.
//! The Data Role Swap executor (Sec. 8.3.3.19.1.5) and the Vconn Swap
//! executor (Sec. 8.3.3.20.1) share this exact pattern.

use usbpd_traits::Driver;

use crate::protocol_layer::message::header::{ControlMessageType, MessageType};
use crate::protocol_layer::{ProtocolError, ProtocolLayer};
use crate::timers::{Timer, TimerType};

/// Types accepted as a reply to a swap request.
#[derive(Clone, Copy)]
pub(crate) enum SwapReply {
    /// `Accept`, `Reject` or `Wait` (per PD 3.2 Sec. 6.6.4.3).
    AcceptRejectWait,
    /// `Accept`, `Reject`, `Wait` or `Not_Supported` (Vconn Swap, Sec. 8.3.3.20.1).
    AcceptRejectWaitNotSupported,
}

impl SwapReply {
    fn message_types(self) -> [Option<MessageType>; 4] {
        match self {
            Self::AcceptRejectWait => [
                Some(MessageType::Control(ControlMessageType::Accept)),
                Some(MessageType::Control(ControlMessageType::Reject)),
                Some(MessageType::Control(ControlMessageType::Wait)),
                None,
            ],
            Self::AcceptRejectWaitNotSupported => [
                Some(MessageType::Control(ControlMessageType::Accept)),
                Some(MessageType::Control(ControlMessageType::Reject)),
                Some(MessageType::Control(ControlMessageType::Wait)),
                Some(MessageType::Control(ControlMessageType::NotSupported)),
            ],
        }
    }
}

/// Await the reply to a transmitted swap request.
///
/// Rejects any reply containing a `Wait` when `accepted` is [`SwapReply::AcceptReject`],
/// by mapping it to [`ProtocolError::UnexpectedMessage`].
pub(crate) async fn await_swap_reply<DRIVER: Driver, TIMER: Timer>(
    protocol_layer: &mut ProtocolLayer<DRIVER, TIMER>,
    accepted: SwapReply,
) -> Result<ControlMessageType, ProtocolError> {
    let message_types = &accepted.message_types();
    let expected: heapless::Vec<MessageType, 4> = message_types.iter().filter_map(|ty| *ty).collect();

    let message = protocol_layer
        .receive_message_type(expected.as_slice(), TimerType::SenderResponse)
        .await?;

    match message.header.message_type() {
        MessageType::Control(ControlMessageType::Accept) => Ok(ControlMessageType::Accept),
        MessageType::Control(ControlMessageType::Reject) => Ok(ControlMessageType::Reject),
        MessageType::Control(ControlMessageType::Wait) => Ok(ControlMessageType::Wait),
        MessageType::Control(ControlMessageType::NotSupported) => Ok(ControlMessageType::NotSupported),
        _ => {
            // `receive_message_type` filtered to the control types above.
            unreachable!()
        }
    }
}

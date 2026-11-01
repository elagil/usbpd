//! EPR protocol layer extensions.
//!
//! Transmission and reception of EPR messages, per PD 3.2 Sec. 6.5.15 and 6.4.9.
//! Implemented as `impl` blocks on the protocol layer types to keep the shared
//! files free of EPR logic.

use usbpd_traits::Driver;

use super::message::data::sink_capabilities::SinkCapabilities;
use super::message::data::source_capabilities::SourceCapabilities;
use super::message::data::{Data, epr_mode};
use super::message::extended::extended_control::ExtendedControlMessageType;
use super::message::header::{DataMessageType, ExtendedMessageType, Header};
use super::message::{Message, Payload, extended};
use super::{ProtocolError, ProtocolLayer, SinkProtocolLayer, SourceProtocolLayer, TxError};
use crate::PowerRole;
use crate::timers::Timer;

impl<DRIVER: Driver, TIMER: Timer> ProtocolLayer<DRIVER, TIMER> {
    /// Request EPR source capabilities (EPR_Get_SourceCap extended control message).
    pub async fn transmit_epr_source_cap_request(&mut self) -> Result<(), ProtocolError> {
        self.transmit_extended_control_message(ExtendedControlMessageType::EprGetSourceCap)
            .await
    }

    /// Transmit the device's EPR Source Capabilities
    ///
    /// Could be sent from a Source or Dual Role Device. Also accessible on
    /// [`SourceProtocolLayer`] via deref.
    pub async fn transmit_epr_source_capabilities(
        &mut self,
        source_capabilities: &SourceCapabilities,
    ) -> Result<(), ProtocolError> {
        debug_assert!(matches!(self.default_header.port_power_role(), PowerRole::Source));

        let pdos: heapless::Vec<_, 16> = source_capabilities.0.iter().cloned().collect();
        let extended_payload = extended::Extended::EprSourceCapabilities(pdos);

        let header = Header::new_extended(
            self.default_header,
            self.counters.tx_message,
            ExtendedMessageType::EprSourceCapabilities,
            0, // num_objects is 0 for extended messages
        );

        let mut message = Message::new(header);
        message.payload = Some(Payload::Extended(extended_payload));

        self.transmit(message).await
    }

    /// Transmit an EPR mode data message.
    pub async fn transmit_epr_mode(&mut self, action: epr_mode::Action, data: u8) -> Result<(), ProtocolError> {
        let header = Header::new_data(
            self.default_header,
            self.counters.tx_message,
            DataMessageType::EprMode,
            1,
        );

        let mdo = epr_mode::EprModeDataObject::default()
            .with_action(action)
            .with_data(data);

        self.transmit(Message::new_with_data(header, Data::EprMode(mdo))).await
    }

    /// Transmit EPR sink capabilities in response to EPR_Get_Sink_Cap.
    ///
    /// Per PD 3.2 Sec. 8.3.3.3.10, sinks respond to EPR_Get_Sink_Cap
    /// messages with an EPR_Sink_Capabilities message.
    async fn transmit_epr_sink_capabilities_impl(
        &mut self,
        capabilities: SinkCapabilities,
    ) -> Result<(), ProtocolError> {
        // Convert SinkCapabilities PDOs to the extended message format
        let pdos: heapless::Vec<_, 7> = capabilities.0.iter().cloned().collect();
        let extended_payload = extended::Extended::EprSinkCapabilities(pdos);

        let header = Header::new_extended(
            self.default_header,
            self.counters.tx_message,
            ExtendedMessageType::EprSinkCapabilities,
            0, // num_objects is Reserved (0) for unchunked extended messages per spec 6.2.1.1.2
        );

        let mut message = Message::new(header);
        message.payload = Some(Payload::Extended(extended_payload));

        self.transmit(message).await
    }
}

impl<DRIVER: Driver, TIMER: Timer> SinkProtocolLayer<DRIVER, TIMER> {
    /// Transmit EPR sink capabilities in response to EPR_Get_Sink_Cap.
    ///
    /// Per PD 3.2 Sec. 8.3.3.3.10, sinks respond to EPR_Get_Sink_Cap
    /// messages with an EPR_Sink_Capabilities message.
    pub async fn transmit_epr_sink_capabilities(
        &mut self,
        capabilities: SinkCapabilities,
    ) -> Result<(), ProtocolError> {
        self.0.transmit_epr_sink_capabilities_impl(capabilities).await
    }
}

impl<DRIVER: Driver, TIMER: Timer> SourceProtocolLayer<DRIVER, TIMER> {
    /// Wait for the sink to request a capability with an EPR_Request Message
    pub async fn wait_for_epr_request(&mut self) -> Result<Message, ProtocolError> {
        // Only sources await a sink power request
        debug_assert!(matches!(self.default_header.port_power_role(), PowerRole::Source));

        self.receive_message_type(
            &[super::message::header::MessageType::Data(DataMessageType::EprRequest)],
            crate::timers::TimerType::SenderResponse,
        )
        .await
    }

    /// Transmit EPR sink capabilities (source acting as DRP responds to EPR_Get_Sink_Cap).
    pub async fn transmit_epr_sink_capabilities(
        &mut self,
        capabilities: SinkCapabilities,
    ) -> Result<(), ProtocolError> {
        self.0.transmit_epr_sink_capabilities_impl(capabilities).await
    }
}

/// Validate an outgoing EPR request.
///
/// Checks the raw RDO for spec compliance:
/// - unchunked extended messages bit must not be set,
/// - AVS RDOs (per PD 3.2 Tab. 6.26) require the voltage LSB two bits to be zero.
pub(super) fn validate_epr_request(rdo_bits: u32) -> Result<(), TxError> {
    let unchunked = (rdo_bits >> 23) & 1 == 1;
    if unchunked {
        return Err(TxError::UnchunkedExtendedMessagesNotSupported);
    }

    let voltage = (rdo_bits >> 9) & 0xFFF;
    if (voltage as u16) & 0x3 != 0 {
        return Err(TxError::AvsVoltageAlignmentInvalid);
    }

    Ok(())
}

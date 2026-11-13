//! The protocol layer is controlled by the policy engine, and commands the PHY layer.
//!
//! Handles
//! - construction of messages,
//! - message timers and timeouts,
//! - message retry counters,
//! - extended message chunking and assembly,
//! - reset operation,
//! - error handling,
//! - state behaviour.

#[cfg(feature = "epr")]
pub mod epr;
pub mod message;

use core::future::Future;
use core::marker::PhantomData;

use embassy_futures::select::{Either, select};
use heapless::{Deque, Vec};
use message::data::{Data, request};
#[cfg(any(feature = "epr", test))]
use message::extended::extended_control::ExtendedControlMessageType;
use message::extended::{ExtendedHeader, MAX_EXTENDED_MSG_CHUNK_LEN};
use message::header::{ControlMessageType, DataMessageType, ExtendedMessageType, Header, MessageType};
use message::{MSG_HEADER_SIZE, Message};
use usbpd_traits::{Driver, DriverRxError, DriverTxError};

use crate::PowerRole;
use crate::counters::{Counter, CounterType, Error as CounterError};
use crate::protocol_layer::message::data::source_capabilities::SourceCapabilities;
#[cfg(any(feature = "epr", test))]
use crate::protocol_layer::message::extended::Extended;
use crate::protocol_layer::message::{ParseError, Payload};
use crate::timers::{Timer, TimerType};

/// Maximum message size including headers and payload.
pub(crate) const MAX_MESSAGE_SIZE: usize = 272;

/// Capacity of the pending received-message queue.
///
/// Overflow indicates a misbehaving peer. Per PD 3.2 Sec. 6.12.2.2. If full drop oldest message per PD 3.2 Sec. 6.11.
const PENDING_RX_CAPACITY: usize = 2;

/// Errors that can occur in the protocol layer.
#[derive(thiserror::Error, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ProtocolError {
    /// An error occured during data reception.
    #[error("RX error")]
    RxError(#[from] RxError),
    /// An error occured during data transmission.
    #[error("TX error")]
    TxError(#[from] TxError),
    /// Transmission failed after the maximum number of allowed retries.
    #[error("transmit retries (`{0}`) exceeded")]
    TransmitRetriesExceeded(u8),
    /// An unexpected message was received.
    #[error("unexpected message")]
    UnexpectedMessage,
}

/// Errors that can occur during reception of data.
#[derive(thiserror::Error, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum RxError {
    /// Port partner requested soft reset.
    #[error("soft reset")]
    SoftReset,
    /// Driver reported a hard reset.
    #[error("hard reset")]
    HardReset,
    /// A timeout during message reception.
    #[error("receive timeout")]
    ReceiveTimeout,
    /// An unsupported message was received.
    #[error("unsupported message")]
    UnsupportedMessage,
    /// A message parsing error occured.
    #[error("parse error")]
    ParseError(#[from] ParseError),
    /// The received acknowledgement does not match the last transmitted message's ID.
    #[error("wrong tx id `{0}` acknowledged")]
    AcknowledgeMismatch(u8),
}

/// Errors that can occur during transmission of data.
#[derive(thiserror::Error, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum TxError {
    /// Driver reported a hard reset.
    #[error("hard reset")]
    HardReset,
    /// unchunked_extended_messages_supported must be false (library uses chunked mode).
    #[error("unchunked extended messages not supported")]
    UnchunkedExtendedMessagesNotSupported,
    /// AVS voltage LSB 2 bits must be zero per PD 3.2 Tab. 6.26.
    #[error("AVS voltage alignment invalid")]
    AvsVoltageAlignmentInvalid,
}

#[derive(Debug)]
struct Counters {
    _busy: Counter,
    _caps: Counter, // Unused, optional.
    _discover_identity: Counter,
    rx_message: Option<Counter>,
    tx_message: Counter,
    retry: Counter,
}

impl Default for Counters {
    fn default() -> Self {
        Counters {
            _busy: Counter::new(CounterType::Busy),
            _caps: Counter::new(CounterType::Caps),
            _discover_identity: Counter::new(CounterType::DiscoverIdentity),
            rx_message: None,
            tx_message: Counter::new(CounterType::MessageId),
            retry: Counter::new(CounterType::Retry),
        }
    }
}

/// Queue of messages received while this port was awaiting GoodCRC for its own transmission.
///
/// Per PD 3.2 Sec. 6.12.2.3.1 the receive state machine runs concurrently, so messages are acknowledged and stored for
/// later delivery instead of being dropped.
#[derive(Debug, Default)]
struct PendingRx<const N: usize> {
    // The queue of pending received messages.
    queue: Deque<Message, N>,
}

impl<const N: usize> PendingRx<N> {
    /// Park a message, dropping the oldest one on overflow (per PD 3.2 Sec. 6.11).
    fn push(&mut self, message: Message) {
        if self.queue.is_full() {
            _ = self.queue.pop_front();
            warn!("Pending received-message queue full, dropping oldest message");
        }
        _ = self.queue.push_back(message);
    }

    /// Take the oldest parked message, if any.
    fn pop(&mut self) -> Option<Message> {
        self.queue.pop_front()
    }

    /// Discard all parked messages (per PD 3.2 Sec. 6.12.2.3.2, layer reset for receive).
    fn clear(&mut self) {
        self.queue.clear();
    }
}

/// An oversized extended message being assembled from chunks (RX side).
///
/// Per PD 3.2 Sec. 6.12.2.1.2.4.
#[derive(Debug)]
struct ChunkedRxState {
    /// Assembled chunk payload bytes.
    buffer: Vec<u8, MAX_MESSAGE_SIZE>,
    /// The extended message type being assembled.
    extended_message_type: ExtendedMessageType,
    /// The total payload size once all chunks have arrived.
    data_size: u16,
    /// The chunk number expected next.
    next_chunk: u8,
}

/// An oversized extended message retained after transmission for answering `Chunk Request`s (TX side).
///
/// Per PD 3.2 Sec. 6.12.2.1.2.2.
#[derive(Debug)]
struct ChunkedTxState {
    /// The full transmitted payload.
    buffer: Vec<u8, MAX_MESSAGE_SIZE>,
    /// The extended message type that was transmitted.
    extended_message_type: ExtendedMessageType,
    /// The total payload size.
    data_size: u16,
}

/// The USB PD protocol layer.
///
/// Implements message-level state including message IDs, retries,
/// `GoodCRC` handling (also via parked extended messages) and hard resets.
#[derive(Debug)]
pub(crate) struct ProtocolLayer<DRIVER: Driver, TIMER: Timer> {
    driver: DRIVER,
    counters: Counters,
    default_header: Header,
    extended_rx: Option<ChunkedRxState>,
    pending_rx: PendingRx<PENDING_RX_CAPACITY>,
    /// The most recently transmitted oversized extended payload, for answering Chunk Requests.
    extended_tx: Option<ChunkedTxState>,
    _timer: PhantomData<TIMER>,
}

impl<DRIVER: Driver, TIMER: Timer> ProtocolLayer<DRIVER, TIMER> {
    /// Create a new protocol layer from a driver and default header.
    pub fn new(driver: DRIVER, default_header: Header) -> Self {
        Self {
            driver,
            counters: Default::default(),
            default_header,
            extended_rx: None,
            pending_rx: PendingRx::default(),
            extended_tx: None,
            _timer: PhantomData,
        }
    }

    pub fn deconstruct(self) -> DRIVER {
        self.driver
    }

    /// Reset the protocol layer.
    pub fn reset(&mut self) {
        self.counters = Default::default();
        self.pending_rx.clear();
        // A stale retained extended payload must not survive a layer reset.
        self.reset_chunked_rx();
        self.extended_tx = None;
    }

    /// Allows tests to access the driver directly.
    #[cfg(test)]
    pub fn driver(&mut self) -> &mut DRIVER {
        &mut self.driver
    }

    /// Access the default header directly.
    pub fn header(&self) -> &Header {
        &self.default_header
    }

    fn get_message_buffer() -> [u8; MAX_MESSAGE_SIZE] {
        [0u8; MAX_MESSAGE_SIZE]
    }

    /// Get a timer future for a given type.
    pub fn get_timer(timer_type: TimerType) -> impl Future<Output = ()> {
        TimerType::get_timer::<TIMER>(timer_type)
    }

    /// Receive a simple (non-chunked) message from the driver.
    /// Used by wait_for_good_crc to avoid recursion with chunked message handling.
    async fn receive_simple(&mut self) -> Result<Message, RxError> {
        loop {
            let mut buffer = Self::get_message_buffer();

            let length = match self.driver.receive(&mut buffer).await {
                Ok(length) => length,
                Err(DriverRxError::Discarded) => continue,
                Err(DriverRxError::HardReset) => return Err(RxError::HardReset),
            };

            let message = Message::from_bytes(&buffer[..length])?;
            return Ok(message);
        }
    }

    /// Wait until a GoodCrc message is received, or a timeout occurs.
    ///
    /// Per PD 3.2 Sec. 6.12.2.3.1 the receive state machine runs concurrently with the
    /// transmit state machine, so a port partner may transmit while this port
    /// awaits GoodCRC. Such messages are acknowledged and parked for later
    /// delivery instead of aborting the wait (per PD 3.2 Sec. 6.12.2.2.1.5, Tab. 8.2).
    async fn wait_for_good_crc(&mut self) -> Result<(), RxError> {
        trace!("Wait for GoodCrc");

        let timeout_fut = Self::get_timer(TimerType::CRCReceive);
        let receive_fut = async {
            loop {
                let message = self.receive_simple().await?;

                if matches!(
                    message.header.message_type(),
                    MessageType::Control(ControlMessageType::GoodCRC)
                ) {
                    trace!(
                        "Received GoodCrc, TX message count: {}, expected: {}",
                        message.header.message_id(),
                        self.counters.tx_message.value()
                    );
                    if message.header.message_id() == self.counters.tx_message.value() {
                        // Per PD 3.2 Sec. 6.7.1.1
                        self.counters.retry.reset();
                        _ = self.counters.tx_message.increment();
                        return Ok(());
                    } else {
                        return Err(RxError::AcknowledgeMismatch(message.header.message_id()));
                    }
                } else {
                    // Extended frames that reach here parsed in `receive_simple`
                    // (single-chunk or unchunked). They park alongside control and
                    // data messages for later delivery. True multi-chunk frames
                    // fail parsing in `receive_simple` before reaching this path
                    // (deferred: D-RDv-3, see docs/spec-conformance-report.md).
                    self.park_received_message(message).await?;
                }
            }
        };

        match select(timeout_fut, receive_fut).await {
            Either::First(_) => Err(RxError::ReceiveTimeout),
            Either::Second(receive_result) => receive_result,
        }
    }

    /// Park a regular (not `GoodCRC`) message received while awaiting `GoodCRC` for our own transmission.
    ///
    /// Acknowledges the message (per PD 3.2 Sec. 6.7.1.2), skipping GoodCRC emission for drivers with automatic acknowledgement.
    /// Duplicates are only acknowledged again, not parked twice (per PD 3.2 Sec. 6.7.1.2).
    ///
    /// Per PD 3.2 Sec. 6.2.1.1.5, the sender of a GoodCRC Message Shall set the Specification Revision field to match the
    /// revision of the message being acknowledged. Adopt the received revision before acknowledging.
    async fn park_received_message(&mut self, message: Message) -> Result<(), RxError> {
        if let Ok(revision) = message.header.spec_revision() {
            self.default_header = self.default_header.with_spec_revision(revision);
        }

        match self.handle_rx_ack(&message).await {
            Ok(true) => Ok(()),
            Ok(false) => {
                self.pending_rx.push(message);
                Ok(())
            }
            Err(RxError::HardReset) => Err(RxError::HardReset),
            Err(_) => {
                trace!("Failed to acknowledge parked message");
                Ok(())
            }
        }
    }

    /// Validate an outgoing message for spec compliance.
    ///
    /// This catches common mistakes when constructing messages:
    /// - unchunked_extended_messages_supported should always be false
    /// - AVS voltage LSB 2 bits should be zero (per PD 3.2 Tab. 6.26)
    ///
    /// Only validates outgoing messages - never called when parsing received data.
    /// Returns an error if validation fails, allowing the caller to handle it appropriately.
    fn validate_outgoing_message(message: &Message) -> Result<(), TxError> {
        if let Some(Payload::Data(message::data::Data::Request(power_source))) = &message.payload {
            use message::data::request::PowerSource;
            match power_source {
                PowerSource::FixedVariableSupply(rdo) => {
                    if rdo.unchunked_extended_messages_supported() {
                        return Err(TxError::UnchunkedExtendedMessagesNotSupported);
                    }
                }
                PowerSource::Pps(rdo) =>
                {
                    #[allow(clippy::collapsible_match)]
                    if rdo.unchunked_extended_messages_supported() {
                        return Err(TxError::UnchunkedExtendedMessagesNotSupported);
                    }
                }
                #[cfg(feature = "epr")]
                PowerSource::EprRequest(epr) => epr::validate_epr_request(epr)?,
                _ => {}
            }
        }
        Ok(())
    }

    async fn transmit_inner(&mut self, buffer: &[u8]) -> Result<(), TxError> {
        loop {
            match self.driver.transmit(buffer).await {
                Ok(_) => return Ok(()),
                Err(DriverTxError::HardReset) => return Err(TxError::HardReset),
                Err(DriverTxError::Discarded) => {
                    // Retry transmission.
                }
            }
        }
    }

    /// Transmit a message.
    ///
    // GoodCrc message transmission is handled separately.
    // See `transmit_good_crc()` instead.
    pub async fn transmit(&mut self, message: Message) -> Result<(), ProtocolError> {
        assert_ne!(
            message.header.message_type(),
            MessageType::Control(ControlMessageType::GoodCRC)
        );

        // Validate outgoing message for spec compliance
        Self::validate_outgoing_message(&message)?;

        trace!("Transmit message: {:?}", message);

        let mut buffer = Self::get_message_buffer();
        let size = message.to_bytes(&mut buffer);

        // In the oversized case, `to_bytes` serialized the full payload into `buffer` but
        // truncated the returned size to the first chunk. Retain the full payload so
        // `Chunk Request` frames can be answered (PD 3.2 Sec. 6.6.18.3 / Tab. 8.4).
        if let Some(Payload::Extended(extended)) = message.payload.as_ref()
            && usize::from(extended.data_size()) > MAX_EXTENDED_MSG_CHUNK_LEN
            && let MessageType::Extended(extended_message_type) = message.header.message_type()
        {
            let data_size = extended.data_size();
            let end = MSG_HEADER_SIZE + ExtendedHeader::SIZE + usize::from(data_size);
            // `to_bytes` serialized the full payload into `buffer`, but returned a size
            // truncated to the first chunk. Copy the payload out before the buffer is reused.
            self.extended_tx = Some(ChunkedTxState {
                buffer: Vec::from_slice(&buffer[MSG_HEADER_SIZE + ExtendedHeader::SIZE..end]).unwrap_or_default(),
                extended_message_type,
                data_size,
            });
        }

        if DRIVER::HAS_AUTO_RETRY {
            // Hardware handles retries and verifies GoodCRC reception.
            // Call driver.transmit() directly (not transmit_inner()) because
            // Discarded here means all hardware retries exhausted — no point
            // retrying in software.
            match self.driver.transmit(&buffer[..size]).await {
                Ok(()) => {
                    self.counters.retry.reset();
                    _ = self.counters.tx_message.increment();
                    trace!("Transmit success (hardware retry)");
                    Ok(())
                }
                Err(DriverTxError::HardReset) => Err(TxError::HardReset.into()),
                Err(DriverTxError::Discarded) => {
                    // Per PD 3.2 Sec. 6.12.2.2.1.9 (`PRL_Tx_Transmission_Error`): increment the `MessageIDCounter` and
                    // inform the Policy Engine of the transmission error, so the partner's stored ID is not reused.
                    _ = self.counters.tx_message.increment();
                    Err(ProtocolError::TransmitRetriesExceeded(self.counters.retry.max_value()))
                }
            }
        } else {
            // Software retry loop
            self.counters.retry.reset();

            loop {
                match self.transmit_inner(&buffer[..size]).await {
                    Ok(_) => match self.wait_for_good_crc().await {
                        Ok(()) => {
                            trace!("Transmit success");
                            return Ok(());
                        }
                        Err(RxError::ReceiveTimeout | RxError::AcknowledgeMismatch(_)) => {
                            match self.counters.retry.increment() {
                                Ok(_) => {
                                    // Retry transmission, until the retry counter is exceeded.
                                }
                                Err(CounterError::Exceeded) => {
                                    // Per PD 3.2 Sec. 6.12.2.2.1.9 (PRL_Tx_Transmission_Error): increment the `MessageIDCounter`
                                    // and inform the Policy Engine of the transmission error, so the partner's stored ID is not reused.
                                    let _ = self.counters.tx_message.increment();
                                    return Err(ProtocolError::TransmitRetriesExceeded(
                                        self.counters.retry.max_value(),
                                    ));
                                }
                            }
                        }
                        Err(other) => return Err(other.into()),
                    },
                    Err(other) => return Err(other.into()),
                }
            }
        }
    }

    /// Send a GoodCrc message to the port partner.
    async fn transmit_good_crc(&mut self) -> Result<(), ProtocolError> {
        trace!(
            "Transmit message GoodCrc for RX message count: {}",
            self.counters.rx_message.unwrap().value()
        );

        let mut buffer = Self::get_message_buffer();

        let size = Message::new(Header::new_control(
            self.default_header,
            self.counters.rx_message.unwrap(), // A message must have been received before.
            ControlMessageType::GoodCRC,
        ))
        .to_bytes(&mut buffer);

        Ok(self.transmit_inner(&buffer[..size]).await?)
    }

    /// Handle acknowledgement and retransmission detection for a received message.
    ///
    /// Returns `Ok(true)` if this was a retransmission (caller should continue to next message),
    /// `Ok(false)` if this is a new message to process, or `Err` on failure.
    async fn handle_rx_ack(&mut self, message: &Message) -> Result<bool, RxError> {
        let is_good_crc = matches!(
            message.header.message_type(),
            MessageType::Control(ControlMessageType::GoodCRC)
        );

        let is_retransmission = if is_good_crc {
            false
        } else {
            self.update_rx_message_counter(message)
        };

        if !DRIVER::HAS_AUTO_GOOD_CRC && !is_good_crc {
            match self.transmit_good_crc().await {
                Ok(()) => {}
                Err(ProtocolError::TxError(TxError::HardReset)) => return Err(RxError::HardReset),
                Err(_) => return Err(RxError::UnsupportedMessage),
            }
        }

        Ok(is_retransmission)
    }

    /// Reset chunked message reception state.
    fn reset_chunked_rx(&mut self) {
        self.extended_rx = None;
    }

    /// Receive a message, assembling chunked extended messages as needed.
    async fn receive_message_inner(&mut self) -> Result<Message, RxError> {
        loop {
            // Deliver parked messages (during wait for GoodCRC).
            //
            // They were parsed, acknowledged and counted when parked (per 6.7.1.2), so they skip driver receive,
            // chunk assembly and the acknowledgement pipeline. Only post-parse checks are repeated.
            // Update of spec revision is mirrored here.
            if let Some(message) = self.pending_rx.pop() {
                self.default_header = self.default_header.with_spec_revision(message.header.spec_revision()?);

                match message.header.message_type() {
                    MessageType::Control(ControlMessageType::Reserved)
                    | MessageType::Data(DataMessageType::Reserved) => {
                        trace!("Unsupported message type in header: {:?}", message.header);
                        return Err(RxError::UnsupportedMessage);
                    }
                    MessageType::Control(ControlMessageType::SoftReset) => return Err(RxError::SoftReset),
                    _ => (),
                }

                trace!("Received message {:?}", message);
                return Ok(message);
            }

            let mut buffer = Self::get_message_buffer();

            let length = match self.driver.receive(&mut buffer).await {
                Ok(length) => length,
                Err(DriverRxError::Discarded) => continue,
                Err(DriverRxError::HardReset) => return Err(RxError::HardReset),
            };

            // Parse header early to handle chunking.
            let header = Header::from_bytes(&buffer[..MSG_HEADER_SIZE])?;
            let message_type = header.message_type();

            if matches!(message_type, MessageType::Extended(_)) {
                let ext_header_end = MSG_HEADER_SIZE + ExtendedHeader::SIZE;
                let ext_header =
                    message::extended::ExtendedHeader::from_bytes(&buffer[MSG_HEADER_SIZE..ext_header_end]);
                let payload = &buffer[ext_header_end..length];
                let total_size = ext_header.data_size();
                let chunked = ext_header.chunked();
                let chunk_number = ext_header.chunk_number();
                let msg_type = match message_type {
                    MessageType::Extended(mt) => mt,
                    _ => unreachable!(),
                };

                // Update specification revision, based on the received frame.
                self.default_header = self.default_header.with_spec_revision(header.spec_revision()?);

                // Answer the Chunk Request from the retained payload (PD 3.2 Sec. 6.12.2.1.2.2).
                if chunked && ext_header.request_chunk() {
                    // Acknowledge first. A duplicate request (same MessageID) gets a
                    // re-ack, but the chunk is served anyway: the re-served response is
                    // the correct retry for a lost response, and it is idempotent.
                    let tmp_message = Message { header, payload: None };
                    self.handle_rx_ack(&tmp_message).await?;

                    let (data_size, chunk): (u16, Vec<u8, MAX_MESSAGE_SIZE>) = match self.extended_tx.as_ref() {
                        Some(tx_state) if tx_state.extended_message_type == msg_type => {
                            let start = chunk_number as usize * MAX_EXTENDED_MSG_CHUNK_LEN;
                            // Requested chunk beyond the retained payload: do not answer.
                            // The partner's chunk timer runs its own error flow. Never
                            // index: start can exceed the buffer for hostile requests.
                            if start >= usize::from(tx_state.data_size) {
                                continue;
                            }
                            let end =
                                core::cmp::min(start + MAX_EXTENDED_MSG_CHUNK_LEN, usize::from(tx_state.data_size));
                            (
                                tx_state.data_size,
                                Vec::from_slice(&tx_state.buffer[start..end]).unwrap_or_default(),
                            )
                        }
                        _ => {
                            // No matching retained payload: report an error to the PE.
                            return Err(RxError::UnsupportedMessage);
                        }
                    };

                    if !chunk.is_empty() {
                        self.transmit_chunk_response(msg_type, chunk_number, data_size, &chunk)
                            .await?;
                    }

                    continue;
                }

                if chunked {
                    trace!(
                        "Received chunked extended message {:?}, chunk {}, size {}",
                        message_type,
                        chunk_number,
                        payload.len()
                    );

                    // Update RX counters and acknowledge.
                    let tmp_message = Message { header, payload: None };
                    if self.handle_rx_ack(&tmp_message).await? {
                        continue; // Retransmission
                    }

                    // A fresh assembly starts when no matching assembly is in progress.
                    if !matches!(self.extended_rx.as_ref(), Some(state) if state.extended_message_type == msg_type) {
                        self.extended_rx = Some(ChunkedRxState {
                            buffer: Vec::new(),
                            extended_message_type: msg_type,
                            data_size: total_size,
                            next_chunk: 0,
                        });
                    }

                    let state = self.extended_rx.as_mut().unwrap();

                    // Ensure chunks arrive in order.
                    if state.next_chunk != chunk_number {
                        self.reset_chunked_rx();
                        return Err(RxError::UnsupportedMessage);
                    }

                    state.next_chunk += 1;

                    if state.buffer.extend_from_slice(payload).is_err() {
                        self.extended_rx = None;
                        return Err(RxError::UnsupportedMessage);
                    }

                    if state.buffer.len() < usize::from(state.data_size) {
                        // Need more chunks - send chunk request per PD 3.2 Sec. 6.12.2.1.2.4
                        let next_chunk = state.next_chunk;
                        self.transmit_chunk_request(msg_type, next_chunk).await?;
                        continue;
                    }

                    // All chunks received, parse payload.
                    let data_size = state.data_size;
                    let ext_payload: Vec<u8, MAX_MESSAGE_SIZE> =
                        Vec::from_slice(&state.buffer[..usize::from(data_size)]).unwrap_or_default();

                    self.extended_rx = None;
                    let mut message = Message::new(header);
                    message.payload = Some(Payload::Extended(Message::parse_extended_payload(
                        msg_type,
                        &ext_payload,
                    )));

                    trace!("Received assembled extended message {:?}", message);
                    return Ok(message);
                }
            }

            // Non-extended or unchunked extended messages.
            let message = Message::from_bytes(&buffer[..length])?;

            // Update specification revision, based on the received frame.
            self.default_header = self.default_header.with_spec_revision(message.header.spec_revision()?);

            match message.header.message_type() {
                MessageType::Control(ControlMessageType::Reserved) | MessageType::Data(DataMessageType::Reserved) => {
                    trace!("Unsupported message type in header: {:?}", message.header);
                    return Err(RxError::UnsupportedMessage);
                }
                MessageType::Control(ControlMessageType::SoftReset) => {
                    // Per PD 3.2 Sec. 6.12.2.3.2/.3: reset the `MessageIDCounter` and clear the stored `MessageID` in
                    // `PRL_Rx_Layer_Reset_for_Receive`, then send `GoodCRC` from `PRL_RxSend_GoodCRC`
                    // once reset actions completed.
                    //
                    // Per PD 3.2 Sec. 6.8.1, the receiver resets its `MessageIDCounter` and `RetryCounter`
                    // before the `Accept` response.
                    self.reset();
                    match self.handle_rx_ack(&message).await? {
                        true => continue, // Retransmission of the `Soft_Reset`
                        false => return Err(RxError::SoftReset),
                    }
                }
                _ => (),
            }

            // Handle GoodCRC and retransmissions.
            if self.handle_rx_ack(&message).await? {
                continue; // Retransmission
            }

            trace!("Received message {:?}", message);
            return Ok(message);
        }
    }

    /// Receive a message.
    ///
    /// Waits until a valid message arrives or the receive timeout expires.
    pub async fn receive_message(&mut self) -> Result<Message, ProtocolError> {
        self.receive_message_inner().await.map_err(|err| err.into())
    }

    /// Updates the received message counter.
    ///
    /// If receiving the first message after protocol layer reset, copy its ID.
    /// Otherwise, compare the received ID with the stored ID. If they are equal, this is a retransmission.
    ///
    /// Returns `true`, if this was a retransmission.
    fn update_rx_message_counter(&mut self, rx_message: &Message) -> bool {
        match self.counters.rx_message.as_mut() {
            None => {
                trace!(
                    "Received first message after protocol layer reset with RX counter value: {}",
                    rx_message.header.message_id()
                );
                self.counters.rx_message = Some(Counter::new_from_value(
                    CounterType::MessageId,
                    rx_message.header.message_id(),
                ));
                false
            }
            Some(counter) => {
                if rx_message.header.message_id() == counter.value() {
                    trace!("Received retransmission of RX counter value: {}", counter.value());
                    true
                } else {
                    counter.set(rx_message.header.message_id());
                    false
                }
            }
        }
    }

    /// Wait until a message of one of the chosen types is received, or a timeout occurs.
    pub async fn receive_message_type(
        &mut self,
        message_types: &[MessageType],
        timer_type: TimerType,
    ) -> Result<Message, ProtocolError> {
        // GoodCrc message reception is handled separately.
        // See `wait_for_good_crc()` instead.
        for message_type in message_types {
            assert_ne!(*message_type, MessageType::Control(ControlMessageType::GoodCRC));
        }

        let timeout_fut = Self::get_timer(timer_type);
        let receive_fut = async {
            loop {
                match self.receive_message_inner().await {
                    Ok(message) => {
                        if matches!(
                            message.header.message_type(),
                            MessageType::Control(ControlMessageType::GoodCRC)
                        ) {
                            continue;
                        }
                        return if message_types.contains(&message.header.message_type()) {
                            Ok(message)
                        } else {
                            Err(ProtocolError::UnexpectedMessage)
                        };
                    }
                    Err(RxError::ParseError(err)) => {
                        // Per PD 3.2 Sec. 6.6.1, a corrupt header is dropped.
                        // The sender retries after its `CRCReceiveTimer` expiry.
                        trace!("Dropping frame with parse error: {:?}", err);
                        continue;
                    }
                    Err(other) => return Err(other.into()),
                }
            }
        };

        match select(timeout_fut, receive_fut).await {
            Either::First(_) => Err(RxError::ReceiveTimeout.into()),
            Either::Second(receive_result) => receive_result,
        }
    }

    /// Perform a hard-reset procedure.
    ///
    // Per PD 3.2 Sec. 6.7.1.1
    pub async fn hard_reset(&mut self) -> Result<(), ProtocolError> {
        // Per PD 3.2 Sec. 6.12.2.3.2, pending receive state is cleared on layer reset.
        self.pending_rx.clear();
        // Per PD 3.2 Sec. 6.12.2.4.1 / Tab. 8.58: the `MessageIDCounter`, the stored copy of the `MessageID` and
        // the `RetryCounter` are all reset.
        self.reset();
        self.counters.retry.reset();
        // Stale chunked-assembly state must not survive a layer reset.
        self.reset_chunked_rx();
        // Bound the retry loop by tHardResetComplete; a wedged driver must not stall the PE.
        let timeout_fut = Self::get_timer(TimerType::HardResetComplete);

        let transmit_fut = async {
            loop {
                match self.driver.transmit_hard_reset().await {
                    Ok(_) | Err(DriverTxError::HardReset) => break Ok::<(), DriverTxError>(()),
                    // Mid-collision: retry, yielding so the timeout future can progress.
                    Err(DriverTxError::Discarded) => embassy_futures::yield_now().await,
                }
            }
        };

        match select(timeout_fut, transmit_fut).await {
            Either::First(_) => Err(ProtocolError::TxError(TxError::HardReset)),
            Either::Second(result) => {
                trace!("Performed hard reset");
                result.map_err(|_| ProtocolError::UnexpectedMessage)
            }
        }
    }

    /// Wait for VBUS to be available.
    pub async fn wait_for_vbus(&mut self) {
        self.driver.wait_for_vbus().await
    }

    /// Transmit a control message of the provided type.
    pub async fn transmit_control_message(&mut self, message_type: ControlMessageType) -> Result<(), ProtocolError> {
        let message = Message::new(Header::new_control(
            self.default_header,
            self.counters.tx_message,
            message_type,
        ));

        self.transmit(message).await
    }

    /// Transmit an extended control message of the provided type.
    #[cfg(any(feature = "epr", test))]
    pub async fn transmit_extended_control_message(
        &mut self,
        message_type: ExtendedControlMessageType,
    ) -> Result<(), ProtocolError> {
        // Per PD 3.2 Sec. 6.2.1.1.2: for extended messages, num_objects must be non-zero.
        // ExtendedControl = 2-byte extended header + 2-byte data = 4 bytes = 1 data object.
        let mut message = Message::new(Header::new_extended(
            self.default_header,
            self.counters.tx_message,
            ExtendedMessageType::ExtendedControl,
            1,
        ));

        message.payload = Some(Payload::Extended(Extended::ExtendedControl(
            message::extended::extended_control::ExtendedControl::default().with_message_type(message_type),
        )));

        self.transmit(message).await
    }

    /// Answer a `Chunk Request` with the requested chunk of the retained extended
    /// payload (PD 3.2 Sec. 6.12.2.1.2.2).
    async fn transmit_chunk_response(
        &mut self,
        message_type: ExtendedMessageType,
        chunk_number: u8,
        data_size: u16,
        chunk_data: &[u8],
    ) -> Result<(), RxError> {
        trace!("Serving chunk request for {:?} chunk {}", message_type, chunk_number);

        let ext_header = message::extended::ExtendedHeader::new(data_size)
            .with_chunked(true)
            .with_chunk_number(chunk_number);

        // Per PD 3.2 Sec. 6.2.1.1.2: the object count covers the extended header plus
        // the chunk data, padded to a 4-byte boundary (see also Fig. 6.9).
        let num_objects = (message::extended::ExtendedHeader::SIZE + chunk_data.len()).div_ceil(4) as u8;
        let header = Header::new_extended(self.default_header, self.counters.tx_message, message_type, num_objects);

        let mut buffer = Self::get_message_buffer();
        let mut offset = header.to_bytes(&mut buffer);
        offset += ext_header.to_bytes(&mut buffer[offset..]);
        buffer[offset..offset + chunk_data.len()].copy_from_slice(chunk_data);
        offset += chunk_data.len();

        if DRIVER::HAS_AUTO_RETRY {
            match self.driver.transmit(&buffer[..offset]).await {
                Ok(()) => {
                    self.counters.retry.reset();
                    _ = self.counters.tx_message.increment();
                    Ok(())
                }
                Err(DriverTxError::HardReset) => Err(RxError::HardReset),
                Err(DriverTxError::Discarded) => Err(RxError::ReceiveTimeout),
            }
        } else {
            match self.transmit_inner(&buffer[..offset]).await {
                Ok(_) => self.wait_for_good_crc().await,
                Err(TxError::HardReset) => Err(RxError::HardReset),
                Err(_) => Err(RxError::ReceiveTimeout),
            }
        }
    }

    /// Transmit a chunk request message per PD 3.2 Sec. 6.12.2.1.2.4.
    ///
    /// A chunk request is an extended message with:
    /// - The same message type as the chunked message being received
    /// - Extended header with: chunked=1, request_chunk=1, chunk_number=requested_chunk, data_size=0
    async fn transmit_chunk_request(
        &mut self,
        message_type: ExtendedMessageType,
        chunk_number: u8,
    ) -> Result<(), RxError> {
        trace!("Transmit chunk request for {:?} chunk {}", message_type, chunk_number);

        // Build extended header for chunk request
        let ext_header = message::extended::ExtendedHeader::default()
            .with_chunked(true)
            .with_request_chunk(true)
            .with_chunk_number(chunk_number);

        // Build message header - num_objects = 1 for the extended header word
        let header = Header::new_extended(self.default_header, self.counters.tx_message, message_type, 1);

        // Build message bytes manually
        let mut buffer = Self::get_message_buffer();
        let mut offset = header.to_bytes(&mut buffer);
        offset += ext_header.to_bytes(&mut buffer[offset..]);
        // Pad to 4-byte Data Object boundary per PD 3.2.
        // Extended header is 2 bytes, so add 2 bytes padding to complete the Data Object.
        // Buffer is already zeroed, so just advance offset.
        offset += 2;

        // Transmit and wait for GoodCRC
        if DRIVER::HAS_AUTO_RETRY {
            match self.driver.transmit(&buffer[..offset]).await {
                Ok(()) => {
                    self.counters.retry.reset();
                    _ = self.counters.tx_message.increment();
                    Ok(())
                }
                Err(DriverTxError::HardReset) => Err(RxError::HardReset),
                Err(DriverTxError::Discarded) => Err(RxError::ReceiveTimeout),
            }
        } else {
            match self.transmit_inner(&buffer[..offset]).await {
                Ok(_) => self.wait_for_good_crc().await,
                Err(TxError::HardReset) => Err(RxError::HardReset),
                Err(TxError::UnchunkedExtendedMessagesNotSupported | TxError::AvsVoltageAlignmentInvalid) => {
                    unreachable!("validation should happen before transmit_inner")
                }
            }
        }
    }

    /// Transmit sink capabilities in response to Get_Sink_Cap.
    ///
    /// Per PD 3.2 Sec. 6.4.1.6, sinks respond to Get_Sink_Cap messages
    /// with a Sink_Capabilities message containing PDOs describing what power levels
    /// the sink can operate at.
    pub async fn transmit_sink_capabilities(
        &mut self,
        capabilities: message::data::sink_capabilities::SinkCapabilities,
    ) -> Result<(), ProtocolError> {
        let num_objects = capabilities.num_objects();
        let header = Header::new_data(
            self.default_header,
            self.counters.tx_message,
            DataMessageType::SinkCapabilities,
            num_objects,
        );

        self.transmit(Message::new_with_data(header, Data::SinkCapabilities(capabilities)))
            .await
    }

    /// Transmit the device's Source Capabilities
    ///
    /// Could be sent from a Source or Dual Role Device
    pub async fn transmit_source_capabilities(
        &mut self,
        source_capabilities: &SourceCapabilities,
    ) -> Result<(), ProtocolError> {
        // Only sources can send capabilities
        debug_assert!(matches!(self.default_header.port_power_role(), PowerRole::Source));
        // Per PD 3.2 Sec. 8.3.3.3.8: reject SPR capabilities containing EPR PDOs
        #[cfg(feature = "epr")]
        if source_capabilities.has_epr_pdo_in_spr_positions() {
            return Err(ProtocolError::TxError(TxError::HardReset));
        }

        let header = Header::new_data(
            self.default_header,
            self.counters.tx_message,
            DataMessageType::SourceCapabilities,
            source_capabilities.0.len() as u8, // Raw cast OK since since len has domain of [0, 8]
        );

        let message = Message::new_with_data(header, Data::SourceCapabilities(source_capabilities.clone()));

        self.transmit(message).await
    }
}

#[repr(transparent)]
#[derive(Debug)]
/// The USB PD Protocol Layer for a `Sink`
pub(crate) struct SinkProtocolLayer<DRIVER: Driver, TIMER: Timer>(pub(crate) ProtocolLayer<DRIVER, TIMER>);

impl<DRIVER: Driver, TIMER: Timer> core::ops::Deref for SinkProtocolLayer<DRIVER, TIMER> {
    type Target = ProtocolLayer<DRIVER, TIMER>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<DRIVER: Driver, TIMER: Timer> core::ops::DerefMut for SinkProtocolLayer<DRIVER, TIMER> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<DRIVER: Driver, TIMER: Timer> SinkProtocolLayer<DRIVER, TIMER> {
    /// Create a new protocol layer from a driver and default header.
    pub fn new(driver: DRIVER, default_header: Header) -> Self {
        Self(ProtocolLayer::new(driver, default_header))
    }

    pub fn deconstruct(self) -> DRIVER {
        self.0.deconstruct()
    }

    /// Wait for the source to provide its capabilities.
    pub async fn wait_for_source_capabilities(&mut self) -> Result<Message, ProtocolError> {
        // Only sinks can await capabilities.
        debug_assert!(matches!(self.default_header.port_power_role(), PowerRole::Sink));

        self.receive_message_type(
            &[
                MessageType::Data(message::header::DataMessageType::SourceCapabilities),
                #[cfg(feature = "epr")]
                MessageType::Extended(ExtendedMessageType::EprSourceCapabilities),
            ],
            TimerType::SinkWaitCap,
        )
        .await
    }

    /// Request a certain power level from the source.
    pub async fn request_power(&mut self, power_source_request: request::PowerSource) -> Result<(), ProtocolError> {
        // Only sinks can request from a supply.
        debug_assert!(matches!(self.default_header.port_power_role(), PowerRole::Sink));

        let message_type = power_source_request.message_type();
        let num_objects = power_source_request.num_objects();
        let header = Header::new_data(self.default_header, self.counters.tx_message, message_type, num_objects);

        self.transmit(Message::new_with_data(header, Data::Request(power_source_request)))
            .await
    }
}

#[repr(transparent)]
#[derive(Debug)]
/// The USB PD Protocol Layer for a `Source`
pub(crate) struct SourceProtocolLayer<DRIVER: Driver, TIMER: Timer>(pub(crate) ProtocolLayer<DRIVER, TIMER>);

impl<DRIVER: Driver, TIMER: Timer> core::ops::Deref for SourceProtocolLayer<DRIVER, TIMER> {
    type Target = ProtocolLayer<DRIVER, TIMER>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<DRIVER: Driver, TIMER: Timer> core::ops::DerefMut for SourceProtocolLayer<DRIVER, TIMER> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<DRIVER: Driver, TIMER: Timer> SourceProtocolLayer<DRIVER, TIMER> {
    /// Create a new protocol layer from a driver and default header.
    pub fn new(driver: DRIVER, default_header: Header) -> Self {
        Self(ProtocolLayer::new(driver, default_header))
    }

    pub fn deconstruct(self) -> DRIVER {
        self.0.deconstruct()
    }

    /// Wait for the sink to request a capability with a Request Message.
    pub async fn wait_for_request(&mut self) -> Result<Message, ProtocolError> {
        // Only sources await a sink power request
        debug_assert!(matches!(self.default_header.port_power_role(), PowerRole::Source));

        self.receive_message_type(
            &[MessageType::Data(message::header::DataMessageType::Request)],
            TimerType::SenderResponse,
        )
        .await
    }
}

#[cfg(test)]
mod tests {

    use core::iter::zip;

    use super::message::data::Data;
    use super::message::data::source_capabilities::SourceCapabilities;
    use super::message::header::Header;
    use super::{MAX_EXTENDED_MSG_CHUNK_LEN, ProtocolLayer};
    use crate::dummy::{
        DUMMY_CAPABILITIES, DummyDriver, DummyTimer, MAX_DATA_MESSAGE_SIZE, get_dummy_source_capabilities,
    };
    use crate::protocol_layer::message::Payload;

    fn get_protocol_layer(
        driver: DummyDriver<MAX_DATA_MESSAGE_SIZE>,
    ) -> ProtocolLayer<DummyDriver<MAX_DATA_MESSAGE_SIZE>, DummyTimer> {
        ProtocolLayer::new(
            driver,
            Header::new_template(
                crate::DataRole::Ufp,
                crate::PowerRole::Sink,
                super::message::header::SpecificationRevision::R3_X,
            ),
        )
    }

    /// Protocol layer with a Source/DFP default header (for source-only transmissions).
    fn get_protocol_layer_with_source_role() -> ProtocolLayer<DummyDriver<{ super::MAX_MESSAGE_SIZE }>, DummyTimer> {
        ProtocolLayer::new(
            DummyDriver::new(),
            Header::new_template(
                crate::DataRole::Dfp,
                crate::PowerRole::Source,
                super::message::header::SpecificationRevision::R3_X,
            ),
        )
    }

    #[tokio::test]
    async fn test_it() {
        let mut protocol_layer = get_protocol_layer(DummyDriver::new());

        protocol_layer.driver.inject_received_data(&DUMMY_CAPABILITIES);
        let message = protocol_layer.receive_message().await.unwrap();

        if let Some(Payload::Data(Data::SourceCapabilities(SourceCapabilities(caps)))) = message.payload {
            for (cap, dummy_cap) in zip(caps, get_dummy_source_capabilities()) {
                assert_eq!(cap, dummy_cap);
            }
        } else {
            panic!()
        }
    }

    /// Build a `GoodCRC` frame acknowledging `message_id` (per PD 3.2 Sec. 6.3.1).
    fn good_crc_bytes(message_id: u8) -> [u8; 2] {
        // Header: Control `GoodCRC` (raw type 1), num_objects = 0, spec rev 3.x,
        // power role sink (0), data role UFP (0). Little-endian on the wire.
        let header = 0x0001u16 | ((message_id as u16) << 9) | (0b10 << 6);
        [(header & 0xFF) as u8, (header >> 8) as u8]
    }

    /// Build a `Source_Capabilities` frame with the fixed 5 V / 3 A PDO.
    fn source_caps_bytes(message_id: u8) -> [u8; 6] {
        // Header: Data `Source_Capabilities` (raw type 1), num_objects = 1,
        // spec rev 3.x, power role sink (0), data role UFP (0), followed by one
        // PDO from DUMMY_CAPABILITIES. Little-endian on the wire.
        let header = 0x1000u16 | ((message_id as u16) << 9) | (0b10 << 6) | 0b0_0001;
        let mut buffer = [0u8; 6];
        buffer[0] = (header & 0xFF) as u8;
        buffer[1] = (header >> 8) as u8;
        buffer[2..6].copy_from_slice(&DUMMY_CAPABILITIES[2..6]);
        buffer
    }

    fn transmit_get_source_cap_message() -> super::message::Message {
        super::message::Message::new(Header::new_control(
            Header::new_template(
                crate::DataRole::Ufp,
                crate::PowerRole::Sink,
                super::message::header::SpecificationRevision::R3_X,
            ),
            crate::counters::Counter::new(crate::counters::CounterType::MessageId),
            super::message::header::ControlMessageType::GetSourceCap,
        ))
    }

    /// Build a serialized single-chunk Extended_Control (`EPR_KeepAlive_Ack`) frame from the partner,
    /// exercising the extended-message parking path during a `GoodCRC` wait.
    fn extended_control_bytes(message_id: u8) -> [u8; 8] {
        use byteorder::{ByteOrder, LittleEndian};

        use crate::protocol_layer::message::extended::ExtendedHeader;
        use crate::protocol_layer::message::extended::extended_control::{ExtendedControl, ExtendedControlMessageType};

        // Header: Extended Extended_Control (raw type 16), spec rev 3.x.
        let header = Header::new_extended(
            Header::new_template(
                crate::DataRole::Dfp,
                crate::PowerRole::Source,
                super::message::header::SpecificationRevision::R3_X,
            ),
            crate::counters::Counter::new_from_value(crate::counters::CounterType::MessageId, message_id),
            super::message::header::ExtendedMessageType::ExtendedControl,
            1,
        );

        // Extended header: data size 2, single chunk 0 (fits in one chunk).
        let ext_header = ExtendedHeader::new(2).with_chunked(true).with_chunk_number(0);
        let control = ExtendedControl::default().with_message_type(ExtendedControlMessageType::EprKeepAliveAck);

        let mut buffer = [0u8; 8];
        let mut offset = header.to_bytes(&mut buffer);
        offset += ext_header.to_bytes(&mut buffer[offset..]);
        LittleEndian::write_u16(&mut buffer[offset..], control.0);
        buffer
    }

    #[tokio::test]
    async fn test_non_goodcrc_during_goodcrc_wait_is_parked_and_acked() {
        let mut protocol_layer = get_protocol_layer(DummyDriver::new());

        protocol_layer.driver.inject_received_data(&source_caps_bytes(7));
        protocol_layer.driver.inject_received_data(&good_crc_bytes(0));

        let result = protocol_layer.transmit(transmit_get_source_cap_message()).await;
        result.unwrap();

        // The only transmitted frames are our own Get_Source_Cap and the
        // GoodCRC acking the partner's Source_Capabilities; no retransmission.
        let own = protocol_layer.driver.probe_transmitted_data();
        let own = super::message::Message::from_bytes(&own).unwrap();
        assert_eq!(
            own.header.message_type(),
            super::message::header::MessageType::Control(super::message::header::ControlMessageType::GetSourceCap)
        );
        let good_crc = protocol_layer.driver.probe_transmitted_data();
        let ack = super::message::Message::from_bytes(&good_crc).unwrap();
        assert_eq!(
            ack.header.message_type(),
            super::message::header::MessageType::Control(super::message::header::ControlMessageType::GoodCRC)
        );
        assert_eq!(ack.header.message_id(), 7);
        assert!(!protocol_layer.driver.has_transmitted_data());

        // The parked Source_Capabilities is delivered intact afterwards.
        let message = protocol_layer.receive_message().await.unwrap();
        assert_eq!(
            message.header.message_type(),
            super::message::header::MessageType::Data(super::message::header::DataMessageType::SourceCapabilities)
        );
        assert_eq!(message.header.message_id(), 7);
        assert!(matches!(
            message.payload,
            Some(Payload::Data(Data::SourceCapabilities(_)))
        ));
    }

    #[tokio::test]
    async fn test_extended_message_during_goodcrc_wait_is_parked_and_acked() {
        let mut protocol_layer = get_protocol_layer(DummyDriver::new());

        protocol_layer.driver.inject_received_data(&extended_control_bytes(3));
        protocol_layer.driver.inject_received_data(&good_crc_bytes(0));

        let result = protocol_layer.transmit(transmit_get_source_cap_message()).await;
        result.unwrap();

        // Transmitted frames: our own Get_Source_Cap, then the GoodCRC
        // acknowledging the partner's Extended_Control; no retransmission.
        let own = protocol_layer.driver.probe_transmitted_data();
        assert_eq!(
            super::message::Message::from_bytes(&own).unwrap().header.message_type(),
            super::message::header::MessageType::Control(super::message::header::ControlMessageType::GetSourceCap)
        );
        let good_crc = protocol_layer.driver.probe_transmitted_data();
        let ack = super::message::Message::from_bytes(&good_crc).unwrap();
        assert_eq!(
            ack.header.message_type(),
            super::message::header::MessageType::Control(super::message::header::ControlMessageType::GoodCRC)
        );
        assert_eq!(ack.header.message_id(), 3);
        assert!(!protocol_layer.driver.has_transmitted_data());

        // The parked Extended_Control is delivered intact afterwards.
        let message = protocol_layer.receive_message().await.unwrap();
        assert_eq!(message.header.message_id(), 3);
        assert_eq!(
            message.header.message_type(),
            super::message::header::MessageType::Extended(super::message::header::ExtendedMessageType::ExtendedControl)
        );
        assert!(matches!(
            message.payload,
            Some(super::message::Payload::Extended(
                super::message::extended::Extended::ExtendedControl(_)
            ))
        ));
    }

    #[tokio::test]
    async fn test_mismatched_goodcrc_retries() {
        let mut protocol_layer = get_protocol_layer(DummyDriver::new());

        // Wrong acknowledgment followed by the correct one (original message id 0).
        protocol_layer.driver.inject_received_data(&good_crc_bytes(5));
        protocol_layer.driver.inject_received_data(&good_crc_bytes(0));

        let result = protocol_layer.transmit(transmit_get_source_cap_message()).await;
        result.unwrap();

        // The original frame and one retry share the same MessageID.
        let first = protocol_layer.driver.probe_transmitted_data();
        let second = protocol_layer.driver.probe_transmitted_data();
        assert_eq!(first, second);
        let message = super::message::Message::from_bytes(&first).unwrap();
        assert_eq!(message.header.message_id(), 0);
        assert!(!protocol_layer.driver.has_transmitted_data());
    }

    #[tokio::test]
    async fn test_two_messages_parked_in_order() {
        let mut protocol_layer = get_protocol_layer(DummyDriver::new());

        protocol_layer.driver.inject_received_data(&source_caps_bytes(0));
        protocol_layer.driver.inject_received_data(&source_caps_bytes(1));
        protocol_layer.driver.inject_received_data(&good_crc_bytes(0));

        let result = protocol_layer.transmit(transmit_get_source_cap_message()).await;
        result.unwrap();

        // Transmitted frames: our own Get_Source_Cap, then one GoodCRC per
        // partner message, in arrival order.
        let own = protocol_layer.driver.probe_transmitted_data();
        assert_eq!(
            super::message::Message::from_bytes(&own).unwrap().header.message_type(),
            super::message::header::MessageType::Control(super::message::header::ControlMessageType::GetSourceCap)
        );
        let good_crc = protocol_layer.driver.probe_transmitted_data();
        assert_eq!(
            super::message::Message::from_bytes(&good_crc)
                .unwrap()
                .header
                .message_id(),
            0
        );
        let good_crc = protocol_layer.driver.probe_transmitted_data();
        assert_eq!(
            super::message::Message::from_bytes(&good_crc)
                .unwrap()
                .header
                .message_id(),
            1
        );
        assert!(!protocol_layer.driver.has_transmitted_data());

        // Both parked messages are delivered in order with matching ids.
        let first = protocol_layer.receive_message().await.unwrap();
        let second = protocol_layer.receive_message().await.unwrap();
        assert_eq!(first.header.message_id(), 0);
        assert_eq!(second.header.message_id(), 1);
    }

    #[test]
    fn test_pending_rx_overflow_drops_oldest() {
        let mut pending = super::PendingRx::<{ super::PENDING_RX_CAPACITY }>::default();

        // Push messages with MessageIDs 1, 2, 3; capacity is 2, so the second
        // push overflows and drops the MessageID 1 message.
        for message_id in 1..4u16 {
            let header = super::message::header::Header((message_id << 9) | 0x0001);
            pending.push(super::message::Message::new(header));
        }

        assert_eq!(2, pending.queue.len());
        assert_eq!(Some(2), pending.pop().map(|message| message.header.message_id()));
        assert_eq!(Some(3), pending.pop().map(|message| message.header.message_id()));
        assert!(pending.queue.is_empty());
    }

    // ==== Chunked extended message transmission (PD 3.2 Sec. 6.2.1.2.5.2) ====

    /// A >26-byte extended payload must be transmitted as a correct partial chunk-0
    /// frame (26 payload bytes), not the whole payload (the pre-fix bug).
    #[tokio::test]
    async fn test_oversized_extended_tx_is_chunked() {
        let mut protocol_layer = get_protocol_layer_with_source_role();

        #[cfg(feature = "epr")]
        {
            use super::message::header::{ExtendedMessageType, MessageType};
            use crate::protocol_layer::message::data::source_capabilities::{PowerDataObject, parse_raw_pdo};

            let pdos: heapless::Vec<PowerDataObject, 16> = (0..15)
                .map(|i| {
                    parse_raw_pdo(
                        // EPR AVS APDO raw: supply type 01b @ 30..31, max voltage 17..25, min 8..15.
                        (0b01 << 30) | (1000 + i as u32 * 2) | ((400 + i as u32) << 8) | (1500 << 17),
                    )
                })
                .collect();
            assert_eq!(pdos.len(), 15);

            let caps = SourceCapabilities(pdos);
            // Pre-inject the GoodCRC acknowledging chunk 0 (tx id 0).
            protocol_layer.driver.inject_received_data(&good_crc_bytes(0));
            protocol_layer.transmit_epr_source_capabilities(&caps).await.unwrap();

            let total_payload_len = caps.0.len() * 4;
            let mut chunk_numbers = heapless::Vec::<u8, 8>::new();
            while protocol_layer.driver.has_transmitted_data() {
                let bytes = protocol_layer.driver.probe_transmitted_data();
                // Per-chunk frames require assembly, so parse only header + extended header here.
                assert!(bytes.len() >= super::MSG_HEADER_SIZE + super::message::extended::ExtendedHeader::SIZE);
                let frame_header = Header::from_bytes(&bytes[..super::MSG_HEADER_SIZE]).unwrap();
                let ext_header = super::message::extended::ExtendedHeader::from_bytes(
                    &bytes[super::MSG_HEADER_SIZE
                        ..super::MSG_HEADER_SIZE + super::message::extended::ExtendedHeader::SIZE],
                );
                assert!(matches!(
                    frame_header.message_type(),
                    MessageType::Extended(ExtendedMessageType::EprSourceCapabilities)
                ));
                assert!(ext_header.chunked());
                assert_eq!(usize::from(ext_header.chunk_number()), chunk_numbers.len());
                assert!((ext_header.data_size() as usize) == total_payload_len);
                // Per PD 3.2 Sec. 6.2.1.1.2 / Fig. 6.9: object count covers the extended
                // header plus the chunk data, padded to a 4-byte boundary.
                let expected_num_objects = (super::message::extended::ExtendedHeader::SIZE
                    + core::cmp::min(total_payload_len, MAX_EXTENDED_MSG_CHUNK_LEN))
                .div_ceil(4);
                assert_eq!(frame_header.num_objects(), expected_num_objects);
                // Chunk 0 carries only one chunk of data; a whole-payload frame would be
                // `+ total_payload_len` bytes (the pre-fix bug).
                let expected_frame_len = super::MSG_HEADER_SIZE
                    + super::message::extended::ExtendedHeader::SIZE
                    + core::cmp::min(total_payload_len, MAX_EXTENDED_MSG_CHUNK_LEN);
                assert_eq!(bytes.len(), expected_frame_len);
                chunk_numbers.push(ext_header.chunk_number()).unwrap();
            }
            assert_eq!(chunk_numbers.len(), 1, "chunk 0 sent; remaining chunks on request");
            assert_eq!(chunk_numbers.into_iter().next(), Some(0));
        }
    }

    /// A chunk request frame received after an oversized chunked TX must be answered
    /// with the requested chunk (26 payload bytes) from the retained payload
    /// (PD 3.2 Sec. 6.12.2.1.2.2).
    #[cfg(feature = "epr")]
    #[tokio::test]
    async fn test_chunk_request_is_served_from_retained_payload() {
        use super::message::header::{ControlMessageType, ExtendedMessageType, MessageType};

        let mut protocol_layer = get_protocol_layer_with_source_role();

        use super::message::data::source_capabilities::{PowerDataObject, parse_raw_pdo};

        // 60-byte payload (15 PDOs) -> would be 3 chunks in full.
        let pdos: heapless::Vec<PowerDataObject, 16> = (0..15)
            .map(|i| parse_raw_pdo((0b01 << 30) | (1000 + i as u32 * 2) | ((400 + i as u32) << 8) | (1500 << 17)))
            .collect();
        let caps = SourceCapabilities(pdos);

        // GoodCRC for the initial chunk 0 transmit (tx id 0); then drop chunk 0 frame.
        protocol_layer.driver.inject_received_data(&good_crc_bytes(0));
        protocol_layer.transmit_epr_source_capabilities(&caps).await.unwrap();
        _ = protocol_layer.driver.probe_transmitted_data();

        // Build a chunk request for chunk 1 of EPR_Source_Capabilities.
        let request_ext_header = super::message::extended::ExtendedHeader::default()
            .with_chunked(true)
            .with_request_chunk(true)
            .with_chunk_number(1);
        let request_header = Header::new_extended(
            *protocol_layer.header(),
            crate::counters::Counter::new(crate::counters::CounterType::MessageId),
            ExtendedMessageType::EprSourceCapabilities,
            1,
        );
        let mut request_frame = [0u8; 8];
        let mut offset = request_header.to_bytes(&mut request_frame);
        offset += request_ext_header.to_bytes(&mut request_frame[offset..]);

        // Request first, then the GoodCRC for the upcoming served-chunk response (tx id 1).
        protocol_layer.driver.inject_received_data(&request_frame[..offset]);
        protocol_layer.driver.inject_received_data(&good_crc_bytes(1));

        // Receiving the request must serve the chunk (side effect). The served frame then
        // consumes the goodCRC, leaving nothing for `receive_message`, so it times out.
        // A pre-fix layer errors immediately (assembly failure) - also an Err, but then the
        // TX probe below finds no served chunk and panics: the discriminator.
        tokio::time::timeout(core::time::Duration::from_millis(50), protocol_layer.receive_message())
            .await
            .expect_err("receive must keep waiting after serving the chunk");
        // The served flow transmits two frames: the GoodCRC for the request, then the chunk.
        let ack_bytes = protocol_layer.driver().probe_transmitted_data();
        let ack_header = Header::from_bytes(&ack_bytes[..super::MSG_HEADER_SIZE]).unwrap();
        assert!(matches!(
            ack_header.message_type(),
            MessageType::Control(ControlMessageType::GoodCRC)
        ));
        let response_bytes = protocol_layer.driver().probe_transmitted_data();

        assert_eq!(
            response_bytes.len(),
            super::MSG_HEADER_SIZE + super::message::extended::ExtendedHeader::SIZE + MAX_EXTENDED_MSG_CHUNK_LEN
        );
        let response_ext = super::message::extended::ExtendedHeader::from_bytes(
            &response_bytes
                [super::MSG_HEADER_SIZE..super::MSG_HEADER_SIZE + super::message::extended::ExtendedHeader::SIZE],
        );
        assert!(response_ext.chunked());
        assert!(!response_ext.request_chunk());
        assert_eq!(response_ext.chunk_number(), 1);
        assert_eq!(response_ext.data_size(), 60);
        // The main header must echo the EPR_Source_Capabilities type and carry the
        // object count per PD 3.2 Sec. 6.2.1.1.2 (ext header + 26 bytes -> 7 objects).
        let response_header = Header::from_bytes(&response_bytes[..super::MSG_HEADER_SIZE]).unwrap();
        assert!(matches!(
            response_header.message_type(),
            MessageType::Extended(ExtendedMessageType::EprSourceCapabilities)
        ));
        assert_eq!(
            response_header.num_objects(),
            (super::message::extended::ExtendedHeader::SIZE + MAX_EXTENDED_MSG_CHUNK_LEN).div_ceil(4)
        );
        // Payload bytes: chunk 1 begins at payload byte 26, i.e. mid-PDO.
        // pdo[6] raw = 0x4BB997F4 (LE: F4 97 B9 4B) - its last 2 bytes open the chunk,
        // followed by the first 2 bytes of pdo[7] (0x4BB997F6 -> F6 97).
        let first_word = u32::from_le_bytes(
            response_bytes[super::MSG_HEADER_SIZE + super::message::extended::ExtendedHeader::SIZE
                ..super::MSG_HEADER_SIZE + super::message::extended::ExtendedHeader::SIZE + 4]
                .try_into()
                .unwrap(),
        );
        let pdo6 = (0b01u32 << 30) | (1000 + 6 * 2) | ((400 + 6) << 8) | (1500 << 17);
        let pdo7 = (0b01u32 << 30) | (1000 + 7 * 2) | ((400 + 7) << 8) | (1500 << 17);
        let expected = (pdo6 >> 16) | ((pdo7 & 0xFFFF) << 16);
        assert_eq!(first_word, expected);
    }

    /// A chunk request whose chunk number points beyond the retained payload must be
    /// silently ignored: no response, no panic. The partner's chunk timer runs its own
    /// error flow. Pre-fix, the layer panicked slicing `buffer[start..end]`.
    #[cfg(feature = "epr")]
    #[tokio::test]
    async fn test_chunk_request_out_of_range_is_silently_ignored() {
        use super::message::header::{ControlMessageType, ExtendedMessageType, MessageType};

        let mut protocol_layer = get_protocol_layer_with_source_role();

        use super::message::data::source_capabilities::{PowerDataObject, parse_raw_pdo};

        // 40-byte payload (10 PDOs): only chunks 0 (26 B) and 1 (14 B) exist.
        let pdos: heapless::Vec<PowerDataObject, 16> = (0..10)
            .map(|i| parse_raw_pdo((0b01 << 30) | (1000 + i as u32 * 2) | ((400 + i as u32) << 8) | (1500 << 17)))
            .collect();
        let caps = SourceCapabilities(pdos);

        protocol_layer.driver.inject_received_data(&good_crc_bytes(0));
        protocol_layer.transmit_epr_source_capabilities(&caps).await.unwrap();
        _ = protocol_layer.driver.probe_transmitted_data();

        // Request chunk 7: far beyond the 2-chunk payload.
        let request_ext_header = super::message::extended::ExtendedHeader::default()
            .with_chunked(true)
            .with_request_chunk(true)
            .with_chunk_number(7);
        let request_header = Header::new_extended(
            *protocol_layer.header(),
            crate::counters::Counter::new(crate::counters::CounterType::MessageId),
            ExtendedMessageType::EprSourceCapabilities,
            1,
        );
        let mut request_frame = [0u8; 8];
        let mut offset = request_header.to_bytes(&mut request_frame);
        offset += request_ext_header.to_bytes(&mut request_frame[offset..]);

        protocol_layer.driver.inject_received_data(&request_frame[..offset]);

        // The request is acked then dropped. `receive_message` must keep waiting
        // (not panic, not transmit a chunk response).

        tokio::time::timeout(core::time::Duration::from_millis(50), protocol_layer.receive_message())
            .await
            .expect_err("receive must keep waiting after an out-of-range request");
        let ack_bytes = protocol_layer.driver().probe_transmitted_data();
        let ack_header = Header::from_bytes(&ack_bytes[..super::MSG_HEADER_SIZE]).unwrap();
        assert!(matches!(
            ack_header.message_type(),
            MessageType::Control(ControlMessageType::GoodCRC)
        ));
        assert!(
            !protocol_layer.driver().has_transmitted_data(),
            "no chunk response may follow an out-of-range request"
        );
    }

    /// A duplicate chunk request (same MessageID) must be re-acked and the chunk
    /// re-served: the re-served response is the correct retry for a lost response
    /// (idempotent, bytes come from the retained payload).
    #[cfg(feature = "epr")]
    #[tokio::test]
    async fn test_duplicate_chunk_request_is_reacked_and_reserved() {
        use super::message::header::{ControlMessageType, ExtendedMessageType, MessageType};

        let mut protocol_layer = get_protocol_layer_with_source_role();

        use super::message::data::source_capabilities::{PowerDataObject, parse_raw_pdo};

        let pdos: heapless::Vec<PowerDataObject, 16> = (0..10)
            .map(|i| parse_raw_pdo((0b01 << 30) | (1000 + i as u32 * 2) | ((400 + i as u32) << 8) | (1500 << 17)))
            .collect();
        let caps = SourceCapabilities(pdos);

        protocol_layer.driver.inject_received_data(&good_crc_bytes(0));
        protocol_layer.transmit_epr_source_capabilities(&caps).await.unwrap();
        _ = protocol_layer.driver.probe_transmitted_data();

        // Chunk 1 request with MessageID 1, sent twice.
        let request_ext_header = super::message::extended::ExtendedHeader::default()
            .with_chunked(true)
            .with_request_chunk(true)
            .with_chunk_number(1);
        let request_header = Header::new_extended(
            *protocol_layer.header(),
            crate::counters::Counter::new_from_value(crate::counters::CounterType::MessageId, 1),
            ExtendedMessageType::EprSourceCapabilities,
            1,
        );
        let mut request_frame = [0u8; 8];
        let mut offset = request_header.to_bytes(&mut request_frame);
        offset += request_ext_header.to_bytes(&mut request_frame[offset..]);

        // First request: fresh (acked, then served). The served chunk carries TX MessageID 1,
        // so its GoodCRC must echo ID 1.
        protocol_layer.driver.inject_received_data(&request_frame[..offset]);
        protocol_layer.driver.inject_received_data(&good_crc_bytes(1));
        tokio::time::timeout(core::time::Duration::from_millis(50), protocol_layer.receive_message())
            .await
            .expect_err("receive must keep waiting after serving chunk 1");
        let ack1 = protocol_layer.driver().probe_transmitted_data();
        let served1 = protocol_layer.driver().probe_transmitted_data();
        assert!(matches!(
            Header::from_bytes(&ack1[..super::MSG_HEADER_SIZE])
                .unwrap()
                .message_type(),
            MessageType::Control(ControlMessageType::GoodCRC)
        ));
        assert_eq!(
            served1.len(),
            super::MSG_HEADER_SIZE + super::message::extended::ExtendedHeader::SIZE + (40 - MAX_EXTENDED_MSG_CHUNK_LEN)
        );

        // Duplicate request: same MessageID -> re-ack; the re-served chunk carries TX
        // MessageID 2 (the counter advanced when chunk 1 was acknowledged).
        protocol_layer.driver.inject_received_data(&request_frame[..offset]);
        protocol_layer.driver.inject_received_data(&good_crc_bytes(2));
        tokio::time::timeout(core::time::Duration::from_millis(50), protocol_layer.receive_message())
            .await
            .expect_err("receive must keep waiting after re-serving chunk 1");
        let ack2 = protocol_layer.driver().probe_transmitted_data();
        let served2 = protocol_layer.driver().probe_transmitted_data();
        assert!(matches!(
            Header::from_bytes(&ack2[..super::MSG_HEADER_SIZE])
                .unwrap()
                .message_type(),
            MessageType::Control(ControlMessageType::GoodCRC)
        ));
        // The re-served response is a new frame: its MessageID advances per Sec. 6.7.1.2.
        // The chunk content itself must be identical.
        assert_eq!(
            served1[super::MSG_HEADER_SIZE..],
            served2[super::MSG_HEADER_SIZE..],
            "re-served chunk content must be identical"
        );
    }

    /// A driver that always discards transmissions (wedged state machine) must not
    /// make `hard_reset()` loop forever: the loop is bounded by `HardResetCompleteTimer`.
    #[tokio::test]
    async fn test_hard_reset_bounded_when_driver_discards() {
        use usbpd_traits::{DriverRxError, DriverTxError};

        struct DiscardingDriver;

        impl usbpd_traits::Driver for DiscardingDriver {
            async fn receive(&mut self, _buffer: &mut [u8]) -> Result<usize, DriverRxError> {
                Err(DriverRxError::Discarded)
            }
            async fn transmit(&mut self, _data: &[u8]) -> Result<(), DriverTxError> {
                Err(DriverTxError::Discarded)
            }
            async fn transmit_hard_reset(&mut self) -> Result<(), DriverTxError> {
                Err(DriverTxError::Discarded)
            }
            async fn wait_for_vbus(&mut self) {}
        }

        let mut protocol_layer = ProtocolLayer::<DiscardingDriver, DummyTimer>::new(
            DiscardingDriver,
            crate::protocol_layer::message::header::Header::new_template(
                crate::DataRole::Ufp,
                crate::PowerRole::Sink,
                super::message::header::SpecificationRevision::R3_X,
            ),
        );

        // Pre-fix, this looped forever. It must now return an error.
        let result = protocol_layer.hard_reset().await;
        assert!(
            result.is_err(),
            "hard_reset must fail on a persistently discarding driver"
        );
    }
}

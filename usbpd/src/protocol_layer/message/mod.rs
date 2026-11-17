//! Definitions of message content.

pub mod data;
pub mod extended;
pub mod header;

use byteorder::{ByteOrder, LittleEndian};
pub use extended::{ExtendedHeader, MAX_EXTENDED_MSG_CHUNK_LEN};
use header::{Header, MessageType};

/// Size of the PD message header in bytes.
pub const MSG_HEADER_SIZE: usize = header::Header::SIZE;

#[cfg(test)]
#[cfg(feature = "epr")]
mod epr_messages_test;

/// Errors that can occur during message/header parsing.
#[derive(thiserror::Error, Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ParseError {
    /// The input buffer has an invalid length.
    /// * `expected` - The expected length.
    /// * `found` - The actual length found.
    #[error("invalid input buffer length (expected {expected:?}, found {found:?})")]
    InvalidLength {
        /// The expected length.
        expected: usize,
        /// The actual length found.
        found: usize,
    },
    /// The specification revision field is not supported.
    #[error("unsupported specification revision `{0}`")]
    UnsupportedSpecificationRevision(u8),
    /// An unknown or reserved message type was encountered.
    #[error("unknown or reserved message type `{0}`")]
    InvalidMessageType(u8),
    /// An unknown or reserved data message type was encountered.
    #[error("unknown or reserved data message type `{0}`")]
    InvalidDataMessageType(u8),
    /// An unknown or reserved control message type was encountered.
    #[error("unknown or reserved control message type `{0}`")]
    InvalidControlMessageType(u8),
    /// Received a chunked extended message that requires assembly.
    /// The protocol layer handles chunk assembly transparently.
    #[error("chunked extended message (chunk {chunk_number}, total size {data_size})")]
    ChunkedExtendedMessage {
        /// The chunk number (0 = first chunk).
        chunk_number: u8,
        /// Total data size across all chunks.
        data_size: u16,
        /// Whether this is a chunk request.
        request_chunk: bool,
        /// The extended message type.
        message_type: header::ExtendedMessageType,
    },
    /// Other parsing error with a message.
    #[error("other parse error: {0}")]
    Other(&'static str),
}

/// Payload of a USB PD message, if any.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Payload {
    /// Payload for a data message.
    Data(data::Data),
    /// Payload for an extended message.
    Extended(extended::Extended),
}

/// A USB PD message.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Message {
    /// The message header.
    pub header: Header,
    /// Optional payload for  messages.
    pub payload: Option<Payload>,
}

impl Message {
    /// Create a new message from a message header.
    pub fn new(header: Header) -> Self {
        Self { header, payload: None }
    }

    /// Create a new message from a message header and payload data.
    pub fn new_with_data(header: Header, data: data::Data) -> Self {
        Self {
            header,
            payload: Some(Payload::Data(data)),
        }
    }

    /// Serialize a message to a slice, returning the number of written bytes.
    pub fn to_bytes(&self, buffer: &mut [u8]) -> usize {
        let header_len = self.header.to_bytes(buffer);

        match self.payload.as_ref() {
            Some(Payload::Data(data)) => header_len + data.to_bytes(&mut buffer[header_len..]),
            Some(Payload::Extended(extended)) => {
                // Per PD 3.2 Sec. 6.2.1.2.5.2 (TCH_Construct_Chunked_Message): payloads
                // larger than 26 bytes must be split into chunks of at most 26 bytes.
                // Chunked mode is used for all extended messages, per Sec. 6.2.1.2.1.
                // Only the first chunk is transmitted here; the partner requests the
                // remaining chunks (`TCH_Request_Chunk`), which the protocol layer
                // answers from a retained copy of the full payload.
                let full_len = extended.to_bytes(&mut buffer[header_len + ExtendedHeader::SIZE..]);
                debug_assert_eq!(usize::from(extended.data_size()), full_len);
                let first_chunk_len = core::cmp::min(full_len, MAX_EXTENDED_MSG_CHUNK_LEN);
                let extended_header = ExtendedHeader::new(extended.data_size())
                    .with_chunked(true)
                    .with_chunk_number(0);
                header_len + extended_header.to_bytes(&mut buffer[header_len..]) + first_chunk_len
            }
            None => header_len,
        }
    }

    /// Deserialize an assembled extended message payload into an [Extended] enum.
    ///
    /// Shared by `from_bytes` (single-chunk frames) and the protocol layer's
    /// chunk-assembly path.
    pub fn parse_extended_payload(message_type: header::ExtendedMessageType, payload: &[u8]) -> extended::Extended {
        match message_type {
            header::ExtendedMessageType::ExtendedControl => {
                if let Ok(buf) = <&[u8; 2]>::try_from(payload) {
                    extended::Extended::ExtendedControl(extended::extended_control::ExtendedControl(
                        LittleEndian::read_u16(buf),
                    ))
                } else {
                    extended::Extended::Unknown
                }
            }
            #[cfg(feature = "epr")]
            header::ExtendedMessageType::EprSourceCapabilities => {
                extended::Extended::EprSourceCapabilities(extended::epr::parse_pdos::<16>(payload))
            }
            _ => extended::Extended::Unknown,
        }
    }

    /// Parse a message from a slice of bytes.
    pub fn from_bytes(data: &[u8]) -> Result<Self, ParseError> {
        let header = Header::from_bytes(&data[..2])?;
        let message = Self::new(header);
        let payload = &data[2..];

        match message.header.message_type() {
            MessageType::Control(_) => Ok(message),
            MessageType::Extended(message_type) => {
                let ext_header = ExtendedHeader::from_bytes(payload);
                let data_size = ext_header.data_size() as usize;

                // Check if this is a true multi-chunk message that needs assembly
                // Single-chunk messages (chunk 0 with all data present) can be parsed directly
                if ext_header.chunked() {
                    let is_chunk_request = ext_header.request_chunk();
                    let chunk_number = ext_header.chunk_number();
                    let available_payload = payload.len().saturating_sub(2);

                    // Multi-chunk required if:
                    // - This is a chunk request, OR
                    // - Chunk number > 0 (continuation chunk), OR
                    // - Data size exceeds what's available in this chunk
                    let needs_assembly = is_chunk_request || chunk_number > 0 || data_size > available_payload;

                    if needs_assembly {
                        return Err(ParseError::ChunkedExtendedMessage {
                            chunk_number,
                            data_size: ext_header.data_size(),
                            request_chunk: is_chunk_request,
                            message_type,
                        });
                    }
                    // Otherwise, it's a single-chunk message - parse normally
                }
                if payload.len() < 2 + data_size {
                    return Err(ParseError::InvalidLength {
                        expected: 2 + data_size,
                        found: payload.len(),
                    });
                }

                let payload_bytes = &payload[2..2 + data_size];
                Ok(Self {
                    payload: Some(Payload::Extended(Self::parse_extended_payload(
                        message_type,
                        payload_bytes,
                    ))),
                    ..message
                })
            }
            MessageType::Data(message_type) => data::Data::parse_message(message, message_type, payload, &()),
        }
    }
}

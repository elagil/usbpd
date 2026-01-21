//! Definitions of message content.

// FIXME: add documentation
pub mod data;
pub mod extended;
#[allow(missing_docs)]
pub mod header;

#[cfg(test)]
mod epr_messages_test;

use byteorder::{ByteOrder, LittleEndian};
use header::{Header, MessageType};

use crate::protocol_layer::message::extended::ExtendedHeader;

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
    /// Use `ChunkedMessageAssembler` to handle these messages.
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
    /// Received a chunk larger than the maximum allowed size (26 bytes).
    #[error("chunk size {0} exceeds maximum {1}")]
    ChunkOverflow(usize, usize),
    /// Attempt to reuse a ChunkedMessageAssembler that is already processing a message.
    /// The user must create a new assembler or explicitly call reset() first.
    #[error("parser already in use, create a new assembler or call reset()")]
    ParserReuse,
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
                // Per USB PD spec 6.2.1.2.1: use chunked mode for compatibility with more PHYs.
                // Most power supplies don't support unchunked extended messages.
                let extended_header = ExtendedHeader::new(extended.data_size())
                    .with_chunked(true)
                    .with_chunk_number(0);
                let ext_header_len = extended_header.to_bytes(&mut buffer[header_len..]);
                header_len + ext_header_len + extended.to_bytes(&mut buffer[header_len + ext_header_len..])
            }
            None => header_len,
        }
    }

    /// Parse assembled extended message payload into an Extended enum.
    ///
    /// This is used after `ChunkedMessageAssembler` has assembled all chunks.
    ///
    /// # Arguments
    /// * `message_type` - The extended message type
    /// * `payload` - The complete assembled payload data
    pub fn parse_extended_payload(message_type: header::ExtendedMessageType, payload: &[u8]) -> extended::Extended {
        match message_type {
            header::ExtendedMessageType::ExtendedControl => {
                if payload.len() >= 2 {
                    extended::Extended::ExtendedControl(extended::extended_control::ExtendedControl(
                        LittleEndian::read_u16(payload),
                    ))
                } else {
                    extended::Extended::Unknown
                }
            }
            header::ExtendedMessageType::EprSourceCapabilities => extended::Extended::EprSourceCapabilities(
                payload
                    .chunks_exact(4)
                    .map(|buf| {
                        crate::protocol_layer::message::data::source_capabilities::parse_raw_pdo(
                            LittleEndian::read_u32(buf),
                        )
                    })
                    .collect(),
            ),
            _ => extended::Extended::Unknown,
        }
    }

    /// Parse an extended message chunk, returning the header info and chunk data.
    ///
    /// This is used for handling chunked extended messages when `from_bytes`
    /// returns `ParseError::ChunkedExtendedMessage`.
    ///
    /// Returns (Header, ExtendedHeader, chunk_payload_data).
    pub fn parse_extended_chunk(data: &[u8]) -> Result<(Header, ExtendedHeader, &[u8]), ParseError> {
        if data.len() < 4 {
            return Err(ParseError::InvalidLength {
                expected: 4,
                found: data.len(),
            });
        }

        let header = Header::from_bytes(&data[..2])?;
        let ext_header = ExtendedHeader::from_bytes(&data[2..]);

        // Chunk payload starts after headers (2 + 2 = 4 bytes)
        let chunk_payload = &data[4..];

        Ok((header, ext_header, chunk_payload))
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
                    payload: Some(Payload::Extended(match message_type {
                        header::ExtendedMessageType::ExtendedControl => extended::Extended::ExtendedControl(
                            extended::extended_control::ExtendedControl(LittleEndian::read_u16(payload_bytes)),
                        ),
                        header::ExtendedMessageType::EprSourceCapabilities => {
                            extended::Extended::EprSourceCapabilities(
                                payload_bytes
                                    .chunks_exact(4)
                                    .map(|buf| {
                                        crate::protocol_layer::message::data::source_capabilities::parse_raw_pdo(
                                            LittleEndian::read_u32(buf),
                                        )
                                    })
                                    .collect(),
                            )
                        }
                        _ => extended::Extended::Unknown,
                    })),
                    ..message
                })
            }
            MessageType::Data(message_type) => data::Data::parse_message(message, message_type, payload, &()),
        }
    }
}

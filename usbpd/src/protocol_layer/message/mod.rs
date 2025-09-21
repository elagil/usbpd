//! Definitions of message content.

// FIXME: add documentation
pub mod data;
pub mod extended;
#[allow(missing_docs)]
pub mod header;

use header::{Header, MessageType};

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
        self.header.to_bytes(buffer)
            + match self.payload.as_ref() {
                Some(Payload::Data(data)) => data.to_bytes(&mut buffer[2..]),
                Some(Payload::Extended(extended)) => extended.to_bytes(&mut buffer[2..]),
                None => 0,
            }
    }

    /// Parse a message from a slice of bytes.
    pub fn from_bytes(data: &[u8]) -> Result<Self, ParseError> {
        let header = Header::from_bytes(&data[..2])?;
        let message = Self::new(header);
        let payload = &data[2..];

        match message.header.message_type() {
            MessageType::Control(_) => Ok(message),
            MessageType::Extended(_) => Ok(message),
            MessageType::Data(message_type) => data::Data::parse_message(message, message_type, payload, &()),
        }
    }
}

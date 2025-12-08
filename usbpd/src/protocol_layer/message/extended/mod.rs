//! Definitions and implementations of extended messages.
//!
//! See [6.5].

pub mod chunked;
pub mod extended_control;
use byteorder::{ByteOrder, LittleEndian};
use heapless::Vec;
use proc_bitfield::bitfield;

use crate::protocol_layer::message::data::sink_capabilities::SinkPowerDataObject;
use crate::protocol_layer::message::data::source_capabilities::PowerDataObject;

/// Types of extended messages.
///
/// TODO: Add missing types as per [6.5] and [Table 6.53].
#[derive(Debug, Clone)]
#[non_exhaustive]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[allow(unused)]
pub enum Extended {
    /// Extended source capabilities.
    SourceCapabilitiesExtended,
    /// Extended control message payload.
    ExtendedControl(extended_control::ExtendedControl),
    /// EPR source capabilities list.
    EprSourceCapabilities(Vec<PowerDataObject, 16>),
    /// EPR sink capabilities list.
    EprSinkCapabilities(Vec<SinkPowerDataObject, 7>),
    /// Unknown data type.
    Unknown,
}

impl Extended {
    /// Size of the extended payload in bytes.
    pub fn data_size(&self) -> u16 {
        match self {
            Self::SourceCapabilitiesExtended => 0,
            Self::ExtendedControl(_payload) => 2,
            Self::EprSourceCapabilities(pdos) => (pdos.len() * core::mem::size_of::<u32>()) as u16,
            Self::EprSinkCapabilities(pdos) => (pdos.len() * core::mem::size_of::<u32>()) as u16,
            Self::Unknown => 0,
        }
    }

    /// Serialize message data to a slice, returning the number of written bytes.
    pub fn to_bytes(&self, payload: &mut [u8]) -> usize {
        match self {
            Self::Unknown => 0,
            Self::SourceCapabilitiesExtended => unimplemented!(),
            Self::ExtendedControl(control) => control.to_bytes(payload),
            Self::EprSourceCapabilities(pdos) => {
                let mut written = 0;
                for pdo in pdos {
                    let raw = match pdo {
                        PowerDataObject::FixedSupply(p) => p.0,
                        PowerDataObject::Battery(p) => p.0,
                        PowerDataObject::VariableSupply(p) => p.0,
                        PowerDataObject::Augmented(a) => match a {
                            crate::protocol_layer::message::data::source_capabilities::Augmented::Spr(p) => p.0,
                            crate::protocol_layer::message::data::source_capabilities::Augmented::Epr(p) => p.0,
                            crate::protocol_layer::message::data::source_capabilities::Augmented::Unknown(p) => *p,
                        },
                        PowerDataObject::Unknown(p) => p.0,
                    };
                    LittleEndian::write_u32(&mut payload[written..written + 4], raw);
                    written += 4;
                }
                written
            }
            Self::EprSinkCapabilities(pdos) => {
                let mut written = 0;
                for pdo in pdos {
                    LittleEndian::write_u32(&mut payload[written..written + 4], pdo.to_raw());
                    written += 4;
                }
                written
            }
        }
    }
}

bitfield! {
    /// Extended message header.
    ///
    /// Chunked messages are currently unsupported.
    #[derive(Clone, Copy, PartialEq, Eq)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
    pub struct ExtendedHeader(pub u16): Debug, FromStorage, IntoStorage {
        /// Payload size in bytes.
        pub data_size: u16 @ 0..=8,
        /// Request chunk flag.
        pub request_chunk: bool @ 10,
        /// Chunk number of this extended message.
        pub chunk_number: u8 @ 11..=14,
        /// Whether the message is chunked.
        pub chunked: bool @ 15,
    }
}

impl ExtendedHeader {
    /// Create a new, unchunked extended header for a given payload size.
    pub fn new(data_size: u16) -> Self {
        Self(0).with_data_size(data_size)
    }

    /// Serialize the extended header into the buffer, returning bytes written.
    pub fn to_bytes(self, buf: &mut [u8]) -> usize {
        LittleEndian::write_u16(buf, self.0);
        2
    }

    /// Parse an extended header from bytes.
    pub fn from_bytes(buf: &[u8]) -> Self {
        assert!(buf.len() >= 2);
        Self(LittleEndian::read_u16(buf))
    }
}

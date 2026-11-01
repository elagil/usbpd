//! Definitions and implementations of extended messages.
//!
//! Per PD 3.2 Sec. 6.5.

pub mod chunked;
#[cfg(feature = "epr")]
pub mod epr;
pub mod extended_control;
use byteorder::{ByteOrder, LittleEndian};
#[cfg(feature = "epr")]
use heapless::Vec;
use proc_bitfield::bitfield;

use crate::protocol_layer::message::data;

/// Extended message payload types, per PD 3.2 Sec. 6.5.
#[derive(Debug, Clone)]
#[non_exhaustive]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[allow(unused)]
// TODO: Add missing types per PD 3.2 Sec. 6.5 and Tab. 6.53.
pub enum Extended {
    /// Extended source capabilities.
    SourceCapabilitiesExtended,
    /// Extended control message payload.
    ExtendedControl(extended_control::ExtendedControl),
    /// EPR source capabilities list.
    #[cfg(feature = "epr")]
    EprSourceCapabilities(Vec<data::source_capabilities::PowerDataObject, 16>),
    /// EPR sink capabilities list.
    #[cfg(feature = "epr")]
    EprSinkCapabilities(Vec<data::sink_capabilities::SinkPowerDataObject, 7>),
    /// Unknown data type.
    Unknown,
}

impl Extended {
    /// Size of the extended payload in bytes.
    pub fn data_size(&self) -> u16 {
        match self {
            Self::SourceCapabilitiesExtended => 0,
            Self::ExtendedControl(_payload) => 2,
            #[cfg(feature = "epr")]
            Self::EprSourceCapabilities(pdos) => (pdos.len() * core::mem::size_of::<u32>()) as u16,
            #[cfg(feature = "epr")]
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
            #[cfg(feature = "epr")]
            Self::EprSourceCapabilities(pdos) => epr::serialize_pdos(pdos, payload),
            #[cfg(feature = "epr")]
            Self::EprSinkCapabilities(pdos) => epr::serialize_pdos(pdos, payload),
        }
    }
}

bitfield! {
    /// Extended message header.
    ///
    /// Chunked messages are currently unsupported.
    #[derive(Clone, Copy, PartialEq, Eq, Default)]
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

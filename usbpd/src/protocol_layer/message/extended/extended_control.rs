//! Definitions of extended control message content.
//!
//! See [6.5.14].

use byteorder::{ByteOrder, LittleEndian};
use proc_bitfield::bitfield;

/// Types of extended control message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ExtendedControlMessageType {
    /// Get capabilities offered by a source in EPR mode.
    ///
    /// See [6.5.14.1].
    EprGetSourceCap,
    /// Get capabilities offered by a sink in EPR mode.
    ///
    /// See [6.5.14.2].
    EprGetSinkCap,
    /// The EPR keep-alive message may be sent by a sink operating in EPR mode to meet the requirement for periodic traffic.
    ///
    /// See [6.5.14.3].
    EprKeepAlive,
    /// The EPR keep-alive ack message shall be sent by a source operating in EPR mode in response to an [`Self::EprKeepAlive`] message.
    EprKeepAliveAck,
}

impl From<ExtendedControlMessageType> for u8 {
    fn from(value: ExtendedControlMessageType) -> Self {
        match value {
            ExtendedControlMessageType::EprGetSourceCap => 1,
            ExtendedControlMessageType::EprGetSinkCap => 2,
            ExtendedControlMessageType::EprKeepAlive => 3,
            ExtendedControlMessageType::EprKeepAliveAck => 4,
        }
    }
}

impl TryFrom<u8> for ExtendedControlMessageType {
    type Error = ();

    /// Per spec 6.5.14 Table 6.67, all values not explicitly defined are `Reserved` and Shall Not be used.
    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(ExtendedControlMessageType::EprGetSourceCap),
            2 => Ok(ExtendedControlMessageType::EprGetSinkCap),
            3 => Ok(ExtendedControlMessageType::EprKeepAlive),
            4 => Ok(ExtendedControlMessageType::EprKeepAliveAck),
            _ => Err(()),
        }
    }
}

bitfield!(
    /// The extended control message extends the control message space.
    ///
    /// Includes one byte of data.
    #[derive(Clone, Copy, PartialEq, Eq)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
    pub struct ExtendedControl(pub u16): Debug, FromStorage, IntoStorage {
        /// Payload, shall be set to zero when not used.
        pub data: u8 @ 8..=15,
        /// The extended control message type. Reserved values fail to parse (see spec 6.5.14 Table 6.67) and
        /// message consumers answer `Extended::Unknown` with `Not_Supported`.
        pub try_message_type: u8 [try_get ExtendedControlMessageType] @ 0..=7,
    }
);

impl ExtendedControl {
    /// Store the extended control message in a binary buffer, returning the written size in number of bytes.
    pub fn to_bytes(self, buf: &mut [u8]) -> usize {
        LittleEndian::write_u16(buf, self.0);
        2
    }

    /// Set the extended control message type from a typed value.
    pub fn with_message_type(mut self, message_type: ExtendedControlMessageType) -> Self {
        self.0 = (self.0 & !0xFF) | u8::from(message_type) as u16;
        self
    }

    /// Parse an extended control message from bytes.
    pub fn from_bytes(buf: &[u8]) -> Self {
        assert!(buf.len() >= 2);
        Self(LittleEndian::read_u16(buf))
    }
}

impl Default for ExtendedControl {
    fn default() -> Self {
        Self(0).with_data(0)
    }
}

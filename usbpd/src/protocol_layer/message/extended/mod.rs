//! Definitions and implementations of extended messages.
//!
//! See [6.5].

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
    /// Extended control message.
    ExtendedControl,
    /// Unknown data type.
    Unknown,
}

impl Extended {
    /// Serialize message data to a slice, returning the number of written bytes.
    pub fn to_bytes(&self, _payload: &mut [u8]) -> usize {
        match self {
            Self::Unknown => 0,
            Self::SourceCapabilitiesExtended => unimplemented!(),
            Self::ExtendedControl => unimplemented!(),
        }
    }
}

//! Definitions of EPR mode data message content.
//!
//! See [6.4.10].
use proc_bitfield::bitfield;

/// Possible actions, encoded in the EPR mode data object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Action {
    /// Enter EPR mode.
    Enter,
    /// Entering EPR mode was acknowledged.
    EnterAcknowledged,
    /// Entering EPR mode succeeded.
    EnterSucceeded,
    /// Entering EPR mode failed.
    EnterFailed,
    /// Exit EPR mode.
    Exit,
}

impl From<Action> for u8 {
    fn from(value: Action) -> Self {
        match value {
            Action::Enter => 0x01,
            Action::EnterAcknowledged => 0x02,
            Action::EnterSucceeded => 0x03,
            Action::EnterFailed => 0x04,
            Action::Exit => 0x05,
        }
    }
}

impl From<u8> for Action {
    fn from(value: u8) -> Self {
        match value {
            0x01 => Action::Enter,
            0x02 => Action::EnterAcknowledged,
            0x03 => Action::EnterSucceeded,
            0x04 => Action::EnterFailed,
            0x05 => Action::Exit,
            _ => panic!("Cannot convert {} to Action", value), // Illegal values shall panic.
        }
    }
}

bitfield! {
    /// The EPR mode data object that encodes an action, as well as corresponding payload data.
    ///
    /// See [Table 6.50].
    #[derive(Clone, Copy, PartialEq, Eq)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
    pub struct EprModeDataObject(pub u32): Debug, FromStorage, IntoStorage {
        /// Action to perform with regard to EPR mode (e.g. enter).
        pub action: u8 [Action] @ 24..=31,
        /// Payload data that is attached to an [`Self::action`]
        pub data: u8 @ 16..=23,
    }
}

#[allow(clippy::derivable_impls)]
impl Default for EprModeDataObject {
    fn default() -> Self {
        Self(0)
    }
}

/// Causes for failing to enter EPR mode.
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum DataEnterFailed {
    /// Unknown cause.
    UnknownCause,
    /// The cable is not EPR capable.
    CableNotEprCapable,
    /// The source failed to become the Vconn source.
    SourceFailedToBecomeVconnSource,
    /// The "EPR capable" bit is not set in RDO.
    EprCapableBitNotSetInRdo,
    /// The source is unable to enter EPR mode.
    ///
    /// The sink may retry entering EPR mode after receiving this [`Action::EnterFailed`] response.
    SourceUnableToEnterEprMode,
    /// The "EPR capable" bit is not set in PDO.
    EprCapableBitNotSetInPdo,
}

impl From<DataEnterFailed> for u8 {
    fn from(value: DataEnterFailed) -> Self {
        match value {
            DataEnterFailed::UnknownCause => 0x00,
            DataEnterFailed::CableNotEprCapable => 0x01,
            DataEnterFailed::SourceFailedToBecomeVconnSource => 0x02,
            DataEnterFailed::EprCapableBitNotSetInRdo => 0x03,
            DataEnterFailed::SourceUnableToEnterEprMode => 0x04,
            DataEnterFailed::EprCapableBitNotSetInPdo => 0x05,
        }
    }
}

impl From<u8> for DataEnterFailed {
    fn from(value: u8) -> Self {
        match value {
            0x00 => DataEnterFailed::UnknownCause,
            0x01 => DataEnterFailed::CableNotEprCapable,
            0x02 => DataEnterFailed::SourceFailedToBecomeVconnSource,
            0x03 => DataEnterFailed::EprCapableBitNotSetInRdo,
            0x04 => DataEnterFailed::SourceUnableToEnterEprMode,
            0x05 => DataEnterFailed::EprCapableBitNotSetInPdo,
            _ => panic!("Cannot convert {} to DataEnterFailed", value), // Illegal values shall panic.
        }
    }
}

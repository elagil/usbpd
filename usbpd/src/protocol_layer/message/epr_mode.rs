// use byteorder::{ByteOrder, LittleEndian};
use proc_bitfield::bitfield;

bitfield! {
    #[derive(Clone, Copy, PartialEq, Eq)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
    pub struct EprModeDataObject(pub u32): Debug, FromStorage, IntoStorage {
        /// Action
        pub action: u8 @ 24..=31,
        /// Data
        pub data: u8 @ 16..=23,
    }
}

impl Default for EprModeDataObject {
    fn default() -> Self {
        Self::new()
    }
}

impl EprModeDataObject {
    pub fn new() -> Self {
        Self(0)
    }
}

#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Action {
    Enter,
    EnterAcknowledged,
    EnterSucceeded,
    EnterFailed,
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

#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum DataEnterFailed {
    UnknownCause,
    CableNotEprCapable,
    SourceFailedToBecomeVconnSource,
    EprCapableBitNotSetInRdo,
    SourceUnableToEnterEprMode,
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

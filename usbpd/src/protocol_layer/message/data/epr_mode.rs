//! Definitions of EPR mode data message content.
//!
//! Per PD 3.2 Sec. 6.4.10.
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
    /// Reserved value. Treated like any other unexpected `EPR_Mode` message.
    Reserved(u8),
}

impl From<Action> for u8 {
    fn from(value: Action) -> Self {
        match value {
            Action::Enter => 0x01,
            Action::EnterAcknowledged => 0x02,
            Action::EnterSucceeded => 0x03,
            Action::EnterFailed => 0x04,
            Action::Exit => 0x05,
            Action::Reserved(raw) => raw,
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
            // Reserved values must not panic. Partner-supplied frames may carry them.
            raw => Action::Reserved(raw),
        }
    }
}

bitfield! {
    /// The EPR mode data object that encodes an action, as well as corresponding payload data.
    ///
    /// Per PD 3.2 Tab. 6.50.
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

impl PartialEq for DataEnterFailed {
    fn eq(&self, other: &Self) -> bool {
        core::mem::discriminant(self) == core::mem::discriminant(other)
    }
}

impl Eq for DataEnterFailed {}

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
            _ => DataEnterFailed::UnknownCause,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A partner-sent reserved EPRMDO action must map to `Action::Reserved`, not panic.
    #[test]
    fn reserved_action_maps_to_reserved_variant() {
        // 0x00 and 0x06..=0xff are reserved per PD 3.2 Tab. 6.50.
        for raw in [0x00u8].into_iter().chain(0x06..=0xff) {
            let mdo = EprModeDataObject((raw as u32) << 24);
            assert_eq!(mdo.action(), Action::Reserved(raw));
        }
    }

    #[test]
    fn valid_actions_round_trip() {
        for (raw, action) in [
            (0x01, Action::Enter),
            (0x02, Action::EnterAcknowledged),
            (0x03, Action::EnterSucceeded),
            (0x04, Action::EnterFailed),
            (0x05, Action::Exit),
        ] {
            let mdo = EprModeDataObject((raw as u32) << 24);
            assert_eq!(mdo.action(), action);
            assert_eq!(u8::from(action), raw);
        }
    }

    #[test]
    fn reserved_action_stores_raw_value() {
        // The reserved value must survive a setter round-trip without being remapped.
        let raw = 0x7f;
        let mdo = EprModeDataObject::default().with_action(Action::Reserved(raw));
        assert_eq!(mdo.action(), Action::Reserved(raw));
    }

    // Reserved EnterFailed reasons fall back to `UnknownCause` instead of panicking.
    // Per PD 3.2 Tab. 6.50: values 0x06..=0xff are reserved. "Shall not be used" binds
    // the sender; the receiver tolerates them.
    #[test]
    fn data_enter_failed_from_reserved_falls_back_to_unknown_cause() {
        for raw in [0x06u8, 0x7f, 0xff] {
            assert_eq!(DataEnterFailed::from(raw), DataEnterFailed::UnknownCause);
        }
    }

    #[test]
    fn data_enter_failed_from_valid_maps_all_defined_causes() {
        for (raw, expected) in [
            (0x00, DataEnterFailed::UnknownCause),
            (0x01, DataEnterFailed::CableNotEprCapable),
            (0x02, DataEnterFailed::SourceFailedToBecomeVconnSource),
            (0x03, DataEnterFailed::EprCapableBitNotSetInRdo),
            (0x04, DataEnterFailed::SourceUnableToEnterEprMode),
            (0x05, DataEnterFailed::EprCapableBitNotSetInPdo),
        ] {
            assert_eq!(DataEnterFailed::from(raw), expected);
            assert_eq!(u8::from(expected), raw);
        }
    }
}

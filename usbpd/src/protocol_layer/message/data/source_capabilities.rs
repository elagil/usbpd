//! Definitions of source capabilities data message content.
use heapless::Vec;
use proc_bitfield::bitfield;
use uom::si::electric_current::centiampere;
use uom::si::electric_potential::{decivolt, volt};
use uom::si::power::watt;

use super::PdoState;
use crate::_50milliamperes_mod::_50milliamperes;
use crate::_50millivolts_mod::_50millivolts;
use crate::_250milliwatts_mod::_250milliwatts;
use crate::units::{ElectricCurrent, ElectricPotential, Power};

/// Kinds of supplies that can be reported within source capabilities.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Kind {
    /// Fixed voltage supply.
    FixedSupply,
    /// Battery supply.
    Battery,
    /// Variable voltage supply.
    VariableSupply,
    /// Programmable power supply.
    Pps,
    /// Augmented voltage source.
    Avs,
}

/// A power data object holds information about one type of source capability.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum PowerDataObject {
    /// Fixed voltage supply.
    FixedSupply(FixedSupply),
    /// Battery supply.
    Battery(Battery),
    /// Variable voltage supply.
    VariableSupply(VariableSupply),
    /// Augmented supply.
    Augmented(Augmented),
    /// Unknown kind of power data object.
    Unknown(RawPowerDataObject),
}

impl PowerDataObject {
    /// Check if this PDO is zero-padding (used in EPR capabilities messages).
    ///
    /// Per USB PD Spec R3.2 Section 6.5.15.1, if the SPR Capabilities Message
    /// contains fewer than 7 PDOs, the unused Data Objects are zero-filled.
    pub fn is_zero_padding(&self) -> bool {
        match self {
            PowerDataObject::FixedSupply(f) => f.0 == 0,
            PowerDataObject::Battery(b) => b.0 == 0,
            PowerDataObject::VariableSupply(v) => v.0 == 0,
            PowerDataObject::Augmented(a) => match a {
                Augmented::Spr(s) => s.0 == 0,
                Augmented::Epr(e) => e.0 == 0,
                Augmented::Unknown(u) => *u == 0,
            },
            PowerDataObject::Unknown(u) => u.0 == 0,
        }
    }
}

bitfield! {
    /// A raw power data object.
    ///
    /// Used as a fallback for encoding unknown source types.
    #[derive(Clone, Copy, PartialEq, Eq)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
    pub struct RawPowerDataObject(pub u32): Debug, FromStorage, IntoStorage {
        /// The kind of power data object.
        pub kind: u8 @ 30..=31,
    }
}

bitfield! {
    /// A fixed voltage supply PDO.
    #[derive(Clone, Copy, PartialEq, Eq)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
    pub struct FixedSupply(pub u32): Debug, FromStorage, IntoStorage {
        /// Fixed supply
        pub kind: u8 @ 30..=31,
        /// Dual-role power
        pub dual_role_power: bool @ 29,
        /// USB suspend supported
        pub usb_suspend_supported: bool @ 28,
        /// Unconstrained power
        pub unconstrained_power: bool @ 27,
        /// USB communications capable
        pub usb_communications_capable: bool @ 26,
        /// Dual-role data
        pub dual_role_data: bool @ 25,
        /// Unchunked extended messages supported
        pub unchunked_extended_messages_supported: bool @ 24,
        /// EPR mode capable
        pub epr_mode_capable: bool @ 23,
        /// Peak current
        pub peak_current: u8 @ 20..=21,
        /// Voltage in 50 mV units
        pub raw_voltage: u16 @ 10..=19,
        /// Maximum current in 10 mA units
        pub raw_max_current: u16 @ 0..=9,
    }
}

#[allow(clippy::derivable_impls)]
impl Default for FixedSupply {
    fn default() -> Self {
        Self(0)
    }
}

impl FixedSupply {
    pub fn voltage(&self) -> ElectricPotential {
        ElectricPotential::new::<_50millivolts>(self.raw_voltage().into())
    }

    pub fn max_current(&self) -> ElectricCurrent {
        ElectricCurrent::new::<centiampere>(self.raw_max_current().into())
    }
}

bitfield! {
    #[derive(Clone, Copy, PartialEq, Eq)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
    pub struct Battery(pub u32): Debug, FromStorage, IntoStorage {
        /// Battery
        pub kind: u8 @ 30..=31,
        /// Maximum Voltage in 50 mV units
        pub raw_max_voltage: u16 @ 20..=29,
        /// Minimum Voltage in 50 mV units
        pub raw_min_voltage: u16 @ 10..=19,
        /// Maximum Allowable Power in 250 mW units
        pub raw_max_power: u16 @ 0..=9,
    }
}

impl Battery {
    pub fn max_voltage(&self) -> ElectricPotential {
        ElectricPotential::new::<_50millivolts>(self.raw_max_voltage().into())
    }

    pub fn min_voltage(&self) -> ElectricPotential {
        ElectricPotential::new::<_50millivolts>(self.raw_min_voltage().into())
    }

    pub fn max_power(&self) -> Power {
        Power::new::<_250milliwatts>(self.raw_max_power().into())
    }
}

bitfield! {
    #[derive(Clone, Copy, PartialEq, Eq)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
    pub struct VariableSupply(pub u32): Debug, FromStorage, IntoStorage {
        /// Variable supply (non-battery)
        pub kind: u8 @ 30..=31,
        /// Maximum Voltage in 50mV units
        pub raw_max_voltage: u16 @ 20..=29,
        /// Minimum Voltage in 50mV units
        pub raw_min_voltage: u16 @ 10..=19,
        /// Maximum current in 10mA units
        pub raw_max_current: u16 @ 0..=9,
    }
}

impl VariableSupply {
    pub fn max_voltage(&self) -> ElectricPotential {
        ElectricPotential::new::<_50millivolts>(self.raw_max_voltage().into())
    }

    pub fn min_voltage(&self) -> ElectricPotential {
        ElectricPotential::new::<_50millivolts>(self.raw_min_voltage().into())
    }

    pub fn max_current(&self) -> ElectricCurrent {
        ElectricCurrent::new::<centiampere>(self.raw_max_current().into())
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Augmented {
    Spr(SprProgrammablePowerSupply),
    Epr(EprAdjustableVoltageSupply),
    Unknown(u32),
}

bitfield! {
    #[derive(Clone, Copy, PartialEq, Eq)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
    pub struct AugmentedRaw(pub u32): Debug, FromStorage, IntoStorage {
        /// Augmented power data object
        pub kind: u8 @ 30..=31,
        pub supply: u8 @ 28..=29,
        pub power_capabilities: u32 @ 0..=27,
    }
}

bitfield! {
    #[derive(Clone, Copy, PartialEq, Eq)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
    pub struct SprProgrammablePowerSupply(pub u32): Debug, FromStorage, IntoStorage {
        /// Augmented power data object
        pub kind: u8 @ 30..=31,
        /// SPR programmable power supply
        pub supply: u8 @ 28..=29,
        pub pps_power_limited: bool @ 27,
        /// Maximum voltage in 100mV increments
        pub raw_max_voltage: u8 @ 17..=24,
        /// Minimum Voltage in 100mV increments
        pub raw_min_voltage: u8 @ 8..=15,
        /// Maximum Current in 50mA increments
        pub raw_max_current: u8 @ 0..=6,
    }
}

impl Default for SprProgrammablePowerSupply {
    fn default() -> Self {
        Self(0).with_kind(0b11).with_supply(0b00)
    }
}

impl SprProgrammablePowerSupply {
    pub fn max_voltage(&self) -> ElectricPotential {
        ElectricPotential::new::<decivolt>(self.raw_max_voltage().into())
    }

    pub fn min_voltage(&self) -> ElectricPotential {
        ElectricPotential::new::<decivolt>(self.raw_min_voltage().into())
    }

    pub fn max_current(&self) -> ElectricCurrent {
        ElectricCurrent::new::<_50milliamperes>(self.raw_max_current().into())
    }
}

bitfield! {
    #[derive(Clone, Copy, PartialEq, Eq)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
    pub struct EprAdjustableVoltageSupply(pub u32): Debug, FromStorage, IntoStorage {
        /// Augmented power data object
        pub kind: u8 @ 30..=31,
        /// EPR adjustable voltage supply
        pub supply: u8 @ 28..=29,
        pub peak_current: u8 @ 26..=27,
        /// Maximum voltage in 100mV increments
        pub raw_max_voltage: u16 @ 17..=25,
        /// Minimum Voltage in 100mV increments
        pub raw_min_voltage: u8 @ 8..=15,
        /// PDP in 1W increments
        pub raw_pd_power: u8 @ 0..=7,
    }
}

impl EprAdjustableVoltageSupply {
    pub fn max_voltage(&self) -> ElectricPotential {
        ElectricPotential::new::<decivolt>(self.raw_max_voltage().into())
    }

    pub fn min_voltage(&self) -> ElectricPotential {
        ElectricPotential::new::<decivolt>(self.raw_min_voltage().into())
    }

    pub fn pd_power(&self) -> Power {
        Power::new::<watt>(self.raw_pd_power().into())
    }
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SourceCapabilities(pub(crate) Vec<PowerDataObject, 16>);

impl SourceCapabilities {
    pub fn vsafe_5v(&self) -> Option<&FixedSupply> {
        self.0.first().and_then(|supply| {
            if let PowerDataObject::FixedSupply(supply) = supply {
                Some(supply)
            } else {
                None
            }
        })
    }

    pub fn dual_role_power(&self) -> bool {
        self.vsafe_5v().map(FixedSupply::dual_role_power).unwrap_or_default()
    }

    pub fn usb_suspend_supported(&self) -> bool {
        self.vsafe_5v()
            .map(FixedSupply::usb_suspend_supported)
            .unwrap_or_default()
    }

    pub fn unconstrained_power(&self) -> bool {
        self.vsafe_5v()
            .map(FixedSupply::unconstrained_power)
            .unwrap_or_default()
    }

    /// Determine, whether dual-role data is supported by the source.
    pub fn dual_role_data(&self) -> bool {
        self.vsafe_5v().map(FixedSupply::dual_role_data).unwrap_or_default()
    }

    /// Determine, whether unchunked extended messages are supported by the source.
    pub fn unchunked_extended_messages_supported(&self) -> bool {
        self.vsafe_5v()
            .map(FixedSupply::unchunked_extended_messages_supported)
            .unwrap_or_default()
    }

    /// Determine, whether the source is EPR mode capable.
    pub fn epr_mode_capable(&self) -> bool {
        self.vsafe_5v().map(FixedSupply::epr_mode_capable).unwrap_or_default()
    }

    /// Get power data objects (PDOs) from the source.
    pub fn pdos(&self) -> &[PowerDataObject] {
        &self.0
    }

    /// Check if this is an EPR capabilities message (has PDOs at position 8+).
    ///
    /// Per USB PD Spec R3.2 Section 6.5.15.1, EPR Capabilities Messages have
    /// SPR PDOs in positions 1-7 and EPR PDOs starting at position 8.
    pub fn is_epr_capabilities(&self) -> bool {
        self.0.len() > 7
    }

    /// Get SPR PDOs (positions 1-7), excluding zero-padding entries.
    ///
    /// Per USB PD Spec R3.2 Section 6.5.15.1:
    /// - Positions 1-7 contain SPR (A)PDOs
    /// - If fewer than 7 SPR PDOs exist, unused positions are zero-filled
    ///
    /// Returns iterator of (position, PDO) tuples where position is 1-indexed.
    pub fn spr_pdos(&self) -> impl Iterator<Item = (u8, &PowerDataObject)> {
        self.0
            .iter()
            .take(7)
            .enumerate()
            .filter(|(_, pdo)| !pdo.is_zero_padding())
            .map(|(i, pdo)| ((i + 1) as u8, pdo))
    }

    /// Get EPR PDOs (positions 8+).
    ///
    /// Per USB PD Spec R3.2 Section 6.5.15.1:
    /// - EPR (A)PDOs start at Data Object position 8
    /// - Only valid in EPR Capabilities Messages
    ///
    /// Returns iterator of (position, PDO) tuples where position is 1-indexed (8, 9, 10, 11).
    pub fn epr_pdos(&self) -> impl Iterator<Item = (u8, &PowerDataObject)> {
        self.0.iter().skip(7).enumerate().map(|(i, pdo)| ((i + 8) as u8, pdo))
    }

    /// Check if any EPR PDO is in invalid position (1-7).
    ///
    /// Per USB PD Spec R3.2 Section 8.3.3.3.8:
    /// "In EPR Mode and An EPR_Source_Capabilities Message is received with
    /// an EPR (A)PDO in object positions 1... 7" → Hard Reset
    ///
    /// EPR (A)PDOs per spec:
    /// - Fixed Supply PDOs offering 28V, 36V, or 48V (voltage > 20V)
    /// - EPR AVS APDOs
    pub fn has_epr_pdo_in_spr_positions(&self) -> bool {
        let max_spr_voltage = ElectricPotential::new::<volt>(20);
        self.0.iter().take(7).any(|pdo| match pdo {
            // EPR Fixed Supply: voltage > 20V
            PowerDataObject::FixedSupply(f) => f.voltage() > max_spr_voltage,
            // EPR AVS APDO
            PowerDataObject::Augmented(Augmented::Epr(_)) => true,
            _ => false,
        })
    }
}

impl PdoState for SourceCapabilities {
    fn pdo_at_object_position(&self, position: u8) -> Option<Kind> {
        self.pdos()
            .get(position.saturating_sub(1) as usize)
            .and_then(|pdo| match pdo {
                PowerDataObject::FixedSupply(_) => Some(Kind::FixedSupply),
                PowerDataObject::Battery(_) => Some(Kind::Battery),
                PowerDataObject::VariableSupply(_) => Some(Kind::VariableSupply),
                PowerDataObject::Augmented(augmented) => match augmented {
                    Augmented::Spr(_) => Some(Kind::Pps),
                    Augmented::Epr(_) => Some(Kind::Avs),
                    Augmented::Unknown(_) => None,
                },
                PowerDataObject::Unknown(_) => None,
            })
    }
}

impl PdoState for Option<SourceCapabilities> {
    fn pdo_at_object_position(&self, position: u8) -> Option<Kind> {
        self.as_ref().pdo_at_object_position(position)
    }
}

impl PdoState for Option<&SourceCapabilities> {
    fn pdo_at_object_position(&self, position: u8) -> Option<Kind> {
        self.and_then(|s| s.pdo_at_object_position(position))
    }
}

/// Parse a raw PDO into a typed power data object.
///
/// Decodes the PDO type bits and constructs the appropriate variant.
/// Supports SPR (Fixed, Battery, Variable, PPS) and EPR (AVS) PDO types.
pub fn parse_raw_pdo(raw: u32) -> PowerDataObject {
    let pdo = RawPowerDataObject(raw);
    match pdo.kind() {
        0b00 => PowerDataObject::FixedSupply(FixedSupply(raw)),
        0b01 => PowerDataObject::Battery(Battery(raw)),
        0b10 => PowerDataObject::VariableSupply(VariableSupply(raw)),
        0b11 => PowerDataObject::Augmented(match AugmentedRaw(raw).supply() {
            0b00 => Augmented::Spr(SprProgrammablePowerSupply(raw)),
            0b01 => Augmented::Epr(EprAdjustableVoltageSupply(raw)),
            x => {
                warn!("Unknown AugmentedPowerDataObject supply {}", x);
                Augmented::Unknown(raw)
            }
        }),
        _ => {
            warn!("Unknown PowerDataObject kind");
            PowerDataObject::Unknown(pdo)
        }
    }
}

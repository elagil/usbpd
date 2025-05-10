use heapless::Vec;
use proc_bitfield::bitfield;
use uom::si::electric_current::centiampere;
use uom::si::electric_potential::decivolt;
use uom::si::power::watt;

use super::_50milliamperes_mod::_50milliamperes;
use super::_50millivolts_mod::_50millivolts;
use super::_250milliwatts_mod::_250milliwatts;
use super::PdoState;
use super::units::{ElectricCurrent, ElectricPotential, Power};

#[derive(Clone, Copy, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Kind {
    FixedSupply,
    Battery,
    VariableSupply,
    Pps,
    Avs,
}

#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum PowerDataObject {
    FixedSupply(FixedSupply),
    Battery(Battery),
    VariableSupply(VariableSupply),
    Augmented(Augmented),
    Unknown(RawPowerDataObject),
}

bitfield! {
    #[derive(Clone, Copy, PartialEq, Eq)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    pub struct RawPowerDataObject(pub u32): Debug, FromStorage, IntoStorage {
        pub kind: u8 @ 30..=31,
    }
}

bitfield! {
    #[derive(Clone, Copy, PartialEq, Eq)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
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

impl Default for FixedSupply {
    fn default() -> Self {
        Self::new()
    }
}

impl FixedSupply {
    pub fn new() -> Self {
        Self(0)
    }

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
pub enum Augmented {
    Spr(SprProgrammablePowerSupply),
    Epr(EprAdjustableVoltageSupply),
    Unknown(u32),
}

bitfield! {
    #[derive(Clone, Copy, PartialEq, Eq)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
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
        Self::new()
    }
}

impl SprProgrammablePowerSupply {
    pub fn new() -> Self {
        Self(0).with_kind(0b11).with_supply(0b00)
    }

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
pub struct SourceCapabilities(pub(crate) Vec<PowerDataObject, 8>);

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

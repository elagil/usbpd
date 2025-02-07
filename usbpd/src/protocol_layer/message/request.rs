use byteorder::{ByteOrder, LittleEndian};
use proc_bitfield::bitfield;
use uom::si::electric_current::centiampere;
use uom::si::{self};

use super::_20millivolts_mod::_20millivolts;
use super::_250milliwatts_mod::_250milliwatts;
use super::_50milliamperes_mod::_50milliamperes;

bitfield! {
    #[derive(Clone, Copy, PartialEq, Eq)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    pub struct RawDataObject(pub u32): Debug, FromStorage, IntoStorage {
        /// Valid range 1..=14
        pub object_position: u8 @ 28..=31,
    }
}

bitfield! {
    #[derive(Clone, Copy, PartialEq, Eq)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    pub struct FixedVariableSupply(pub u32): Debug, FromStorage, IntoStorage {
        /// Valid range 1..=14
        pub object_position: u8 @ 28..=31,
        pub giveback_flag: bool @ 27,
        pub capability_mismatch: bool @ 26,
        pub usb_communications_capable: bool @ 25,
        pub no_usb_suspend: bool @ 24,
        pub unchunked_extended_messages_supported: bool @ 23,
        pub epr_mode_capable: bool @ 22,
        pub raw_operating_current: u16 @ 10..=19,
        pub raw_max_operating_current: u16 @ 0..=9,
    }
}

impl FixedVariableSupply {
    pub fn to_bytes(self, buf: &mut [u8]) -> usize {
        LittleEndian::write_u32(buf, self.0);
        4
    }

    pub fn operating_current(&self) -> si::u16::ElectricCurrent {
        si::u16::ElectricCurrent::new::<centiampere>(self.raw_operating_current())
    }

    pub fn max_operating_current(&self) -> si::u16::ElectricCurrent {
        si::u16::ElectricCurrent::new::<centiampere>(self.raw_max_operating_current())
    }
}

bitfield! {
    #[derive(Clone, Copy, PartialEq, Eq)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    pub struct Battery(pub u32): Debug, FromStorage, IntoStorage {
        /// Object position (0000b and 1110b…1111b are Reserved and Shall Not be used)
        pub object_position: u8 @ 28..=31,
        /// GiveBackFlag = 0
        pub giveback_flag: bool @ 27,
        /// Capability mismatch
        pub capability_mismatch: bool @ 26,
        /// USB communications capable
        pub usb_communications_capable: bool @ 25,
        /// No USB Suspend
        pub no_usb_suspend: bool @ 24,
        /// Unchunked extended messages supported
        pub unchunked_extended_messages_supported: bool @ 23,
        /// EPR mode capable
        pub epr_mode_capable: bool @ 22,
        /// Operating power in 250mW units
        pub raw_operating_power: u16 @ 10..=19,
        /// Maximum operating power in 250mW units
        pub raw_max_operating_power: u16 @ 0..=9,
    }
}

impl Battery {
    pub fn to_bytes(self, buf: &mut [u8]) {
        LittleEndian::write_u32(buf, self.0);
    }

    pub fn operating_power(&self) -> si::u32::Power {
        si::u32::Power::new::<_250milliwatts>(self.raw_operating_power().into())
    }

    pub fn max_operating_power(&self) -> si::u32::Power {
        si::u32::Power::new::<_250milliwatts>(self.raw_max_operating_power().into())
    }
}

bitfield!(
    #[derive(Clone, Copy, PartialEq, Eq)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    pub struct Pps(pub u32): Debug, FromStorage, IntoStorage {
        /// Object position (0000b and 1110b…1111b are Reserved and Shall Not be used)
        pub object_position: u8 @ 28..=31,
        /// Capability mismatch
        pub capability_mismatch: bool @ 26,
        /// USB communications capable
        pub usb_communications_capable: bool @ 25,
        /// No USB Suspend
        pub no_usb_suspend: bool @ 24,
        /// Unchunked extended messages supported
        pub unchunked_extended_messages_supported: bool @ 23,
        /// EPR mode capable
        pub epr_mode_capable: bool @ 22,
        /// Output voltage in 20mV units
        pub raw_output_voltage: u16 @ 9..=20,
        /// Operating current in 50mA units
        pub raw_operating_current: u16 @ 0..=6,
    }
);

impl Pps {
    pub fn to_bytes(self, buf: &mut [u8]) {
        LittleEndian::write_u32(buf, self.0);
    }

    pub fn output_voltage(&self) -> si::u16::ElectricPotential {
        si::u16::ElectricPotential::new::<_20millivolts>(self.raw_output_voltage())
    }

    pub fn operating_current(&self) -> si::u16::ElectricCurrent {
        si::u16::ElectricCurrent::new::<_50milliamperes>(self.raw_operating_current())
    }
}

bitfield!(
    #[derive(Clone, Copy, PartialEq, Eq)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    pub struct Avs(pub u32): Debug, FromStorage, IntoStorage {
        /// Object position (0000b and 1110b…1111b are Reserved and Shall Not be used)
        pub object_position: u8 @ 28..=31,
        /// Capability mismatch
        pub capability_mismatch: bool @ 26,
        /// USB communications capable
        pub usb_communications_capable: bool @ 25,
        /// No USB Suspend
        pub no_usb_suspend: bool @ 24,
        /// Unchunked extended messages supported
        pub unchunked_extended_messages_supported: bool @ 23,
        /// EPR mode capable
        pub epr_mode_capable: bool @ 22,
        /// Output voltage in 20mV units
        pub raw_output_voltage: u16 @ 9..=20,
        /// Operating current in 50mA units
        pub raw_operating_current: u16 @ 0..=6,
    }
);

impl Avs {
    pub fn to_bytes(self, buf: &mut [u8]) {
        LittleEndian::write_u32(buf, self.0);
    }

    pub fn output_voltage(&self) -> si::u16::ElectricPotential {
        si::u16::ElectricPotential::new::<_20millivolts>(self.raw_output_voltage())
    }

    pub fn operating_current(&self) -> si::u16::ElectricCurrent {
        si::u16::ElectricCurrent::new::<_50milliamperes>(self.raw_operating_current())
    }
}

/// Power requests towards the source.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[allow(unused)] // FIXME: Implement missing request types.
pub enum PowerSource {
    FixedVariableSupply(FixedVariableSupply),
    Battery(Battery),
    Pps(Pps),
    Avs(Avs),
    Unknown(RawDataObject),
}

impl PowerSource {
    pub fn object_position(&self) -> u8 {
        match self {
            PowerSource::FixedVariableSupply(p) => p.object_position(),
            PowerSource::Battery(p) => p.object_position(),
            PowerSource::Pps(p) => p.object_position(),
            PowerSource::Avs(p) => p.object_position(),
            PowerSource::Unknown(p) => p.object_position(),
        }
    }
}

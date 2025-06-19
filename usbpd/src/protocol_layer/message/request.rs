//! Definitions of request message content.
use byteorder::{ByteOrder, LittleEndian};
use proc_bitfield::bitfield;
use uom::si::electric_current::{self, centiampere};
use uom::si::{self};

use super::_20millivolts_mod::_20millivolts;
use super::_50milliamperes_mod::_50milliamperes;
use super::_250milliwatts_mod::_250milliwatts;
use super::pdo::{self, Augmented};
use super::units::{ElectricCurrent, ElectricPotential};

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

    pub fn operating_current(&self) -> ElectricCurrent {
        ElectricCurrent::new::<centiampere>(self.raw_operating_current().into())
    }

    pub fn max_operating_current(&self) -> ElectricCurrent {
        ElectricCurrent::new::<centiampere>(self.raw_max_operating_current().into())
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
    pub fn to_bytes(self, buf: &mut [u8]) -> usize {
        LittleEndian::write_u32(buf, self.0);
        4
    }

    pub fn output_voltage(&self) -> ElectricPotential {
        ElectricPotential::new::<_20millivolts>(self.raw_output_voltage().into())
    }

    pub fn operating_current(&self) -> ElectricCurrent {
        ElectricCurrent::new::<_50milliamperes>(self.raw_operating_current().into())
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

    pub fn output_voltage(&self) -> ElectricPotential {
        ElectricPotential::new::<_20millivolts>(self.raw_output_voltage().into())
    }

    pub fn operating_current(&self) -> ElectricCurrent {
        ElectricCurrent::new::<_50milliamperes>(self.raw_operating_current().into())
    }
}

/// Power requests towards the source.
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[allow(unused)] // FIXME: Implement missing request types.
pub enum PowerSource {
    FixedVariableSupply(FixedVariableSupply),
    Battery(Battery),
    Pps(Pps),
    Avs(Avs),
    Unknown(RawDataObject),
}

/// Errors that can occur during sink requests towards the source.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// A requested (specific) voltage does not exist in the PDOs.
    VoltageMismatch,
}

/// Requestable voltage levels.
#[derive(Debug)]
pub enum VoltageRequest {
    /// The safe 5 V supply.
    Safe5V,
    /// The highest voltage that the source can supply.
    Highest,
    /// A specific voltage.
    Specific(ElectricPotential),
}

/// Requestable currents.
#[derive(Debug)]
pub enum CurrentRequest {
    /// The highest current that the source can supply.
    Highest,
    /// A specific current.
    Specific(ElectricCurrent),
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

    /// Find the highest fixed voltage that can be found in the source capabilities.
    ///
    /// Reports the index of the found PDO, and the fixed supply instance, or `None` if there is no fixed supply PDO.
    fn find_highest_fixed_voltage(source_capabilities: &pdo::SourceCapabilities) -> Option<(usize, &pdo::FixedSupply)> {
        let mut selected_pdo = None;

        for (index, cap) in source_capabilities.pdos().iter().enumerate() {
            if let pdo::PowerDataObject::FixedSupply(fixed_supply) = cap {
                selected_pdo = match selected_pdo {
                    None => Some((index, fixed_supply)),
                    Some(x) => {
                        if fixed_supply.voltage() > x.1.voltage() {
                            Some((index, fixed_supply))
                        } else {
                            selected_pdo
                        }
                    }
                };
            }
        }

        selected_pdo
    }

    /// Find a specific fixed voltage within the source capabilities.
    ///
    /// Reports the index of the found PDO, and the fixed supply instance, or `None` if there is no match to the request.
    fn find_specific_fixed_voltage(
        source_capabilities: &pdo::SourceCapabilities,
        voltage: ElectricPotential,
    ) -> Option<(usize, &pdo::FixedSupply)> {
        for (index, cap) in source_capabilities.pdos().iter().enumerate() {
            if let pdo::PowerDataObject::FixedSupply(fixed_supply) = cap {
                if fixed_supply.voltage() == voltage {
                    return Some((index, fixed_supply));
                }
            }
        }

        None
    }

    /// Find a suitable PDO for a Programmable Power Supply (PPS) by evaluating the provided voltage
    /// request against the source capabilities.
    ///
    /// Reports the index of the found PDO, and the augmented supply instance, or `None` if there is no match to the request.
    fn find_pps_voltage(
        source_capabilities: &pdo::SourceCapabilities,
        voltage: ElectricPotential,
    ) -> Option<(usize, &pdo::Augmented)> {
        for (index, cap) in source_capabilities.pdos().iter().enumerate() {
            let pdo::PowerDataObject::Augmented(augmented) = cap else {
                trace!("Skip non-augmented PDO {:?}", cap);
                continue;
            };

            // Handle EPR when supported.
            match augmented {
                Augmented::Spr(spr) => {
                    if spr.min_voltage() <= voltage && spr.max_voltage() >= voltage {
                        return Some((index, augmented));
                    } else {
                        trace!("Skip PDO, voltage out of range. {:?}", augmented);
                    }
                }
                _ => trace!("Skip PDO, only SPR is supported. {:?}", augmented),
            };
        }

        trace!("Could not find suitable PPS voltage");
        None
    }

    /// Create a new power source request for a fixed supply.
    ///
    /// Finds a suitable PDO by evaluating the provided current and voltage requests against the source capabilities.
    pub fn new_fixed(
        current_request: CurrentRequest,
        voltage_request: VoltageRequest,
        source_capabilities: &pdo::SourceCapabilities,
    ) -> Result<Self, Error> {
        let selected = match voltage_request {
            VoltageRequest::Safe5V => source_capabilities.vsafe_5v().map(|supply| (0, supply)),
            VoltageRequest::Highest => Self::find_highest_fixed_voltage(source_capabilities),
            VoltageRequest::Specific(x) => Self::find_specific_fixed_voltage(source_capabilities, x),
        };

        if selected.is_none() {
            return Err(Error::VoltageMismatch);
        }

        let (index, supply) = selected.unwrap();

        let (current, mismatch) = match current_request {
            CurrentRequest::Highest => (supply.max_current(), false),
            CurrentRequest::Specific(x) => (x, x > supply.max_current()),
        };

        let mut raw_current = current.get::<electric_current::centiampere>() as u16;

        if raw_current > 0x3ff {
            error!("Clamping invalid current: {} mA", 10 * raw_current);
            raw_current = 0x3ff;
        }

        let object_position = index + 1;
        assert!(object_position > 0b0000 && object_position <= 0b1110);

        Ok(Self::FixedVariableSupply(
            FixedVariableSupply(0)
                .with_raw_operating_current(raw_current)
                .with_raw_max_operating_current(raw_current)
                .with_object_position(object_position as u8)
                .with_capability_mismatch(mismatch)
                .with_no_usb_suspend(true)
                .with_usb_communications_capable(true), // FIXME: Make adjustable?
        ))
    }

    /// Create a new power source request for a programmable power supply (PPS).
    ///
    /// Finds a suitable PDO by evaluating the provided current and voltage requests against the source capabilities.
    /// If no PDO is found that matches the request, an error is returned.
    pub fn new_pps(
        current_request: CurrentRequest,
        voltage: ElectricPotential,
        source_capabilities: &pdo::SourceCapabilities,
    ) -> Result<Self, Error> {
        let selected = Self::find_pps_voltage(source_capabilities, voltage);

        if selected.is_none() {
            return Err(Error::VoltageMismatch);
        }

        let (index, supply) = selected.unwrap();
        let max_current = match supply {
            Augmented::Spr(spr) => spr.max_current(),
            _ => unreachable!(),
        };

        let (current, mismatch) = match current_request {
            CurrentRequest::Highest => (max_current, false),
            CurrentRequest::Specific(x) => (x, x > max_current),
        };

        let mut raw_current = current.get::<_50milliamperes>() as u16;

        if raw_current > 0x3ff {
            error!("Clamping invalid current: {} mA", 10 * raw_current);
            raw_current = 0x3ff;
        }

        let raw_voltage = voltage.get::<_20millivolts>() as u16;

        let object_position = index + 1;
        assert!(object_position > 0b0000 && object_position <= 0b1110);

        Ok(Self::Pps(
            Pps(0)
                .with_raw_output_voltage(raw_voltage)
                .with_raw_operating_current(raw_current)
                .with_object_position(object_position as u8)
                .with_capability_mismatch(mismatch)
                .with_no_usb_suspend(true)
                .with_usb_communications_capable(true),
        ))
    }
}

//! Definitions of sink capabilities data message content.
//!
//! Sink capabilities are sent in response to Get_Sink_Cap messages.
//! Per USB PD Spec R3.2 Section 6.4.1.6, the Sink_Capabilities message
//! contains Power Data Objects describing what power levels the sink can operate at.
use heapless::Vec;
use proc_bitfield::bitfield;
use uom::si::electric_current::centiampere;

use crate::_50millivolts_mod::_50millivolts;
use crate::_250milliwatts_mod::_250milliwatts;
use crate::units::{ElectricCurrent, ElectricPotential, Power};

/// Fast Role Swap required USB Type-C current.
/// Per USB PD Spec R3.2 Table 6.17.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum FastRoleSwapCurrent {
    /// Fast Role Swap not supported (default)
    #[default]
    NotSupported = 0b00,
    /// Default USB Power
    DefaultUsbPower = 0b01,
    /// 1.5A @ 5V
    Current1_5A = 0b10,
    /// 3.0A @ 5V
    Current3_0A = 0b11,
}

impl From<u8> for FastRoleSwapCurrent {
    fn from(value: u8) -> Self {
        match value & 0b11 {
            0b00 => Self::NotSupported,
            0b01 => Self::DefaultUsbPower,
            0b10 => Self::Current1_5A,
            0b11 => Self::Current3_0A,
            _ => unreachable!(),
        }
    }
}

bitfield! {
    /// A Sink Fixed Supply PDO.
    ///
    /// Per USB PD Spec R3.2 Table 6.17 (Fixed Supply PDO - Sink).
    /// Different from Source Fixed Supply PDO in bits 28-20.
    #[derive(Clone, Copy, PartialEq, Eq)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
    pub struct SinkFixedSupply(pub u32): Debug, FromStorage, IntoStorage {
        /// Fixed supply (00b)
        pub kind: u8 @ 30..=31,
        /// Dual-Role Power - set if Dual-Role Power supported
        pub dual_role_power: bool @ 29,
        /// Higher Capability - set if sink needs more than vSafe5V for full functionality
        pub higher_capability: bool @ 28,
        /// Unconstrained Power - set if external power source is available
        pub unconstrained_power: bool @ 27,
        /// USB Communications Capable
        pub usb_communications_capable: bool @ 26,
        /// Dual-Role Data
        pub dual_role_data: bool @ 25,
        /// Fast Role Swap required USB Type-C Current (bits 24:23)
        pub raw_fast_role_swap: u8 @ 23..=24,
        /// Reserved - shall be set to zero (bits 22:20)
        pub reserved: u8 @ 20..=22,
        /// Voltage in 50 mV units
        pub raw_voltage: u16 @ 10..=19,
        /// Operational Current in 10 mA units
        pub raw_operational_current: u16 @ 0..=9,
    }
}

#[allow(clippy::derivable_impls)]
impl Default for SinkFixedSupply {
    fn default() -> Self {
        Self(0)
    }
}

impl SinkFixedSupply {
    /// Create a new SinkFixedSupply PDO for the required vSafe5V entry.
    ///
    /// All sinks must include at least one PDO at 5V.
    pub fn new_vsafe5v(operational_current_10ma: u16) -> Self {
        Self::default()
            .with_kind(0b00)
            .with_raw_voltage(100) // 5V = 100 * 50mV
            .with_raw_operational_current(operational_current_10ma)
    }

    /// Create a new SinkFixedSupply PDO at a specific voltage.
    pub fn new(voltage_50mv: u16, operational_current_10ma: u16) -> Self {
        Self::default()
            .with_kind(0b00)
            .with_raw_voltage(voltage_50mv)
            .with_raw_operational_current(operational_current_10ma)
    }

    /// Get the voltage in standard units.
    pub fn voltage(&self) -> ElectricPotential {
        ElectricPotential::new::<_50millivolts>(self.raw_voltage().into())
    }

    /// Get the operational current in standard units.
    pub fn operational_current(&self) -> ElectricCurrent {
        ElectricCurrent::new::<centiampere>(self.raw_operational_current().into())
    }

    /// Get the Fast Role Swap required current.
    pub fn fast_role_swap(&self) -> FastRoleSwapCurrent {
        FastRoleSwapCurrent::from(self.raw_fast_role_swap())
    }
}

bitfield! {
    /// A Sink Battery Supply PDO.
    ///
    /// Per USB PD Spec R3.2 Table 6.19 (Battery Supply PDO - Sink).
    #[derive(Clone, Copy, PartialEq, Eq)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
    pub struct SinkBattery(pub u32): Debug, FromStorage, IntoStorage {
        /// Battery (01b)
        pub kind: u8 @ 30..=31,
        /// Maximum Voltage in 50 mV units
        pub raw_max_voltage: u16 @ 20..=29,
        /// Minimum Voltage in 50 mV units
        pub raw_min_voltage: u16 @ 10..=19,
        /// Operational Power in 250 mW units
        pub raw_operational_power: u16 @ 0..=9,
    }
}

impl SinkBattery {
    /// Create a new SinkBattery PDO.
    pub fn new(min_voltage_50mv: u16, max_voltage_50mv: u16, operational_power_250mw: u16) -> Self {
        Self::default()
            .with_kind(0b01)
            .with_raw_min_voltage(min_voltage_50mv)
            .with_raw_max_voltage(max_voltage_50mv)
            .with_raw_operational_power(operational_power_250mw)
    }

    /// Get the maximum voltage in standard units.
    pub fn max_voltage(&self) -> ElectricPotential {
        ElectricPotential::new::<_50millivolts>(self.raw_max_voltage().into())
    }

    /// Get the minimum voltage in standard units.
    pub fn min_voltage(&self) -> ElectricPotential {
        ElectricPotential::new::<_50millivolts>(self.raw_min_voltage().into())
    }

    /// Get the operational power in standard units.
    pub fn operational_power(&self) -> Power {
        Power::new::<_250milliwatts>(self.raw_operational_power().into())
    }
}

#[allow(clippy::derivable_impls)]
impl Default for SinkBattery {
    fn default() -> Self {
        Self(0)
    }
}

bitfield! {
    /// A Sink Variable Supply PDO.
    ///
    /// Per USB PD Spec R3.2 Table 6.18 (Variable Supply PDO - Sink).
    #[derive(Clone, Copy, PartialEq, Eq)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
    pub struct SinkVariableSupply(pub u32): Debug, FromStorage, IntoStorage {
        /// Variable supply (10b)
        pub kind: u8 @ 30..=31,
        /// Maximum Voltage in 50mV units
        pub raw_max_voltage: u16 @ 20..=29,
        /// Minimum Voltage in 50mV units
        pub raw_min_voltage: u16 @ 10..=19,
        /// Operational current in 10mA units
        pub raw_operational_current: u16 @ 0..=9,
    }
}

impl SinkVariableSupply {
    /// Create a new SinkVariableSupply PDO.
    pub fn new(min_voltage_50mv: u16, max_voltage_50mv: u16, operational_current_10ma: u16) -> Self {
        Self::default()
            .with_kind(0b10)
            .with_raw_min_voltage(min_voltage_50mv)
            .with_raw_max_voltage(max_voltage_50mv)
            .with_raw_operational_current(operational_current_10ma)
    }

    /// Get the maximum voltage in standard units.
    pub fn max_voltage(&self) -> ElectricPotential {
        ElectricPotential::new::<_50millivolts>(self.raw_max_voltage().into())
    }

    /// Get the minimum voltage in standard units.
    pub fn min_voltage(&self) -> ElectricPotential {
        ElectricPotential::new::<_50millivolts>(self.raw_min_voltage().into())
    }

    /// Get the operational current in standard units.
    pub fn operational_current(&self) -> ElectricCurrent {
        ElectricCurrent::new::<centiampere>(self.raw_operational_current().into())
    }
}

#[allow(clippy::derivable_impls)]
impl Default for SinkVariableSupply {
    fn default() -> Self {
        Self(0)
    }
}

/// A Sink Power Data Object.
///
/// Per USB PD Spec R3.2 Section 6.4.1.6, sinks report power levels they can
/// operate at using Fixed, Variable, or Battery PDOs.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum SinkPowerDataObject {
    /// Fixed voltage supply requirement.
    FixedSupply(SinkFixedSupply),
    /// Battery supply requirement.
    Battery(SinkBattery),
    /// Variable voltage supply requirement.
    VariableSupply(SinkVariableSupply),
}

impl SinkPowerDataObject {
    /// Convert the PDO to its raw u32 representation.
    pub fn to_raw(&self) -> u32 {
        match self {
            SinkPowerDataObject::FixedSupply(f) => f.0,
            SinkPowerDataObject::Battery(b) => b.0,
            SinkPowerDataObject::VariableSupply(v) => v.0,
        }
    }
}

/// Sink capabilities message content.
///
/// Contains a list of Power Data Objects describing what power levels the sink
/// can operate at. Per USB PD Spec R3.2 Section 6.4.1.6:
/// - All sinks shall minimally offer one PDO at vSafe5V
/// - Maximum 7 PDOs for SPR mode
#[derive(Clone, Debug, Default)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SinkCapabilities(pub Vec<SinkPowerDataObject, 7>);

impl SinkCapabilities {
    /// Create new sink capabilities with a single vSafe5V PDO.
    ///
    /// This is the minimum required per spec - all sinks must support 5V.
    pub fn new_vsafe5v_only(operational_current_10ma: u16) -> Self {
        let mut pdos = Vec::new();
        pdos.push(SinkPowerDataObject::FixedSupply(SinkFixedSupply::new_vsafe5v(
            operational_current_10ma,
        )))
        .ok();
        Self(pdos)
    }

    /// Create sink capabilities from a list of PDOs.
    pub fn new(pdos: Vec<SinkPowerDataObject, 7>) -> Self {
        Self(pdos)
    }

    /// Get the PDOs.
    pub fn pdos(&self) -> &[SinkPowerDataObject] {
        &self.0
    }

    /// Get the number of PDOs.
    pub fn num_objects(&self) -> u8 {
        self.0.len() as u8
    }

    /// Convert to bytes for transmission.
    ///
    /// Each PDO is 4 bytes, little-endian.
    pub fn to_bytes(&self, buffer: &mut [u8]) -> usize {
        let mut offset = 0;
        for pdo in &self.0 {
            let raw = pdo.to_raw();
            buffer[offset..offset + 4].copy_from_slice(&raw.to_le_bytes());
            offset += 4;
        }
        offset
    }
}

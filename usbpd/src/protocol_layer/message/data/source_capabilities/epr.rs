//! EPR source capability handling.
//!
//! EPR AVS PDO and EPR Capabilities Message evaluation, per PD 3.2 Sec. 6.4.1.4.5 and Sec. 6.5.15.1.

use proc_bitfield::bitfield;
use uom::si::electric_potential::decivolt;
use uom::si::power::watt;

use crate::units::{ElectricPotential, Power};

bitfield! {
    /// EPR adjustable voltage supply (AVS), per PD 3.2 Sec. 6.4.1.4.5.
    #[derive(Clone, Copy, PartialEq, Eq)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
    pub struct EprAdjustableVoltageSupply(pub u32): Debug, FromStorage, IntoStorage {
        /// Augmented power data object
        pub kind: u8 @ 30..=31,
        /// EPR adjustable voltage supply
        pub supply: u8 @ 28..=29,
        /// Peak current capability during the Overload Period
        pub peak_current: u8 @ 26..=27,
        /// Maximum voltage in 100 mV increments
        pub raw_max_voltage: u16 @ 17..=25,
        /// Minimum Voltage in 100 mV increments
        pub raw_min_voltage: u8 @ 8..=15,
        /// PDP in 1 W increments
        pub raw_pd_power: u8 @ 0..=7,
    }
}

impl EprAdjustableVoltageSupply {
    /// The maximum voltage the PPS can be requested to supply
    pub fn max_voltage(&self) -> ElectricPotential {
        ElectricPotential::new::<decivolt>(self.raw_max_voltage().into())
    }

    /// The minimum voltage the PPS can be requested to supply
    pub fn min_voltage(&self) -> ElectricPotential {
        ElectricPotential::new::<decivolt>(self.raw_min_voltage().into())
    }

    /// Rated power the PPS can supply
    pub fn pd_power(&self) -> Power {
        Power::new::<watt>(self.raw_pd_power().into())
    }
}
use uom::si::electric_potential::volt;

use super::super::source_capabilities::{Augmented, PowerDataObject, SourceCapabilities};

impl SourceCapabilities {
    /// Determine, whether the source is EPR mode capable.
    pub fn epr_mode_capable(&self) -> bool {
        self.vsafe_5v()
            .map(super::super::source_capabilities::FixedSupply::epr_mode_capable)
            .unwrap_or_default()
    }

    /// Check if this is an EPR capabilities message (has PDOs at position 8+).
    ///
    /// Per PD 3.2 Sec. 6.5.15.1, EPR Capabilities Messages have
    /// SPR PDOs in positions 1-7 and EPR PDOs starting at position 8.
    pub fn is_epr_capabilities(&self) -> bool {
        self.0.len() > 7
    }

    /// Get EPR PDOs (positions 8+).
    ///
    /// Per PD 3.2 Sec. 6.5.15.1:
    /// - EPR (A)PDOs start at Data Object position 8
    /// - Only valid in EPR Capabilities Messages
    ///
    /// Returns iterator of (position, PDO) tuples where position is 1-indexed (8, 9, 10, 11).
    pub fn epr_pdos(&self) -> impl Iterator<Item = (u8, &PowerDataObject)> {
        self.0.iter().skip(7).enumerate().map(|(i, pdo)| ((i + 8) as u8, pdo))
    }

    /// Check if any EPR PDO is in invalid position (1-7).
    ///
    /// Per PD 3.2 Sec. 8.3.3.3.8:
    /// "In EPR Mode and An EPR_Source_Capabilities Message is received with
    /// an EPR (A)PDO in object positions 1..7" -> Hard Reset
    ///
    /// EPR (A)PDOs per PD 3.2 Sec. 6.5.15.1:
    /// - Fixed Supply PDOs offering 28 V, 36 V, or 48 V (voltage > 20 V)
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

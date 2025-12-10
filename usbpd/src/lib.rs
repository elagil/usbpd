//! # Library for USB PD
//!
//! Modeled after the Universal Serial Bus Power Delivery Specification: USB PD R3.2 v1.1 (2024/10).
//!
//! The library implements:
//! - A policy engine for each supported mode,
//! - the protocol layer, and
//! - the `DevicePolicyManager` trait, which allows a device user application to talk to the policy engine, and control it.
//!
//! ## Currently supported modes
//!
//! - SPR Sink with helpers for requesting
//! - A fixed supply
//! - A Programmable Power Supply (PPS)
//!

#![cfg_attr(not(test), no_std)]
#![warn(missing_docs)]

#[macro_use]
extern crate uom;

// This mod MUST go first, so that the others see its macros.
pub(crate) mod fmt;

pub(crate) mod counters;
pub mod protocol_layer;
pub mod sink;
pub mod timers;

#[cfg(test)]
pub mod dummy;

/// This module defines the CGS (centimeter-gram-second) unit system
/// for use in the USB Power Delivery Protocol layer. These units are
/// defined using the `uom` (units of measurement) library and are
/// expressed as `u32` values for milliamps, millivolts, and microwatts.
pub mod units {
    ISQ!(
        uom::si,
        u32,
        (millimeter, kilogram, second, milliampere, kelvin, mole, candela)
    );
}

/// Defines a unit for electric current in 50 mA steps.
pub mod _50milliamperes_mod {
    unit! {
        system: uom::si;
        quantity: uom::si::electric_current;

        @_50milliamperes: 0.05; "_50mA", "_50milliamps", "_50milliamps";
    }
}

/// Defines a unit for electric potential in 50 mV steps.
pub mod _50millivolts_mod {
    unit! {
        system: uom::si;
        quantity: uom::si::electric_potential;

        @_50millivolts: 0.05; "_50mV", "_50millivolts", "_50millivolts";
    }
}

/// Defines a unit for electric potential in 20 mV steps.
pub mod _20millivolts_mod {
    unit! {
        system: uom::si;
        quantity: uom::si::electric_potential;

        @_20millivolts: 0.02; "_20mV", "_20millivolts", "_20millivolts";
    }
}

/// Defines a unit for electric potential in 25 mV steps.
/// Used by AVS (Adjustable Voltage Supply) per USB PD 3.2 Table 6.26.
pub mod _25millivolts_mod {
    unit! {
        system: uom::si;
        quantity: uom::si::electric_potential;

        @_25millivolts: 0.025; "_25mV", "_25millivolts", "_25millivolts";
    }
}

/// Defines a unit for power in 250 mW steps.
pub mod _250milliwatts_mod {
    unit! {
        system: uom::si;
        quantity: uom::si::power;

        @_250milliwatts: 0.25; "_250mW", "_250milliwatts", "_250milliwatts";
    }
}

use core::fmt::Debug;

/// The power role of the port.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum PowerRole {
    /// The port is a source.
    /// FIXME: Implement
    Source,
    /// The port is a sink.
    Sink,
}

impl From<bool> for PowerRole {
    fn from(value: bool) -> Self {
        match value {
            false => Self::Sink,
            true => Self::Source,
        }
    }
}

impl From<PowerRole> for bool {
    fn from(role: PowerRole) -> bool {
        match role {
            PowerRole::Sink => false,
            PowerRole::Source => true,
        }
    }
}

/// The data role of the port.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum DataRole {
    /// The port is an upstream-facing port.
    Ufp,
    /// The port is a downstream-facing port.
    Dfp,
}

impl From<bool> for DataRole {
    fn from(value: bool) -> Self {
        match value {
            false => Self::Ufp,
            true => Self::Dfp,
        }
    }
}

impl From<DataRole> for bool {
    fn from(role: DataRole) -> bool {
        match role {
            DataRole::Ufp => false,
            DataRole::Dfp => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use uom::si::electric_current::milliampere;
    use uom::si::electric_potential::millivolt;

    use crate::_20millivolts_mod::_20millivolts;
    use crate::units;

    #[test]
    fn test_units() {
        let current = units::ElectricCurrent::new::<milliampere>(123);
        let potential = units::ElectricPotential::new::<millivolt>(4560);

        assert_eq!(current.get::<milliampere>(), 123);
        assert_eq!(potential.get::<millivolt>(), 4560);
        assert_eq!(potential.get::<_20millivolts>(), 228);
    }
}

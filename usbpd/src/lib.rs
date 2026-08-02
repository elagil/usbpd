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

// This mod MUST go first, so that the others see its macros.
pub(crate) mod fmt;

pub(crate) mod counters;
pub mod protocol_layer;
pub mod sink;
pub mod source;
pub mod timers;

#[cfg(test)]
#[allow(missing_docs)] // FIXME: Docs for the dummy?
pub mod dummy;

/// This module defines the unit system for use in the USB Power Delivery
/// Protocol layer. These units are expressed as `u32` values for milliamps,
/// millivolts, and microwatts.
pub mod units {

    /// Electric potential in mV
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    pub struct ElectricPotential(u32);

    impl ElectricPotential {
        /// Create a new electric potential, given in millivolts
        pub fn new_mv(mv: u32) -> Self {
            Self(mv)
        }
        /// Get the electric potential in millivolts
        pub fn get_mv(&self) -> u32 {
            self.0
        }
    }

    /// Electric current in mA
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    pub struct ElectricCurrent(u32);

    impl ElectricCurrent {
        /// Create a new electric current, given in milliamps
        pub fn new_ma(ma: u32) -> Self {
            Self(ma)
        }
        /// Get the electric current in milliamps
        pub fn get_ma(&self) -> u32 {
            self.0
        }
    }

    /// Electric power in mW
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    pub struct Power(u32);

    impl Power {
        /// Create a new electric power, given in milliwatts
        pub fn new_mw(mw: u32) -> Self {
            Self(mw)
        }
        /// Get the electric power in milliwatts
        pub fn get_mw(&self) -> u32 {
            self.0
        }
    }
}

use core::fmt::Debug;

/// The power role of the port.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum PowerRole {
    /// The port is a source.
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

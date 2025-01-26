//! The sink implementation.

use uom::si::u16::{ElectricCurrent, ElectricPotential};
pub mod device_policy_manager;
pub mod policy_engine;

/// Defines the fixed supply that the device policy manager requests from the policy engine.
#[derive(Debug, Clone, Copy)]
pub struct FixedSupplyRequest {
    /// The index of the fixed supply (starting at zero) in the list of provided PDOs.
    pub index: u8,
    /// The requested current.
    pub current: ElectricCurrent,
    /// The requested voltage.
    pub voltage: ElectricPotential,
    /// If true, signals a capability mismatch to the source.
    pub capability_mismatch: bool,
}

/// Types of power source requests that a device can send to the policy engine.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub enum PowerSourceRequest {
    /// Request a fixed supply voltage.
    FixedSupply(FixedSupplyRequest),
}

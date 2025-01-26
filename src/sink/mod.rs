//! The sink implementation.
pub mod device_policy_manager;
pub mod policy_engine;

/// Defines the fixed supply that the device policy manager requests from the policy engine.
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct FixedSupplyRequest {
    /// The index of the fixed supply (starting at zero) in the list of provided PDOs.
    pub index: u8,
    /// The requested current in units of 10 mA.
    pub current_10ma: u16,
}

/// Types of power source requests that a device can send to the policy engine.
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub enum PowerSourceRequest {
    /// Request a fixed supply voltage.
    FixedSupply(FixedSupplyRequest),
}

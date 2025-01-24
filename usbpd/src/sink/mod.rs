//! The sink implementation.
pub mod device_policy_manager;
pub mod policy_engine;

#[derive(Debug, Clone, Copy)]
pub struct FixedSupply {
    pub index: u8,
    pub raw_max_current: u16,
    pub raw_voltage: u16,
}

#[derive(Debug, Clone, Copy)]
pub enum PowerSourceRequest {
    FixedSupply(FixedSupply),
}

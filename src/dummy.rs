//! Implements a dummy driver and timer for testing.

use crate::{
    protocol_layer::message::pdo::{
        AugmentedPowerDataObject, FixedSupply, PowerDataObject, SprProgrammablePowerSupply,
    },
    timers::Timer,
    Driver,
};
use heapless::Vec;

/// A dummy timer for testing.
pub struct DummyTimer {}

impl Timer for DummyTimer {
    async fn after_millis(_milliseconds: u64) {
        // Return immediately.
    }
}

/// A dummy driver for testing.
pub struct DummyDriver {
    rx_vec: Vec<u8, 30>,
}

impl DummyDriver {
    /// Create a new dummy driver.
    pub fn new() -> Self {
        Self { rx_vec: Vec::new() }
    }

    /// Inject received data that can be retrieved later.
    pub fn inject_received_data(&mut self, data: &[u8]) {
        self.rx_vec.extend_from_slice(data).unwrap()
    }
}

impl Driver for DummyDriver {
    async fn receive(&mut self, buffer: &mut [u8]) -> Result<usize, crate::DriverRxError> {
        let length = self.rx_vec.len();
        buffer.copy_from_slice(&self.rx_vec);
        self.rx_vec.clear();

        Ok(length)
    }

    async fn transmit(&mut self, _data: &[u8]) -> Result<(), crate::DriverTxError> {
        // Do nothing.
        Ok(())
    }

    async fn transmit_hard_reset(&mut self) -> Result<(), crate::DriverTxError> {
        // Do nothing.
        Ok(())
    }

    async fn wait_for_vbus(&self) {
        // Do nothing.
    }
}

/// Dummy capabilities to deserialize.
///
/// - Fixed 5 V at 3 A
/// - Fixed 9 V at 3 A
/// - Fixed 15 V at 3 A
/// - Fixed 20 V at 2.25 A
/// - PPS 3.3-11 V at 5 A
/// - PPS 3.3-16 V at 3 A
/// - PPS 3.3-21 V at 2.25 A
pub const DUMMY_CAPABILITIES: [u8; 30] = [
    0xA1, // Header
    0x71, // Header
    0x2c, // +
    0x91, // | Fixed 5V @ 3A
    0x01, // |
    0x08, // +
    0x2c, // +
    0xD1, // |
    0x02, // | Fixed 9V @ 3A
    0x00, // +
    0x2C, // +
    0xB1, // |
    0x04, // | Fixed 15V @ 3A
    0x00, // +
    0xE1, // +
    0x40, // |
    0x06, // | Fixed 20V @ 2.25A
    0x00, // +
    0x64, // +
    0x21, // |
    0xDC, // | PPS 3.3-11V @ 5A
    0xC8, // +
    0x3C, // +
    0x21, // |
    0x40, // | PPS 3.3-16V @ 3A
    0xC9, // +
    0x2D, // +
    0x21, // |
    0xA4, // | PPS 3.3-21V @ 2.25A
    0xC9, // +
];

/// Get dummy source capabilities for testing.
///
/// Corresponds to the `DUMMY_CAPABILITIES` above.
pub fn get_dummy_source_capabilities() -> Vec<PowerDataObject, 8> {
    let mut pdos: Vec<PowerDataObject, 8> = Vec::new();
    pdos.push(PowerDataObject::FixedSupply(
        FixedSupply::new()
            .with_raw_voltage(100)
            .with_raw_max_current(300)
            .with_unconstrained_power(true),
    ))
    .unwrap();

    pdos.push(PowerDataObject::FixedSupply(
        FixedSupply::new().with_raw_voltage(180).with_raw_max_current(300),
    ))
    .unwrap();

    pdos.push(PowerDataObject::FixedSupply(
        FixedSupply::new().with_raw_voltage(300).with_raw_max_current(300),
    ))
    .unwrap();

    pdos.push(PowerDataObject::FixedSupply(
        FixedSupply::new().with_raw_voltage(400).with_raw_max_current(225),
    ))
    .unwrap();

    pdos.push(PowerDataObject::Augmented(AugmentedPowerDataObject::Spr(
        SprProgrammablePowerSupply::new()
            .with_raw_max_current(100)
            .with_raw_min_voltage(33)
            .with_raw_max_voltage(110)
            .with_pps_power_limited(true),
    )))
    .unwrap();

    pdos.push(PowerDataObject::Augmented(AugmentedPowerDataObject::Spr(
        SprProgrammablePowerSupply::new()
            .with_raw_max_current(60)
            .with_raw_min_voltage(33)
            .with_raw_max_voltage(160)
            .with_pps_power_limited(true),
    )))
    .unwrap();

    pdos.push(PowerDataObject::Augmented(AugmentedPowerDataObject::Spr(
        SprProgrammablePowerSupply::new()
            .with_raw_max_current(45)
            .with_raw_min_voltage(33)
            .with_raw_max_voltage(210)
            .with_pps_power_limited(true),
    )))
    .unwrap();

    pdos
}

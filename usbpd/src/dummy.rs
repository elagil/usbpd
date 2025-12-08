//! Implements a dummy driver and timer for testing.
use std::future::pending;
use std::vec::Vec;

use usbpd_traits::Driver;

use crate::protocol_layer::message::data::source_capabilities::{
    Augmented, FixedSupply, PowerDataObject, SprProgrammablePowerSupply,
};
use crate::sink::device_policy_manager::DevicePolicyManager as SinkDevicePolicyManager;
use crate::timers::Timer;

/// A dummy sink device that implements the sink device policy manager.
pub struct DummySinkDevice {}

impl SinkDevicePolicyManager for DummySinkDevice {}

/// A dummy timer for testing.
pub struct DummyTimer {}

impl Timer for DummyTimer {
    async fn after_millis(_milliseconds: u64) {
        // Never time out
        pending().await
    }
}

/// A dummy driver for testing.
pub struct DummyDriver<const N: usize> {
    rx_vec: Vec<heapless::Vec<u8, N>>,
    tx_vec: Vec<heapless::Vec<u8, N>>,
}

impl<const N: usize> DummyDriver<N> {
    /// Create a new dummy driver.
    pub fn new() -> Self {
        Self {
            rx_vec: Vec::new(),
            tx_vec: Vec::new(),
        }
    }

    /// Inject received data that can be retrieved later.
    pub fn inject_received_data(&mut self, data: &[u8]) {
        let mut vec = heapless::Vec::new();
        vec.extend_from_slice(data).unwrap();

        self.rx_vec.push(vec);
    }

    /// Probe data that was transmitted by the stack.
    pub fn probe_transmitted_data(&mut self) -> heapless::Vec<u8, N> {
        self.tx_vec.remove(0)
    }
}

impl<const N: usize> Driver for DummyDriver<N> {
    async fn receive(&mut self, buffer: &mut [u8]) -> Result<usize, usbpd_traits::DriverRxError> {
        let first = self.rx_vec.remove(0);
        let len = first.len();
        buffer[..len].copy_from_slice(&first);

        Ok(len)
    }

    async fn transmit(&mut self, data: &[u8]) -> Result<(), usbpd_traits::DriverTxError> {
        let mut vec = heapless::Vec::new();
        vec.extend_from_slice(data).unwrap();
        self.tx_vec.push(vec);

        Ok(())
    }

    async fn transmit_hard_reset(&mut self) -> Result<(), usbpd_traits::DriverTxError> {
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
pub fn get_dummy_source_capabilities() -> Vec<PowerDataObject> {
    let mut pdos: Vec<PowerDataObject> = Vec::new();
    pdos.push(PowerDataObject::FixedSupply(
        FixedSupply::default()
            .with_raw_voltage(100)
            .with_raw_max_current(300)
            .with_unconstrained_power(true),
    ));

    pdos.push(PowerDataObject::FixedSupply(
        FixedSupply::default().with_raw_voltage(180).with_raw_max_current(300),
    ));

    pdos.push(PowerDataObject::FixedSupply(
        FixedSupply::default().with_raw_voltage(300).with_raw_max_current(300),
    ));

    pdos.push(PowerDataObject::FixedSupply(
        FixedSupply::default().with_raw_voltage(400).with_raw_max_current(225),
    ));

    pdos.push(PowerDataObject::Augmented(Augmented::Spr(
        SprProgrammablePowerSupply::default()
            .with_raw_max_current(100)
            .with_raw_min_voltage(33)
            .with_raw_max_voltage(110)
            .with_pps_power_limited(true),
    )));

    pdos.push(PowerDataObject::Augmented(Augmented::Spr(
        SprProgrammablePowerSupply::default()
            .with_raw_max_current(60)
            .with_raw_min_voltage(33)
            .with_raw_max_voltage(160)
            .with_pps_power_limited(true),
    )));

    pdos.push(PowerDataObject::Augmented(Augmented::Spr(
        SprProgrammablePowerSupply::default()
            .with_raw_max_current(45)
            .with_raw_min_voltage(33)
            .with_raw_max_voltage(210)
            .with_pps_power_limited(true),
    )));

    pdos
}

#[cfg(test)]
mod tests {
    use usbpd_traits::Driver;

    use crate::dummy::DummyDriver;

    #[tokio::test]
    async fn test_receive() {
        let mut driver: DummyDriver<30> = DummyDriver::new();

        let mut injected_data = [0u8; 30];
        injected_data[0] = 123;

        driver.inject_received_data(&injected_data);

        injected_data[1] = 255;
        driver.inject_received_data(&injected_data);

        let mut buf = [0u8; 30];
        driver.receive(&mut buf).await.unwrap();

        assert_eq!(buf[0], 123);
        assert_eq!(buf[1], 0);

        let mut buf = [0u8; 30];
        driver.receive(&mut buf).await.unwrap();

        assert_eq!(buf[0], 123);
        assert_eq!(buf[1], 255);
    }
}

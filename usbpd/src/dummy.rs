//! Implements a dummy driver and timer for testing.
use std::future::pending;
use std::vec::Vec;

use uom::si::power::watt;
use usbpd_traits::Driver;

use crate::protocol_layer::message::data::request::EprRequestDataObject;
use crate::protocol_layer::message::data::source_capabilities::{
    Augmented, FixedSupply, PowerDataObject, SprProgrammablePowerSupply,
};
use crate::sink::device_policy_manager::DevicePolicyManager as SinkDevicePolicyManager;
use crate::timers::Timer;
use crate::units::Power;

/// SPR source capabilities message for testing (includes EPR capable flag).
/// Captured from real hardware: 5V@3A, 9V@3A, 12V@3A, 15V@3A, 20V@5A, PPS 5-21V@5A
pub const DUMMY_SPR_CAPS_EPR_CAPABLE: [u8; 26] = [
    0xA1, 0x61, 0x2C, 0x91, 0x91, 0x0A, 0x2C, 0xD1, 0x12, 0x00, 0x2C, 0xC1, 0x13, 0x00, 0x2C, 0xB1, 0x14, 0x00, 0xF4,
    0x41, 0x16, 0x00, 0x64, 0x32, 0xA4, 0xC9,
];

/// EPR Source Capabilities - Chunk 0 (first 26 bytes of 40-byte message)
/// Contains: 6 SPR PDOs + separator + start of EPR PDO #8 (28V)
pub const DUMMY_EPR_SOURCE_CAPS_CHUNK_0: [u8; 30] = [
    0xB1, 0xFD, 0x28, 0x80, 0x2C, 0x91, 0x91, 0x0A, 0x2C, 0xD1, 0x12, 0x00, 0x2C, 0xC1, 0x13, 0x00, 0x2C, 0xB1, 0x14,
    0x00, 0xF4, 0x41, 0x16, 0x00, 0x64, 0x32, 0xA4, 0xC9, 0x00, 0x00,
];

/// EPR Source Capabilities - Chunk 1 (remaining 14 bytes)
/// Contains: 3 EPR PDOs (28V, 36V, 48V @ 5A = 140W, 180W, 240W)
pub const DUMMY_EPR_SOURCE_CAPS_CHUNK_1: [u8; 18] = [
    0xB1, 0xCF, 0x28, 0x88, 0x00, 0x00, 0xF4, 0xC1, 0x18, 0x00, 0xF4, 0x41, 0x1B, 0x00, 0xF4, 0x01, 0x1F, 0x00,
];

/// Maximum size of a non-extended USB PD message in bytes.
/// Per USB PD spec, this is 2 bytes header + 7 data objects * 4 bytes = 30 bytes.
pub const MAX_DATA_MESSAGE_SIZE: usize = 30;

/// A dummy sink device that implements the sink device policy manager.
pub struct DummySinkDevice {}

impl SinkDevicePolicyManager for DummySinkDevice {}

/// A dummy EPR-capable sink device that requests EPR power.
///
/// This DPM will:
/// 1. Request EPR source capabilities after initial SPR negotiation
/// 2. Select the highest available EPR voltage when EPR caps are available
#[derive(Default)]
pub struct DummySinkEprDevice {
    requested_epr_caps: bool,
}

impl DummySinkEprDevice {
    /// Create a new EPR-capable dummy sink device.
    pub fn new() -> Self {
        Self::default()
    }
}

impl SinkDevicePolicyManager for DummySinkEprDevice {
    async fn get_event(
        &mut self,
        source_capabilities: &crate::protocol_layer::message::data::source_capabilities::SourceCapabilities,
    ) -> crate::sink::device_policy_manager::Event {
        use crate::sink::device_policy_manager::Event;

        eprintln!(
            "DummySinkEprDevice::get_event called, requested_epr_caps={}",
            self.requested_epr_caps
        );
        eprintln!("  PDOs count: {}", source_capabilities.pdos().len());

        // After initial SPR negotiation, enter EPR mode if source is EPR capable
        if !self.requested_epr_caps {
            // Check if source advertises EPR capability in first PDO
            if let Some(first_pdo) = source_capabilities.pdos().first() {
                eprintln!("  First PDO: {:?}", first_pdo);
                if let PowerDataObject::FixedSupply(fixed) = first_pdo {
                    eprintln!("  EPR capable: {}", fixed.epr_mode_capable());
                    if fixed.epr_mode_capable() {
                        self.requested_epr_caps = true;
                        eprintln!("  Returning Event::EnterEprMode");
                        return Event::EnterEprMode(Power::new::<watt>(140)); // Dummy 140W PDP
                    }
                }
            }
        }

        eprintln!("  Returning Event::None");
        Event::None
    }

    async fn request(
        &mut self,
        source_capabilities: &crate::protocol_layer::message::data::source_capabilities::SourceCapabilities,
    ) -> crate::protocol_layer::message::data::request::PowerSource {
        use crate::protocol_layer::message::data::request::{CurrentRequest, PowerSource, VoltageRequest};
        use crate::protocol_layer::message::data::source_capabilities::PowerDataObject;

        eprintln!("DummySinkEprDevice::request called");
        eprintln!("  Total PDOs: {}", source_capabilities.pdos().len());
        eprintln!("  Is EPR capabilities: {}", source_capabilities.is_epr_capabilities());

        // Log SPR PDOs (positions 1-7)
        for (pos, pdo) in source_capabilities.spr_pdos() {
            if let PowerDataObject::FixedSupply(fixed) = pdo {
                eprintln!(
                    "  SPR PDO[{}]: {}V @ {}A",
                    pos,
                    fixed.raw_voltage() / 20,
                    fixed.raw_max_current() / 100
                );
            }
        }

        // Use the spec-compliant epr_pdos() method to get EPR PDOs at positions 8+
        // Per USB PD Spec R3.2 Section 6.5.15.1, EPR PDOs always start at position 8
        let mut first_epr_pdo: Option<(u8, PowerDataObject)> = None;

        for (pos, pdo) in source_capabilities.epr_pdos() {
            // Skip zero-padding (shouldn't happen at position 8+, but be safe)
            if pdo.is_zero_padding() {
                continue;
            }

            if let PowerDataObject::FixedSupply(fixed) = pdo {
                eprintln!(
                    "  EPR PDO[{}]: {}V @ {}A",
                    pos,
                    fixed.raw_voltage() / 20,
                    fixed.raw_max_current() / 100
                );

                if first_epr_pdo.is_none() {
                    first_epr_pdo = Some((pos, *pdo));
                    eprintln!("  Selected first EPR PDO at position {}", pos);
                }
            }
        }

        if let Some((position, pdo)) = first_epr_pdo {
            let voltage = if let PowerDataObject::FixedSupply(fixed) = &pdo {
                fixed.raw_voltage() / 20
            } else {
                0
            };
            eprintln!("  Requesting EPR PDO#{} ({}V)", position, voltage);

            // Create RDO for EPR fixed supply
            // Use FixedVariableSupply structure which matches RDO format for fixed PDOs
            use crate::protocol_layer::message::data::request::FixedVariableSupply;

            let mut rdo = FixedVariableSupply(0);
            rdo = rdo.with_object_position(position); // Already 1-indexed from epr_pdos()
            rdo = rdo.with_usb_communications_capable(true);
            rdo = rdo.with_no_usb_suspend(true);

            // Set current based on the PDO's max current
            if let PowerDataObject::FixedSupply(fixed) = &pdo {
                let max_current = fixed.raw_max_current();
                rdo = rdo.with_raw_operating_current(max_current);
                rdo = rdo.with_raw_max_operating_current(max_current);
            }

            // Create EPR request with RDO and PDO copy
            PowerSource::EprRequest(EprRequestDataObject { rdo: rdo.0, pdo })
        } else {
            eprintln!("  No EPR PDOs found, selecting default 5V");
            // Fall back to default 5V
            PowerSource::new_fixed(CurrentRequest::Highest, VoltageRequest::Safe5V, source_capabilities).unwrap()
        }
    }
}

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

impl<const N: usize> Default for DummyDriver<N> {
    fn default() -> Self {
        Self {
            rx_vec: Vec::new(),
            tx_vec: Vec::new(),
        }
    }
}

impl<const N: usize> DummyDriver<N> {
    /// Create a new dummy driver.
    pub fn new() -> Self {
        Self::default()
    }

    /// Inject received data that can be retrieved later.
    pub fn inject_received_data(&mut self, data: &[u8]) {
        let mut vec = heapless::Vec::new();
        vec.extend_from_slice(data).unwrap();

        self.rx_vec.push(vec);
    }

    /// Probe data that was transmitted by the stack.
    pub fn probe_transmitted_data(&mut self) -> heapless::Vec<u8, N> {
        eprintln!("probe_transmitted_data called, tx_vec len: {}", self.tx_vec.len());
        self.tx_vec.remove(0)
    }

    /// Check if there's transmitted data available to probe.
    pub fn has_transmitted_data(&self) -> bool {
        !self.tx_vec.is_empty()
    }
}

impl<const N: usize> Driver for DummyDriver<N> {
    async fn receive(&mut self, buffer: &mut [u8]) -> Result<usize, usbpd_traits::DriverRxError> {
        // If no data available, wait indefinitely (like real hardware would)
        if self.rx_vec.is_empty() {
            pending().await
        }

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
    vec![
        PowerDataObject::FixedSupply(
            FixedSupply::default()
                .with_raw_voltage(100)
                .with_raw_max_current(300)
                .with_unconstrained_power(true),
        ),
        PowerDataObject::FixedSupply(FixedSupply::default().with_raw_voltage(180).with_raw_max_current(300)),
        PowerDataObject::FixedSupply(FixedSupply::default().with_raw_voltage(300).with_raw_max_current(300)),
        PowerDataObject::FixedSupply(FixedSupply::default().with_raw_voltage(400).with_raw_max_current(225)),
        PowerDataObject::Augmented(Augmented::Spr(
            SprProgrammablePowerSupply::default()
                .with_raw_max_current(100)
                .with_raw_min_voltage(33)
                .with_raw_max_voltage(110)
                .with_pps_power_limited(true),
        )),
        PowerDataObject::Augmented(Augmented::Spr(
            SprProgrammablePowerSupply::default()
                .with_raw_max_current(60)
                .with_raw_min_voltage(33)
                .with_raw_max_voltage(160)
                .with_pps_power_limited(true),
        )),
        PowerDataObject::Augmented(Augmented::Spr(
            SprProgrammablePowerSupply::default()
                .with_raw_max_current(45)
                .with_raw_min_voltage(33)
                .with_raw_max_voltage(210)
                .with_pps_power_limited(true),
        )),
    ]
}

#[cfg(test)]
mod tests {
    use usbpd_traits::Driver;

    use crate::dummy::{DummyDriver, MAX_DATA_MESSAGE_SIZE};

    #[tokio::test]
    async fn test_receive() {
        let mut driver: DummyDriver<MAX_DATA_MESSAGE_SIZE> = DummyDriver::new();

        let mut injected_data = [0u8; MAX_DATA_MESSAGE_SIZE];
        injected_data[0] = 123;

        driver.inject_received_data(&injected_data);

        injected_data[1] = 255;
        driver.inject_received_data(&injected_data);

        let mut buf = [0u8; MAX_DATA_MESSAGE_SIZE];
        driver.receive(&mut buf).await.unwrap();

        assert_eq!(buf[0], 123);
        assert_eq!(buf[1], 0);

        let mut buf = [0u8; 30];
        driver.receive(&mut buf).await.unwrap();

        assert_eq!(buf[0], 123);
        assert_eq!(buf[1], 255);
    }
}

//! EPR-specific test dummies.
//!
//! DPMs and fixture data for exercising EPR negotiation in tests.

use uom::si::power::watt;

use super::{DummyDualRoleDevice, DummySinkDevice};
use crate::protocol_layer::message::data::request::{EprRequestDataObject, PowerSource};
use crate::protocol_layer::message::data::source_capabilities::{PowerDataObject, SourceCapabilities};
use crate::sink::device_policy_manager::{
    DevicePolicyManager as SinkDPM, DrpDevicePolicyManager as SinkDrpDPM, EprDevicePolicyManager as SinkEprDPM,
    Event as SinkEvent, SinkDpm,
};
use crate::source::device_policy_manager::EprDevicePolicyManager as SourceEprDPM;
use crate::units::Power;

/// SPR source capabilities message for testing (includes EPR capable flag).
/// Captured from real hardware: 5 V @ 3 A, 9 V @ 3 A, 12 V @ 3 A, 15 V @ 3 A, 20 V @ 5 A, PPS 5–21 V @ 5 A
pub const DUMMY_SPR_CAPS_EPR_CAPABLE: [u8; 26] = [
    0xA1, 0x61, 0x2C, 0x91, 0x91, 0x0A, 0x2C, 0xD1, 0x12, 0x00, 0x2C, 0xC1, 0x13, 0x00, 0x2C, 0xB1, 0x14, 0x00, 0xF4,
    0x41, 0x16, 0x00, 0x64, 0x32, 0xA4, 0xC9,
];

/// EPR Source Capabilities - Chunk 0 (first 26 bytes of 40-byte message)
/// Contains: 6 SPR PDOs + separator + start of EPR PDO #8 (28 V)
pub const DUMMY_EPR_SOURCE_CAPS_CHUNK_0: [u8; 30] = [
    0xB1, 0xFD, 0x28, 0x80, 0x2C, 0x91, 0x91, 0x0A, 0x2C, 0xD1, 0x12, 0x00, 0x2C, 0xC1, 0x13, 0x00, 0x2C, 0xB1, 0x14,
    0x00, 0xF4, 0x41, 0x16, 0x00, 0x64, 0x32, 0xA4, 0xC9, 0x00, 0x00,
];

/// EPR Source Capabilities - Chunk 1 (remaining 14 bytes)
/// Contains: 3 EPR PDOs (28 V, 36 V, 48 V @ 5 A = 140 W, 180 W, 240 W)
pub const DUMMY_EPR_SOURCE_CAPS_CHUNK_1: [u8; 18] = [
    0xB1, 0xCF, 0x28, 0x88, 0x00, 0x00, 0xF4, 0xC1, 0x18, 0x00, 0xF4, 0x41, 0x1B, 0x00, 0xF4, 0x01, 0x1F, 0x00,
];

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

impl SinkDPM for DummySinkEprDevice {
    async fn get_event(&mut self, source_capabilities: &SourceCapabilities) -> SinkEvent {
        // After initial SPR negotiation, enter EPR mode if source is EPR capable
        if !self.requested_epr_caps {
            // Check if source advertises EPR capability in first PDO
            if let Some(PowerDataObject::FixedSupply(fixed)) = source_capabilities.pdos().first()
                && fixed.epr_mode_capable()
            {
                self.requested_epr_caps = true;
                return SinkEvent::EnterEprMode(Power::new::<watt>(140)); // Dummy 140W PDP
            }
        }

        SinkEvent::None
    }

    async fn request(&mut self, source_capabilities: &SourceCapabilities) -> PowerSource {
        use crate::protocol_layer::message::data::request::{CurrentRequest, VoltageRequest};

        // Use the spec-compliant epr_pdos() method to get EPR PDOs at positions 8+
        // Per PD 3.2 Sec. 6.5.15.1, EPR PDOs always start at position 8
        let first_epr_pdo = source_capabilities
            .epr_pdos()
            .filter(|(_, pdo)| !pdo.is_zero_padding())
            .find(|(_, pdo)| matches!(pdo, PowerDataObject::FixedSupply(_)));

        if let Some((position, pdo)) = first_epr_pdo {
            // Create RDO for EPR fixed supply
            use crate::protocol_layer::message::data::request::FixedVariableSupply;

            let mut rdo = FixedVariableSupply(0)
                .with_object_position(position)
                .with_usb_communications_capable(true)
                .with_no_usb_suspend(true);

            // Set current based on the PDO's max current
            if let PowerDataObject::FixedSupply(fixed) = pdo {
                let max_current = fixed.raw_max_current();
                rdo = rdo
                    .with_raw_operating_current(max_current)
                    .with_raw_max_operating_current(max_current);
            }

            // Create EPR request with RDO and PDO copy
            PowerSource::EprRequest(EprRequestDataObject { rdo: rdo.0, pdo: *pdo })
        } else {
            // Fall back to default 5V
            PowerSource::new_fixed(CurrentRequest::Highest, VoltageRequest::Safe5V, source_capabilities).unwrap()
        }
    }
}

impl SinkDrpDPM for DummySinkEprDevice {}
impl SinkEprDPM for DummySinkEprDevice {}
impl SinkDpm for DummySinkEprDevice {}

/// Empty EPR DPM impls for the SPR-only dummies.
impl SinkEprDPM for DummySinkDevice {}
impl SourceEprDPM for super::DummySourceDevice {}
impl SinkEprDPM for DummyDualRoleDevice {}
impl SourceEprDPM for DummyDualRoleDevice {}

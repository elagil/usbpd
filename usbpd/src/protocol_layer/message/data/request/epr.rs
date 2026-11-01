//! EPR request message handling.
//!
//! EPR_Request and EPR AVS request construction, per PD 3.2 Sec. 6.4.9.

use byteorder::{ByteOrder, LittleEndian};
use proc_bitfield::bitfield;

use super::super::request::{Avs, CurrentRequest, Error, PowerSource};
use super::super::source_capabilities::{Augmented, PowerDataObject, SourceCapabilities};
use crate::_25millivolts_mod::_25millivolts;
use crate::_50milliamperes_mod::_50milliamperes;
use crate::protocol_layer::request::IndexedAugmented;
use crate::units::ElectricPotential;

/// EPR Request containing RDO + copy of requested PDO for source verification.
///
/// Per PD 3.2 Sec. 6.4.9, EPR_Request always has 2 data objects:
/// - The Request Data Object (format depends on PDO type being requested)
/// - Copy of the PDO being requested (for source verification)
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct EprRequestDataObject {
    /// The raw Request Data Object (format depends on PDO type being requested).
    /// This could be a FixedVariableSupply RDO, Avs RDO, or other EPR RDO type.
    pub rdo: u32,
    /// Copy of the PDO being requested (for source verification)
    pub pdo: PowerDataObject,
}

impl EprRequestDataObject {
    /// Get the object position from the RDO
    pub fn object_position(&self) -> u8 {
        RawDataObject(self.rdo).object_position()
    }
}

bitfield! {
    #[derive(Clone, Copy, PartialEq, Eq)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
    pub struct RawDataObject(pub u32): Debug, FromStorage, IntoStorage {
        /// Valid range 1..=14
        pub object_position: u8 @ 28..=31,
    }
}

/// Create a new EPR AVS request.
///
/// Per PD 3.2 Sec. 6.4.9, this creates an EPR_Request with an AVS RDO
/// and a copy of the requested PDO.
pub fn new_epr_avs(
    current_request: CurrentRequest,
    voltage: ElectricPotential,
    source_capabilities: &SourceCapabilities,
) -> Result<PowerSource, Error> {
    let selected = PowerSource::find_augmented_pdo(source_capabilities, voltage);

    let IndexedAugmented(pdo, index) = selected.ok_or(Error::VoltageMismatch)?;

    let Augmented::Epr(avs) = pdo else {
        return Err(Error::VoltageMismatch);
    };

    let max_current = avs.pd_power() / voltage;

    let (current, mismatch) = match current_request {
        CurrentRequest::Highest => (max_current, false),
        CurrentRequest::Specific(x) => (x, x > max_current),
    };

    let mut raw_current = current.get::<_50milliamperes>() as u16;

    if raw_current > 0x7f {
        error!("Clamping invalid AVS current: {} mA", 50 * raw_current);
        raw_current = 0x7f;
    }

    // AVS voltage is in 25 mV units with LSB 2 bits = 0 (effective 100 mV steps)
    // Per PD 3.2 Tab. 6.26: "Output voltage in 25 mV units,
    // the least two significant bits Shall be set to zero"
    let raw_voltage = (voltage.get::<_25millivolts>() as u16) & !0x3;

    let object_position = index + 1;
    assert!(object_position > 0b0000 && object_position <= 0b1110);

    // Build AVS RDO (per PD 3.2 Tab. 6.26)
    let rdo = Avs(0)
        .with_raw_output_voltage(raw_voltage)
        .with_raw_operating_current(raw_current)
        .with_object_position(object_position as u8)
        .with_capability_mismatch(mismatch)
        .with_no_usb_suspend(true)
        .with_usb_communications_capable(true)
        .with_epr_mode_capable(true)
        .0;

    // Copy of the PDO being requested
    let pdo_copy = PowerDataObject::Augmented(Augmented::Epr(*avs));

    Ok(PowerSource::EprRequest(EprRequestDataObject { rdo, pdo: pdo_copy }))
}

/// Serialize the EPR request's two data objects (RDO + PDO copy) to a slice,
/// returning the number of written bytes.
pub(crate) fn to_bytes(epr: &EprRequestDataObject, payload: &mut [u8]) -> usize {
    // Write RDO (raw u32)
    LittleEndian::write_u32(payload, epr.rdo);
    // Write PDO copy as raw u32
    let raw_pdo = epr.pdo.to_raw();
    LittleEndian::write_u32(&mut payload[4..], raw_pdo);
    8
}

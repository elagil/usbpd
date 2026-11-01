//! EPR extended message payloads.
//!
//! Serialization of EPR source/sink capability lists, per PD 3.2 Sec. 6.5.15.

use byteorder::{ByteOrder, LittleEndian};
use heapless::Vec;

use crate::protocol_layer::message::data::sink_capabilities::SinkPowerDataObject;
use crate::protocol_layer::message::data::source_capabilities::PowerDataObject;

/// Serialize an EPR PDO list to a slice, returning the number of written bytes.
///
/// Used by both `EPR_Source_Capabilities` (SPR PDOs including AVS) and `EPR_Sink_Capabilities`.
pub(crate) fn serialize_pdos<PDO>(pdos: &[PDO], payload: &mut [u8]) -> usize
where
    PDO: PdoRaw,
{
    let mut written = 0;
    for pdo in pdos {
        LittleEndian::write_u32(&mut payload[written..written + 4], pdo.pdo_raw());
        written += 4;
    }
    written
}

/// A PDO that can be converted to its raw `u32` representation.
pub(crate) trait PdoRaw {
    fn pdo_raw(&self) -> u32;
}

impl PdoRaw for PowerDataObject {
    fn pdo_raw(&self) -> u32 {
        use crate::protocol_layer::message::data::source_capabilities::Augmented;

        match self {
            PowerDataObject::FixedSupply(p) => p.0,
            PowerDataObject::Battery(p) => p.0,
            PowerDataObject::VariableSupply(p) => p.0,
            PowerDataObject::Augmented(a) => match a {
                Augmented::Spr(p) => p.0,
                Augmented::Epr(p) => p.0,
                Augmented::Unknown(p) => *p,
            },
            PowerDataObject::Unknown(p) => p.0,
        }
    }
}

impl PdoRaw for SinkPowerDataObject {
    fn pdo_raw(&self) -> u32 {
        self.to_raw()
    }
}

/// Parse a payload of 4-byte little-endian PDOs into a heapless `Vec`.
pub(crate) fn parse_pdos<const N: usize>(payload: &[u8]) -> Vec<PowerDataObject, N> {
    payload
        .as_chunks::<4>()
        .0
        .iter()
        .map(|buf| {
            crate::protocol_layer::message::data::source_capabilities::parse_raw_pdo(LittleEndian::read_u32(buf))
        })
        .collect()
}

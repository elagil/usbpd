//! Definitions and implementations of data messages.
//!
//! See [6.4].
use byteorder::{ByteOrder, LittleEndian};
use heapless::Vec;

use crate::protocol_layer::message::Payload;
use crate::protocol_layer::message::header::DataMessageType;

/// PDO State.
///
/// FIXME: Required?
pub trait PdoState {
    /// FIXME: Required?
    fn pdo_at_object_position(&self, position: u8) -> Option<source_capabilities::Kind>;
}

impl PdoState for () {
    fn pdo_at_object_position(&self, _position: u8) -> Option<source_capabilities::Kind> {
        None
    }
}

/// Types of data messages.
///
/// TODO: Add missing types as per [6.4] and [Table 6.6].
#[derive(Debug, Clone)]
#[non_exhaustive]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[allow(unused)]
pub enum Data {
    /// Source capabilities.
    SourceCapabilities(source_capabilities::SourceCapabilities),
    /// Request for a power level from the source.
    Request(request::PowerSource),
    /// Used to enter, acknowledge or exit EPR mode.
    EprMode(epr_mode::EprModeDataObject),
    /// Vendor defined.
    VendorDefined((vendor_defined::VdmHeader, Vec<u32, 7>)), // TODO: Unused, and incomplete
    /// Unknown data type.
    Unknown,
}

impl Data {
    /// Parse a data message.
    pub fn parse_message<P: PdoState>(
        mut message: super::Message,
        message_type: DataMessageType,
        payload: &[u8],
        state: &P,
    ) -> Result<super::Message, super::ParseError> {
        let len = payload.len();
        message.payload = Some(Payload::Data(match message_type {
            DataMessageType::SourceCapabilities => Data::SourceCapabilities(source_capabilities::SourceCapabilities(
                payload
                    .chunks_exact(4)
                    .take(message.header.num_objects())
                    .map(|buf| source_capabilities::RawPowerDataObject(LittleEndian::read_u32(buf)))
                    .map(|pdo| match pdo.kind() {
                        0b00 => {
                            source_capabilities::PowerDataObject::FixedSupply(source_capabilities::FixedSupply(pdo.0))
                        }
                        0b01 => source_capabilities::PowerDataObject::Battery(source_capabilities::Battery(pdo.0)),
                        0b10 => source_capabilities::PowerDataObject::VariableSupply(
                            source_capabilities::VariableSupply(pdo.0),
                        ),
                        0b11 => source_capabilities::PowerDataObject::Augmented({
                            match source_capabilities::AugmentedRaw(pdo.0).supply() {
                                0b00 => source_capabilities::Augmented::Spr(
                                    source_capabilities::SprProgrammablePowerSupply(pdo.0),
                                ),
                                0b01 => source_capabilities::Augmented::Epr(
                                    source_capabilities::EprAdjustableVoltageSupply(pdo.0),
                                ),
                                x => {
                                    warn!("Unknown AugmentedPowerDataObject supply {}", x);
                                    source_capabilities::Augmented::Unknown(pdo.0)
                                }
                            }
                        }),
                        _ => {
                            warn!("Unknown PowerDataObject kind");
                            source_capabilities::PowerDataObject::Unknown(pdo)
                        }
                    })
                    .collect(),
            )),
            DataMessageType::Request => {
                if len != 4 {
                    Data::Unknown
                } else {
                    let raw = request::RawDataObject(LittleEndian::read_u32(payload));
                    if let Some(t) = state.pdo_at_object_position(raw.object_position()) {
                        Data::Request(match t {
                            source_capabilities::Kind::FixedSupply | source_capabilities::Kind::VariableSupply => {
                                request::PowerSource::FixedVariableSupply(request::FixedVariableSupply(raw.0))
                            }
                            source_capabilities::Kind::Battery => {
                                request::PowerSource::Battery(request::Battery(raw.0))
                            }
                            source_capabilities::Kind::Pps => request::PowerSource::Pps(request::Pps(raw.0)),
                            source_capabilities::Kind::Avs => request::PowerSource::Avs(request::Avs(raw.0)),
                        })
                    } else {
                        Data::Request(request::PowerSource::Unknown(raw))
                    }
                }
            }
            DataMessageType::VendorDefined => {
                // Keep for now...
                if len < 4 {
                    Data::Unknown
                } else {
                    let num_obj = message.header.num_objects();
                    trace!("VENDOR: {:?}, {:?}, {:?}", len, num_obj, payload);

                    let header = {
                        let raw = vendor_defined::VdmHeaderRaw(LittleEndian::read_u32(&payload[..4]));
                        match raw.vdm_type() {
                            vendor_defined::VdmType::Unstructured => {
                                vendor_defined::VdmHeader::Unstructured(vendor_defined::VdmHeaderUnstructured(raw.0))
                            }
                            vendor_defined::VdmType::Structured => {
                                vendor_defined::VdmHeader::Structured(vendor_defined::VdmHeaderStructured(raw.0))
                            }
                        }
                    };

                    let data = payload[4..]
                        .chunks_exact(4)
                        .take(7)
                        .map(LittleEndian::read_u32)
                        .collect::<Vec<u32, 7>>();

                    trace!("VDM RX: {:?} {:?}", header, data);
                    // trace!("HEADER: VDM:: TYPE: {:?}, VERS: {:?}", header.vdm_type(),
                    // header.vdm_version()); trace!("HEADER: CMD:: TYPE: {:?}, CMD:
                    // {:?}", header.command_type(), header.command());

                    // Keep for now...
                    // let pkt = payload
                    //     .chunks_exact(1)
                    //     .take(8)
                    //     .map(|i| i[0])
                    //     .collect::<Vec<u8, 8>>();

                    Data::VendorDefined((header, data))
                }
            }
            _ => {
                warn!("Unhandled message type");
                Data::Unknown
            }
        }));

        Ok(message)
    }

    /// Serialize message data to a slice, returning the number of written bytes.
    pub fn to_bytes(&self, payload: &mut [u8]) -> usize {
        match self {
            Self::Unknown => 0,
            Self::SourceCapabilities(_) => unimplemented!(),
            Self::Request(request::PowerSource::FixedVariableSupply(data_object)) => data_object.to_bytes(payload),
            Self::Request(request::PowerSource::Pps(data_object)) => data_object.to_bytes(payload),
            Self::Request(_) => unimplemented!(),
            Self::EprMode(epr_mode::EprModeDataObject(_data_object)) => unimplemented!(),
            Self::VendorDefined(_) => unimplemented!(),
        }
    }
}

// FIXME: add documentation
#[allow(missing_docs)]
pub mod source_capabilities;

// FIXME: add documentation
#[allow(missing_docs)]
pub mod epr_mode;

// FIXME: add documentation
#[allow(missing_docs)]
pub mod vendor_defined;

// FIXME: add documentation
#[allow(missing_docs)]
pub mod request;

//! Definitions and implementations of data messages.
//!
//! See [6.4].
use core::mem::size_of;

use byteorder::{ByteOrder, LittleEndian};
use heapless::Vec;

use crate::protocol_layer::message::Payload;
use crate::protocol_layer::message::header::DataMessageType;

/// Size of a Power Data Object in bytes.
const PDO_SIZE: usize = size_of::<u32>();

// FIXME: add documentation
#[allow(missing_docs)]
pub mod source_capabilities;

pub mod sink_capabilities;

pub mod epr_mode;

// FIXME: add documentation
#[allow(missing_docs)]
pub mod vendor_defined;

// FIXME: add documentation
#[allow(missing_docs)]
pub mod request;

/// Determine the kind of PDO.
pub trait PdoKind {
    /// Determine the kind of PDO at a given object position.
    fn at_object_position(&self, position: u8) -> Option<source_capabilities::Kind>;
}

impl PdoKind for () {
    fn at_object_position(&self, _position: u8) -> Option<source_capabilities::Kind> {
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
    /// Sink capabilities.
    SinkCapabilities(sink_capabilities::SinkCapabilities),
    /// Request for a power level from the source.
    Request(request::PowerSource),
    /// Used to enter, acknowledge or exit EPR mode.
    EprMode(epr_mode::EprModeDataObject),
    /// Vendor defined messages (VDM).
    ///
    /// Currently parsed from the wire but not forwarded to user applications.
    /// TODO: Add DevicePolicyManager callback to allow applications to handle vendor-specific messages.
    VendorDefined((vendor_defined::VdmHeader, Vec<u32, 7>)),
    /// Unknown data type.
    Unknown,
}

impl Data {
    /// Parse a data message.
    pub fn parse_message<P: PdoKind>(
        mut message: super::Message,
        message_type: DataMessageType,
        payload: &[u8],
        state: &P,
    ) -> Result<super::Message, super::ParseError> {
        let len = payload.len();
        message.payload = Some(Payload::Data(match message_type {
            DataMessageType::SourceCapabilities => Data::SourceCapabilities(source_capabilities::SourceCapabilities(
                payload
                    .chunks_exact(PDO_SIZE)
                    .take(message.header.num_objects())
                    .map(|buf| source_capabilities::parse_raw_pdo(LittleEndian::read_u32(buf)))
                    .collect(),
            )),
            DataMessageType::Request => {
                if len != 4 {
                    Data::Unknown
                } else {
                    let raw = request::RawDataObject(LittleEndian::read_u32(payload));
                    if let Some(t) = state.at_object_position(raw.object_position()) {
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
            DataMessageType::EprRequest => {
                let num_objects = message.header.num_objects();
                trace!("EprRequest: num_objects={}, len={}", num_objects, len);
                // Per USB PD 3.x Section 6.4.9, EPR_Request always has 2 data objects
                if num_objects == 2 && len >= 2 * PDO_SIZE {
                    let rdo = LittleEndian::read_u32(&payload[..PDO_SIZE]);
                    let raw_pdo = LittleEndian::read_u32(&payload[PDO_SIZE..2 * PDO_SIZE]);
                    trace!("EprRequest: rdo=0x{:08X}, pdo=0x{:08X}", rdo, raw_pdo);

                    // Parse the PDO (second object) using the standard PDO parser
                    let pdo = source_capabilities::parse_raw_pdo(raw_pdo);

                    Data::Request(request::PowerSource::EprRequest(request::EprRequestDataObject {
                        rdo,
                        pdo,
                    }))
                } else {
                    warn!("Invalid EPR_Request: expected 2 data objects, got {}", num_objects);
                    Data::Unknown
                }
            }
            DataMessageType::EprMode => {
                if len != PDO_SIZE {
                    Data::Unknown
                } else {
                    Data::EprMode(epr_mode::EprModeDataObject(LittleEndian::read_u32(payload)))
                }
            }
            DataMessageType::VendorDefined => {
                // Keep for now...
                if len < PDO_SIZE {
                    Data::Unknown
                } else {
                    let num_obj = message.header.num_objects();
                    trace!("VENDOR: {:?}, {:?}, {:?}", len, num_obj, payload);

                    let header = {
                        let raw = vendor_defined::VdmHeaderRaw(LittleEndian::read_u32(&payload[..PDO_SIZE]));
                        match raw.vdm_type() {
                            vendor_defined::VdmType::Unstructured => {
                                vendor_defined::VdmHeader::Unstructured(vendor_defined::VdmHeaderUnstructured(raw.0))
                            }
                            vendor_defined::VdmType::Structured => {
                                vendor_defined::VdmHeader::Structured(vendor_defined::VdmHeaderStructured(raw.0))
                            }
                        }
                    };

                    let data = payload[PDO_SIZE..]
                        .chunks_exact(PDO_SIZE)
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
            Self::SinkCapabilities(caps) => caps.to_bytes(payload),
            Self::Request(request::PowerSource::FixedVariableSupply(data_object)) => data_object.to_bytes(payload),
            Self::Request(request::PowerSource::Pps(data_object)) => data_object.to_bytes(payload),
            Self::Request(request::PowerSource::Avs(data_object)) => data_object.to_bytes(payload),
            Self::Request(request::PowerSource::EprRequest(epr)) => {
                // Write RDO (raw u32)
                LittleEndian::write_u32(payload, epr.rdo);
                // Write PDO copy as raw u32
                let raw_pdo = match &epr.pdo {
                    source_capabilities::PowerDataObject::FixedSupply(p) => p.0,
                    source_capabilities::PowerDataObject::Battery(p) => p.0,
                    source_capabilities::PowerDataObject::VariableSupply(p) => p.0,
                    source_capabilities::PowerDataObject::Augmented(a) => match a {
                        source_capabilities::Augmented::Spr(p) => p.0,
                        source_capabilities::Augmented::Epr(p) => p.0,
                        source_capabilities::Augmented::Unknown(p) => *p,
                    },
                    source_capabilities::PowerDataObject::Unknown(p) => p.0,
                };
                LittleEndian::write_u32(&mut payload[PDO_SIZE..], raw_pdo);
                2 * PDO_SIZE
            }
            Self::Request(_) => unimplemented!(),
            Self::EprMode(epr_mode::EprModeDataObject(data_object)) => {
                LittleEndian::write_u32(payload, *data_object);
                PDO_SIZE
            }
            Self::VendorDefined(_) => unimplemented!(),
        }
    }
}

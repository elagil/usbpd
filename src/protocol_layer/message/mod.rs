//! Definitions of message content.

// FIXME: add documentation
#[allow(missing_docs)]
pub mod header;

// FIXME: add documentation
#[allow(missing_docs)]
pub mod pdo;

// FIXME: add documentation
#[allow(missing_docs)]
pub mod vdo;

use byteorder::{ByteOrder, LittleEndian};
use header::{DataMessageType, Header, MessageType};
use heapless::Vec;
use pdo::{
    AugmentedPowerDataObject, AugmentedPowerDataObjectRaw, Battery, EPRAdjustableVoltageSupply, FixedSupply,
    PowerDataObject, PowerDataObjectRaw, SPRProgrammablePowerSupply, SourceCapabilities, VariableSupply,
};
use vdo::{VDMHeader, VDMHeaderRaw, VDMHeaderStructured, VDMHeaderUnstructured, VDMType};

use self::pdo::{
    AVSRequestDataObject, BatteryRequestDataObject, FixedVariableRequestDataObject, PPSRequestDataObject,
    PowerDataObjectType, PowerSourceRequest, RawRequestDataObject,
};

/// PDO State.
///
/// FIXME: Required?
pub trait PdoState {
    /// FIXME: Required?
    fn pdo_at_object_position(&self, position: u8) -> Option<PowerDataObjectType>;
}

impl PdoState for () {
    fn pdo_at_object_position(&self, _position: u8) -> Option<PowerDataObjectType> {
        None
    }
}

/// Data that data messages can carry.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[allow(unused)] // FIXME: Implement or remove vendor defined data message support.
pub enum Data {
    /// Source capability data.
    SourceCapabilities(SourceCapabilities),
    /// Request for a power level from the source.
    PowerSourceRequest(PowerSourceRequest),
    /// Vendor defined.
    VendorDefined((VDMHeader, Vec<u32, 7>)), // TODO: Unused, and incomplete
    /// Unknown data type.
    Unknown,
}

impl Data {
    // Serialize message data to a slice, returning the number of written bytes.
    fn to_bytes(&self, payload: &mut [u8]) -> usize {
        match self {
            Self::Unknown => 0,
            Self::SourceCapabilities(_) => unimplemented!(),
            Self::PowerSourceRequest(PowerSourceRequest::FixedSupply(data_object)) => data_object.to_bytes(payload),
            Self::PowerSourceRequest(_) => unimplemented!(),
            Self::VendorDefined(_) => unimplemented!(),
        }
    }
}

/// A USB PD message.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Message {
    /// The message header.
    pub header: Header,
    /// Optional payload data (for data messages).
    pub data: Option<Data>,
}

impl Message {
    /// Create a new message from a message header.
    pub fn new(header: Header) -> Self {
        Self { header, data: None }
    }

    /// Serialize a message to a slice, returning the number of written bytes.
    pub fn to_bytes(&self, buffer: &mut [u8]) -> usize {
        let mut size = self.header.to_bytes(buffer);

        if let Some(data) = self.data.as_ref() {
            size += data.to_bytes(&mut buffer[2..]);
        }

        size
    }

    /// Parse a message from a slice of bytes, with a PDO state.
    ///
    /// FIXME: Is the state required/to spec?
    pub fn from_bytes_with_state<P: PdoState>(data: &[u8], state: &P) -> Self {
        let mut message = Self::new(Header::from_bytes(&data[..2]));
        let payload = &data[2..];

        match message.header.message_type() {
            MessageType::Control(_) => (),
            MessageType::Data(DataMessageType::SourceCapabilities) => {
                message.data = Some(Data::SourceCapabilities(SourceCapabilities(
                    payload
                        .chunks_exact(4)
                        .take(message.header.num_objects())
                        .map(|buf| PowerDataObjectRaw(LittleEndian::read_u32(buf)))
                        .map(|pdo| match pdo.kind() {
                            0b00 => PowerDataObject::FixedSupply(FixedSupply(pdo.0)),
                            0b01 => PowerDataObject::Battery(Battery(pdo.0)),
                            0b10 => PowerDataObject::VariableSupply(VariableSupply(pdo.0)),
                            0b11 => PowerDataObject::AugmentedPowerDataObject({
                                match AugmentedPowerDataObjectRaw(pdo.0).supply() {
                                    0b00 => AugmentedPowerDataObject::SPR(SPRProgrammablePowerSupply(pdo.0)),
                                    0b01 => AugmentedPowerDataObject::EPR(EPRAdjustableVoltageSupply(pdo.0)),
                                    x => {
                                        warn!("Unknown AugmentedPowerDataObject supply {}", x);
                                        AugmentedPowerDataObject::Unknown(pdo.0)
                                    }
                                }
                            }),
                            _ => {
                                warn!("Unknown PowerDataObject kind");
                                PowerDataObject::Unknown(pdo)
                            }
                        })
                        .collect(),
                )));
            }
            MessageType::Data(DataMessageType::Request) => {
                if payload.len() != 4 {
                    message.data = Some(Data::Unknown);
                    return message;
                }
                let raw = RawRequestDataObject(LittleEndian::read_u32(payload));
                if let Some(t) = state.pdo_at_object_position(raw.object_position()) {
                    message.data = Some(Data::PowerSourceRequest(match t {
                        PowerDataObjectType::FixedSupply => {
                            PowerSourceRequest::FixedSupply(FixedVariableRequestDataObject(raw.0))
                        }
                        PowerDataObjectType::Battery => PowerSourceRequest::Battery(BatteryRequestDataObject(raw.0)),
                        PowerDataObjectType::VariableSupply => {
                            PowerSourceRequest::VariableSupply(FixedVariableRequestDataObject(raw.0))
                        }
                        PowerDataObjectType::PPS => PowerSourceRequest::PPS(PPSRequestDataObject(raw.0)),
                        PowerDataObjectType::AVS => PowerSourceRequest::AVS(AVSRequestDataObject(raw.0)),
                    }));
                } else {
                    message.data = Some(Data::PowerSourceRequest(PowerSourceRequest::Unknown(raw)));
                }
            }
            MessageType::Data(DataMessageType::VendorDefined) => {
                // Keep for now...
                let len = payload.len();
                if len < 4 {
                    message.data = Some(Data::Unknown);
                    return message;
                }
                let num_obj = message.header.num_objects();
                trace!("VENDOR: {:?}, {:?}, {:x}", len, num_obj, payload);

                let header = {
                    let raw = VDMHeaderRaw(LittleEndian::read_u32(&payload[..4]));
                    match raw.vdm_type() {
                        VDMType::Unstructured => VDMHeader::Unstructured(VDMHeaderUnstructured(raw.0)),
                        VDMType::Structured => VDMHeader::Structured(VDMHeaderStructured(raw.0)),
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

                message.data = Some(Data::VendorDefined((header, data)));
            }
            MessageType::Data(_) => {
                warn!("Unhandled message type");
                message.data = Some(Data::Unknown);
            }
        };

        message
    }

    /// Parse a message from a slice of bytes.
    pub fn from_bytes(data: &[u8]) -> Self {
        Self::from_bytes_with_state(data, &())
    }
}

pub mod header;
pub mod pdo;
pub mod vdo;

use byteorder::{ByteOrder, LittleEndian};
use defmt::{trace, warn, Format};
use header::{ControlMessageType, DataMessageType, Header, MessageType};
use heapless::Vec;
use pdo::{
    AugmentedPowerDataObject, AugmentedPowerDataObjectRaw, Battery, EPRAdjustableVoltageSupply, FixedSupply,
    PowerDataObject, PowerDataObjectRaw, SPRProgrammablePowerSupply, SourceCapabilities, VariableSupply,
};
use vdo::{VDMHeader, VDMHeaderRaw, VDMHeaderStructured, VDMHeaderUnstructured, VDMType};

use self::pdo::{
    AVSRequestDataObject, BatteryRequestDataObject, FixedVariableRequestDataObject, PPSRequestDataObject,
    PowerDataObjectType, RawRequestDataObject, Request,
};

pub trait PdoState {
    fn pdo_at_object_position(&self, position: u8) -> Option<PowerDataObjectType>;
}

impl PdoState for () {
    fn pdo_at_object_position(&self, _position: u8) -> Option<PowerDataObjectType> {
        None
    }
}

#[derive(Debug, Clone, Format)]
pub enum MessageContent {
    Empty,
    Control(ControlMessageType),
    SourceCapabilities(SourceCapabilities),
    Request(Request),
    VendorDefined((VDMHeader, Vec<u32, 7>)), // TODO: Incomplete
    Unknown,
}

#[derive(Debug, Clone, Format)]
pub struct Message {
    pub header: Header,
    pub content: MessageContent,
}

impl Message {
    pub fn new(header: Header) -> Self {
        Self {
            header,
            content: MessageContent::Empty,
        }
    }

    pub fn parse_with_state<P: PdoState>(data: &[u8], state: &P) -> Self {
        let mut message = Self::new(Header::from_bytes(&data[..2]));
        let payload = &data[2..];

        match message.header.message_type() {
            MessageType::Control(c) => message.content = MessageContent::Control(c),
            MessageType::Data(DataMessageType::SourceCapabilities) => {
                message.content = MessageContent::SourceCapabilities(SourceCapabilities(
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
                ));
            }
            MessageType::Data(DataMessageType::Request) => {
                if payload.len() != 4 {
                    message.content = MessageContent::Unknown;
                    return message;
                }
                let raw = RawRequestDataObject(LittleEndian::read_u32(payload));
                if let Some(t) = state.pdo_at_object_position(raw.object_position()) {
                    message.content = MessageContent::Request(match t {
                        PowerDataObjectType::FixedSupply => Request::FixedSupply(FixedVariableRequestDataObject(raw.0)),
                        PowerDataObjectType::Battery => Request::Battery(BatteryRequestDataObject(raw.0)),
                        PowerDataObjectType::VariableSupply => {
                            Request::VariableSupply(FixedVariableRequestDataObject(raw.0))
                        }
                        PowerDataObjectType::PPS => Request::PPS(PPSRequestDataObject(raw.0)),
                        PowerDataObjectType::AVS => Request::AVS(AVSRequestDataObject(raw.0)),
                    });
                } else {
                    message.content = MessageContent::Request(Request::Unknown(raw));
                }
            }
            MessageType::Data(DataMessageType::VendorDefined) => {
                // Keep for now...
                let len = payload.len();
                if len < 4 {
                    message.content = MessageContent::Unknown;
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

                message.content = MessageContent::VendorDefined((header, data));
            }
            MessageType::Data(_) => {
                warn!("unknown message type");
                message.content = MessageContent::Unknown;
            }
        };

        message
    }

    pub fn parse(data: &[u8]) -> Self {
        Self::parse_with_state(data, &())
    }
}

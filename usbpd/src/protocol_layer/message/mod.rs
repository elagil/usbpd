//! Definitions of message content.

// FIXME: add documentation
#[allow(missing_docs)]
pub mod header;

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

/// This module defines the CGS (centimeter-gram-second) unit system
/// for use in the USB Power Delivery Protocol layer. These units are
/// defined using the `uom` (units of measurement) library and are
/// expressed as `u32` values for milliamps, millivolts, and microwatts.
pub mod units {
    ISQ!(
        uom::si,
        u32,
        (millimeter, kilogram, second, milliampere, kelvin, mole, candela)
    );
}

#[cfg(test)]
mod tests {
    use uom::si::electric_current::milliampere;
    use uom::si::electric_potential::millivolt;

    use super::_20millivolts_mod::_20millivolts;
    use super::units;

    #[test]
    fn test_units() {
        let current = units::ElectricCurrent::new::<milliampere>(123);
        let potential = units::ElectricPotential::new::<millivolt>(4560);

        assert_eq!(current.get::<milliampere>(), 123);
        assert_eq!(potential.get::<millivolt>(), 4560);
        assert_eq!(potential.get::<_20millivolts>(), 228);
    }
}

use byteorder::{ByteOrder, LittleEndian};
use header::{DataMessageType, Header, MessageType};
use heapless::Vec;

pub(super) mod _50milliamperes_mod {
    unit! {
        system: uom::si;
        quantity: uom::si::electric_current;

        @_50milliamperes: 0.05; "_50mA", "_50milliamps", "_50milliamps";
    }
}

pub(super) mod _50millivolts_mod {
    unit! {
        system: uom::si;
        quantity: uom::si::electric_potential;

        @_50millivolts: 0.05; "_50mV", "_50millivolts", "_50millivolts";
    }
}

pub(super) mod _20millivolts_mod {
    unit! {
        system: uom::si;
        quantity: uom::si::electric_potential;

        @_20millivolts: 0.02; "_20mV", "_20millivolts", "_20millivolts";
    }
}

pub(super) mod _250milliwatts_mod {
    unit! {
        system: uom::si;
        quantity: uom::si::power;

        @_250milliwatts: 0.25; "_250mW", "_250milliwatts", "_250milliwatts";
    }
}

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

/// Data that data messages can carry.
///
/// TODO: Add missing types as per [6.4].
#[derive(Debug, Clone)]
#[non_exhaustive]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[allow(unused)]
pub enum Data {
    /// Source capability data.
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
    // Serialize message data to a slice, returning the number of written bytes.
    fn to_bytes(&self, payload: &mut [u8]) -> usize {
        match self {
            Self::Unknown => 0,
            Self::SourceCapabilities(_) => unimplemented!(),
            Self::Request(request::PowerSource::FixedVariableSupply(data_object)) => data_object.to_bytes(payload),
            Self::Request(request::PowerSource::Pps(data_object)) => data_object.to_bytes(payload),
            Self::Request(_) => unimplemented!(),
            Self::EprMode(_) => unimplemented!(),
            Self::VendorDefined(_) => unimplemented!(),
        }
    }
}

/// A USB PD message.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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

    /// Create a new message from a message header and payload data.
    pub fn new_with_data(header: Header, data: Data) -> Self {
        Self {
            header,
            data: Some(data),
        }
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
    pub fn from_bytes_with_state<P: PdoState>(data: &[u8], state: &P) -> Result<Self, ParseError> {
        let header = Header::from_bytes(&data[..2])?;
        let mut message = Self::new(header);
        let payload = &data[2..];

        match message.header.message_type() {
            MessageType::Control(_) => (),
            MessageType::ExtendedControl(_) => (),
            MessageType::Data(DataMessageType::SourceCapabilities) => {
                message.data = Some(Data::SourceCapabilities(source_capabilities::SourceCapabilities(
                    payload
                        .chunks_exact(4)
                        .take(message.header.num_objects())
                        .map(|buf| source_capabilities::RawPowerDataObject(LittleEndian::read_u32(buf)))
                        .map(|pdo| match pdo.kind() {
                            0b00 => source_capabilities::PowerDataObject::FixedSupply(
                                source_capabilities::FixedSupply(pdo.0),
                            ),
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
                )));
            }
            MessageType::Data(DataMessageType::Request) => {
                if payload.len() != 4 {
                    message.data = Some(Data::Unknown);
                    return Ok(message);
                }
                let raw = request::RawDataObject(LittleEndian::read_u32(payload));
                if let Some(t) = state.pdo_at_object_position(raw.object_position()) {
                    message.data = Some(Data::Request(match t {
                        source_capabilities::Kind::FixedSupply | source_capabilities::Kind::VariableSupply => {
                            request::PowerSource::FixedVariableSupply(request::FixedVariableSupply(raw.0))
                        }
                        source_capabilities::Kind::Battery => request::PowerSource::Battery(request::Battery(raw.0)),
                        source_capabilities::Kind::Pps => request::PowerSource::Pps(request::Pps(raw.0)),
                        source_capabilities::Kind::Avs => request::PowerSource::Avs(request::Avs(raw.0)),
                    }));
                } else {
                    message.data = Some(Data::Request(request::PowerSource::Unknown(raw)));
                }
            }
            MessageType::Data(DataMessageType::VendorDefined) => {
                // Keep for now...
                let len = payload.len();
                if len < 4 {
                    message.data = Some(Data::Unknown);
                    return Ok(message);
                }
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

                message.data = Some(Data::VendorDefined((header, data)));
            }
            MessageType::Data(_) => {
                warn!("Unhandled message type");
                message.data = Some(Data::Unknown);
            }
        };

        Ok(message)
    }

    /// Parse a message from a slice of bytes.
    pub fn from_bytes(data: &[u8]) -> Result<Self, ParseError> {
        Self::from_bytes_with_state(data, &())
    }
}

/// Errors that can occur during message/header parsing.
#[derive(thiserror::Error, Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ParseError {
    /// The input buffer has an invalid length.
    /// * `expected` - The expected length.
    /// * `found` - The actual length found.
    #[error("invalid input buffer length (expected {expected:?}, found {found:?})")]
    InvalidLength {
        /// The expected length.
        expected: usize,
        /// The actual length found.
        found: usize,
    },
    /// The specification revision field is not supported.
    #[error("unsupported specification revision `{0}`")]
    UnsupportedSpecificationRevision(u8),
    /// An unknown or reserved message type was encountered.
    #[error("unknown or reserved message type `{0}`")]
    InvalidMessageType(u8),
    /// An unknown or reserved data message type was encountered.
    #[error("unknown or reserved data message type `{0}`")]
    InvalidDataMessageType(u8),
    /// An unknown or reserved control message type was encountered.
    #[error("unknown or reserved control message type `{0}`")]
    InvalidControlMessageType(u8),
    /// Other parsing error with a message.
    #[error("other parse error: {0}")]
    Other(&'static str),
}

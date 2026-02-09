//! Definitions for a USB PD message header.
//!
//! See [6.2.1.1].
use core::convert::TryFrom;

use byteorder::{ByteOrder, LittleEndian};
use proc_bitfield::bitfield;

use crate::counters::Counter;
use crate::protocol_layer::message::ParseError;
use crate::{DataRole, PowerRole};

bitfield! {
    #[derive(Clone, Copy, PartialEq, Eq)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
    /// Definition of the message header. Every message shall start with it.
    pub struct Header(pub u16): Debug, FromStorage, IntoStorage {
        /// Shall be set to zero to indicate a Control Message or Data Message
        /// and set to one to indicate an Extended Message.
        pub extended: bool @ 15,
        /// The number of 32 bit data objects that follow the header.
        pub num_objects: u8 [get usize] @ 12..=14,
        /// A rolling counter, maintained by the originator of the message.
        pub message_id: u8 @ 9..=11,
        /// Indicate the port's present power role (0 -> sink, 1 -> source).
        pub port_power_role: bool [get PowerRole, set PowerRole] @ 8,
        /// The specification revision.
        ///
        /// 00b - Revision 1.0 (deprecated)
        /// 01b - Revision 2.0
        /// 10b - Revision 3.x
        /// 11b - Reserved, shall not be used
        pub spec_revision: u8 [try_get SpecificationRevision, set SpecificationRevision] @ 6..=7,
        /// The port's data role (0 -> UFP, 1 -> DFP).
        pub port_data_role: bool [get DataRole, set DataRole] @ 5,
        /// The type of message being sent. See [6.2.1.1.8] for details
        pub message_type_raw: u8 @ 0..=4,
    }
}

impl Header {
    /// Create a header template with the given attributes.
    pub fn new_template(
        port_data_role: DataRole,
        port_power_role: PowerRole,
        spec_revision: SpecificationRevision,
    ) -> Self {
        Self(0)
            .with_port_data_role(port_data_role)
            .with_port_power_role(port_power_role)
            .with_spec_revision(spec_revision)
    }

    /// Create a new header that follows a template.
    pub fn new(
        template: Self,
        message_id: Counter,
        message_type: MessageType,
        num_objects: u8,
        extended: bool,
    ) -> Self {
        template
            .with_message_id(message_id.value())
            .with_message_type_raw(match message_type {
                MessageType::Control(x) => x as u8,
                MessageType::Extended(x) => x as u8,
                MessageType::Data(x) => x as u8,
            })
            .with_num_objects(num_objects)
            .with_extended(extended)
    }

    /// Create a new control message header.
    pub fn new_control(template: Self, message_id: Counter, message_type: ControlMessageType) -> Self {
        Self::new(template, message_id, MessageType::Control(message_type), 0, false)
    }

    /// Create a new data message header.
    pub fn new_data(template: Self, message_id: Counter, message_type: DataMessageType, num_objects: u8) -> Self {
        Self::new(
            template,
            message_id,
            MessageType::Data(message_type),
            num_objects,
            false,
        )
    }

    /// Create a new extended message header.
    pub fn new_extended(
        template: Self,
        message_id: Counter,
        extended_message_type: ExtendedMessageType,
        num_objects: u8,
    ) -> Self {
        Self::new(
            template,
            message_id,
            MessageType::Extended(extended_message_type),
            num_objects,
            true,
        )
    }

    /// Parse a header from its binary representation.
    pub fn from_bytes(buf: &[u8]) -> Result<Self, ParseError> {
        assert!(buf.len() == 2);

        let header = Header(LittleEndian::read_u16(buf));
        // Validate spec_revision
        header.spec_revision()?;
        Ok(header)
    }

    /// Serialize the header to its binary representation.
    pub fn to_bytes(self, buf: &mut [u8]) -> usize {
        LittleEndian::write_u16(buf, self.0);
        2
    }

    /// Extract the message type that the header encodes.
    pub fn message_type(&self) -> MessageType {
        // Check extended bit first - Extended messages can have data objects (e.g., EPR Source Capabilities)
        if self.extended() {
            MessageType::Extended(self.message_type_raw().into())
        } else if self.num_objects() == 0 {
            MessageType::Control(self.message_type_raw().into())
        } else {
            MessageType::Data(self.message_type_raw().into())
        }
    }
}

/// Specification revieions.
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[allow(non_camel_case_types)]
pub enum SpecificationRevision {
    /// Version 1.0.
    R1_0,
    /// Version 2.0.
    R2_0,
    /// Version 3.x.
    R3_X,
}

impl TryFrom<u8> for SpecificationRevision {
    type Error = ParseError;
    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0b00 => Ok(Self::R1_0),
            0b01 => Ok(Self::R2_0),
            0b10 => Ok(Self::R3_X),
            _ => Err(ParseError::UnsupportedSpecificationRevision(value)),
        }
    }
}

impl From<SpecificationRevision> for u8 {
    fn from(value: SpecificationRevision) -> Self {
        match value {
            SpecificationRevision::R1_0 => 0b00,
            SpecificationRevision::R2_0 => 0b01,
            SpecificationRevision::R3_X => 0b10,
        }
    }
}

/// The type of message that a header encodes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum MessageType {
    /// A control message, as defined in [6.3].
    Control(ControlMessageType),
    /// A data message, as defined in [6.4].
    Data(DataMessageType),
    /// A data message, as defined in [6.5].
    Extended(ExtendedMessageType),
}

/// Types of control messages.
#[allow(missing_docs)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ControlMessageType {
    GoodCRC = 0b0_0001,
    GotoMin = 0b0_0010,
    Accept = 0b0_0011,
    Reject = 0b0_0100,
    Ping = 0b0_0101,
    PsRdy = 0b0_0110,
    GetSourceCap = 0b0_0111,
    GetSinkCap = 0b0_1000,
    DrSwap = 0b0_1001,
    PrSwap = 0b0_1010,
    VconnSwap = 0b0_1011,
    Wait = 0b0_1100,
    SoftReset = 0b0_1101,
    DataReset = 0b0_1110,
    DataResetComplete = 0b0_1111,
    NotSupported = 0b1_0000,
    GetSourceCapExtended = 0b1_0001,
    GetStatus = 0b1_0010,
    FrSwap = 0b1_0011,
    GetPpsStatus = 0b1_0100,
    GetCountryCodes = 0b1_0101,
    GetSinkCapExtended = 0b1_0110,
    GetSourceInfo = 0b1_0111,
    GetRevision = 0b1_1000,
    Reserved,
}

impl From<u8> for ControlMessageType {
    fn from(value: u8) -> Self {
        match value {
            0b0_0001 => Self::GoodCRC,
            0b0_0010 => Self::GotoMin,
            0b0_0011 => Self::Accept,
            0b0_0100 => Self::Reject,
            0b0_0101 => Self::Ping,
            0b0_0110 => Self::PsRdy,
            0b0_0111 => Self::GetSourceCap,
            0b0_1000 => Self::GetSinkCap,
            0b0_1001 => Self::DrSwap,
            0b0_1010 => Self::PrSwap,
            0b0_1011 => Self::VconnSwap,
            0b0_1100 => Self::Wait,
            0b0_1101 => Self::SoftReset,
            0b0_1110 => Self::DataReset,
            0b0_1111 => Self::DataResetComplete,
            0b1_0000 => Self::NotSupported,
            0b1_0001 => Self::GetSourceCapExtended,
            0b1_0010 => Self::GetStatus,
            0b1_0011 => Self::FrSwap,
            0b1_0100 => Self::GetPpsStatus,
            0b1_0101 => Self::GetCountryCodes,
            0b1_0110 => Self::GetSinkCapExtended,
            0b1_0111 => Self::GetSourceInfo,
            0b1_1000 => Self::GetRevision,
            _ => Self::Reserved,
        }
    }
}

/// Types of data messages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[allow(missing_docs)]
pub enum DataMessageType {
    SourceCapabilities = 0b0_0001,
    Request = 0b0_0010,
    Bist = 0b0_0011,
    SinkCapabilities = 0b0_0100,
    BatteryStatus = 0b0_0101,
    Alert = 0b0_0110,
    GetCountryInfo = 0b0_0111,
    EnterUsb = 0b0_1000,
    EprRequest = 0b0_1001,
    EprMode = 0b0_1010,
    SourceInfo = 0b0_1011,
    Revision = 0b0_1100,
    VendorDefined = 0b0_1111,
    Reserved,
}

impl From<u8> for DataMessageType {
    fn from(value: u8) -> Self {
        match value {
            0b0_0001 => Self::SourceCapabilities,
            0b0_0010 => Self::Request,
            0b0_0011 => Self::Bist,
            0b0_0100 => Self::SinkCapabilities,
            0b0_0101 => Self::BatteryStatus,
            0b0_0110 => Self::Alert,
            0b0_0111 => Self::GetCountryInfo,
            0b0_1000 => Self::EnterUsb,
            0b0_1001 => Self::EprRequest,
            0b0_1010 => Self::EprMode,
            0b0_1011 => Self::SourceInfo,
            0b0_1100 => Self::Revision,
            0b0_1111 => Self::VendorDefined,
            _ => Self::Reserved,
        }
    }
}

impl From<u8> for ExtendedMessageType {
    fn from(value: u8) -> Self {
        match value {
            0b0_0001 => Self::SourceCapabilitiesExtended,
            0b0_0010 => Self::Status,
            0b0_0011 => Self::GetBatteryCap,
            0b0_0100 => Self::GetBatteryStatus,
            0b0_0101 => Self::BatteryCapabilities,
            0b0_0110 => Self::GetManufacturerInfo,
            0b0_0111 => Self::ManufacturerInfo,
            0b0_1000 => Self::SecurityRequest,
            0b0_1001 => Self::SecurityResponse,
            0b0_1010 => Self::FirmwareUpdateRequest,
            0b0_1011 => Self::FirmwareUpdateResponse,
            0b0_1100 => Self::PpsStatus,
            0b0_1101 => Self::CountryInfo,
            0b0_1110 => Self::CountryCodes,
            0b0_1111 => Self::SinkCapabilitiesExtended,
            0b1_0000 => Self::ExtendedControl,
            0b1_0001 => Self::EprSourceCapabilities,
            0b1_0010 => Self::EprSinkCapabilities,
            0b1_1110 => Self::VendorDefinedExtended,
            _ => Self::Reserved,
        }
    }
}

/// Types of extended messages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[allow(missing_docs)]
pub enum ExtendedMessageType {
    SourceCapabilitiesExtended = 0b0_0001,
    Status = 0b0_0010,
    GetBatteryCap = 0b0_0011,
    GetBatteryStatus = 0b0_0100,
    BatteryCapabilities = 0b0_0101,
    GetManufacturerInfo = 0b0_0110,
    ManufacturerInfo = 0b0_0111,
    SecurityRequest = 0b0_1000,
    SecurityResponse = 0b0_1001,
    FirmwareUpdateRequest = 0b0_1010,
    FirmwareUpdateResponse = 0b0_1011,
    PpsStatus = 0b0_1100,
    CountryInfo = 0b0_1101,
    CountryCodes = 0b0_1110,
    SinkCapabilitiesExtended = 0b0_1111,
    ExtendedControl = 0b1_0000,
    EprSourceCapabilities = 0b1_0001,
    EprSinkCapabilities = 0b1_0010,
    VendorDefinedExtended = 0b1_1110,
    Reserved,
}

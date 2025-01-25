use byteorder::{ByteOrder, LittleEndian};
use defmt::Format;
use proc_bitfield::bitfield;

#[derive(Clone, Copy, Format, Debug)]
pub enum VendorDataObject {
    VDMHeader(VDMHeader),
    IDHeader(VDMIdentityHeader),
    CertStat(CertStatVDO),
    Product(ProductVDO),
    UFPType(UFPTypeVDO),
}

impl VendorDataObject {
    pub fn to_bytes(&self, buf: &mut [u8]) {
        match self {
            VendorDataObject::VDMHeader(header) => header.to_bytes(buf),
            VendorDataObject::IDHeader(header) => header.to_bytes(buf),
            VendorDataObject::CertStat(header) => header.to_bytes(buf),
            VendorDataObject::Product(header) => header.to_bytes(buf),
            VendorDataObject::UFPType(header) => header.to_bytes(buf),
        }
    }
}

impl From<VendorDataObject> for u32 {
    fn from(value: VendorDataObject) -> Self {
        match value {
            VendorDataObject::VDMHeader(header) => header.into(),
            VendorDataObject::IDHeader(header) => header.into(),
            VendorDataObject::CertStat(header) => header.into(),
            VendorDataObject::Product(header) => header.into(),
            VendorDataObject::UFPType(header) => header.into(),
        }
    }
}

#[derive(Clone, Copy, Format, Debug)]
pub enum VDMCommandType {
    InitiatorREQ,
    ResponderACK,
    ResponderNAK,
    ResponderBSY,
}

impl From<VDMCommandType> for u8 {
    fn from(value: VDMCommandType) -> Self {
        match value {
            VDMCommandType::InitiatorREQ => 0,
            VDMCommandType::ResponderACK => 1,
            VDMCommandType::ResponderNAK => 2,
            VDMCommandType::ResponderBSY => 3,
        }
    }
}

impl From<u8> for VDMCommandType {
    fn from(value: u8) -> Self {
        match value {
            0 => VDMCommandType::InitiatorREQ,
            1 => VDMCommandType::ResponderACK,
            2 => VDMCommandType::ResponderNAK,
            3 => VDMCommandType::ResponderBSY,
            _ => panic!("Cannot convert {:} to VDMCommandType", value), /* Illegal values shall
                                                                         * panic. */
        }
    }
}

#[derive(Clone, Copy, Format)]
pub enum VDMCommand {
    DiscoverIdentity,
    DiscoverSVIDS,
    DiscoverModes,
    EnterMode,
    ExitMode,
    Attention,
    DisplayPortStatus,
    DisplayPortConfig,
}

impl From<VDMCommand> for u8 {
    fn from(value: VDMCommand) -> Self {
        match value {
            VDMCommand::DiscoverIdentity => 0x1,
            VDMCommand::DiscoverSVIDS => 0x2,
            VDMCommand::DiscoverModes => 0x3,
            VDMCommand::EnterMode => 0x4,
            VDMCommand::ExitMode => 0x5,
            VDMCommand::Attention => 0x6,
            VDMCommand::DisplayPortStatus => 0x10,
            VDMCommand::DisplayPortConfig => 0x11,
        }
    }
}

impl From<u8> for VDMCommand {
    fn from(value: u8) -> Self {
        match value {
            0x01 => VDMCommand::DiscoverIdentity,
            0x02 => VDMCommand::DiscoverSVIDS,
            0x03 => VDMCommand::DiscoverModes,
            0x04 => VDMCommand::EnterMode,
            0x05 => VDMCommand::ExitMode,
            0x06 => VDMCommand::Attention,
            0x10 => VDMCommand::DisplayPortStatus,
            0x11 => VDMCommand::DisplayPortConfig,
            // TODO: Find document that explains what 0x12-0x1f are (DP_SID??)
            _ => panic!("Cannot convert {:} to VDMCommand", value), // Illegal values shall panic.
        }
    }
}

#[derive(Clone, Copy, Format)]
pub enum VDMType {
    Unstructured,
    Structured,
}

impl From<VDMType> for bool {
    fn from(value: VDMType) -> Self {
        match value {
            VDMType::Unstructured => false,
            VDMType::Structured => true,
        }
    }
}

impl From<bool> for VDMType {
    fn from(value: bool) -> Self {
        match value {
            true => VDMType::Structured,
            false => VDMType::Unstructured,
        }
    }
}

#[derive(Clone, Copy, Format, Debug)]
pub enum VDMHeader {
    Structured(VDMHeaderStructured),
    Unstructured(VDMHeaderUnstructured),
}

impl VDMHeader {
    pub fn to_bytes(&self, buf: &mut [u8]) {
        match self {
            VDMHeader::Structured(header) => header.to_bytes(buf),
            VDMHeader::Unstructured(header) => header.to_bytes(buf),
        }
    }
}

impl From<VDMHeader> for u32 {
    fn from(value: VDMHeader) -> Self {
        match value {
            VDMHeader::Structured(header) => header.into(),
            VDMHeader::Unstructured(header) => header.into(),
        }
    }
}

impl From<u32> for VDMHeader {
    fn from(value: u32) -> Self {
        let header = VDMHeaderRaw(value);
        match header.vdm_type() {
            VDMType::Structured => VDMHeader::Structured(VDMHeaderStructured(value)),
            VDMType::Unstructured => VDMHeader::Unstructured(VDMHeaderUnstructured(value)),
        }
    }
}

bitfield! {
    #[derive(Clone, Copy, PartialEq, Eq, Format)]
    pub struct VDMHeaderRaw(pub u32): FromStorage, IntoStorage {
        /// VDM Standard or Vendor ID
        pub standard_or_vid: u16 @ 16..=31,
        /// VDM Type (Unstructured/Structured)
        pub vdm_type: bool [VDMType] @ 15,
    }
}

impl VDMHeaderRaw {
    pub fn to_bytes(&self, buf: &mut [u8]) {
        LittleEndian::write_u32(buf, self.0);
    }
}

bitfield! {
    #[derive(Clone, Copy, PartialEq, Eq, Format, Debug)]
    pub struct VDMHeaderStructured(pub u32): FromStorage, IntoStorage {
        /// VDM Standard or Vendor ID
        pub standard_or_vid: u16 @ 16..=31,
        /// VDM Type (Unstructured/Structured)
        pub vdm_type: bool [VDMType] @ 15,
        /// Structured VDM Version
        pub vdm_version_major: u8 @ 13..=14,
        pub vdm_version_minor: u8 @ 11..=12,
        /// Object Position
        pub object_position: u8 @ 8..=10,
        /// Command Type
        pub command_type: u8 [VDMCommandType] @ 6..=7,
        /// Command
        pub command: u8 [VDMCommand] @ 0..=4,
    }
}

impl VDMHeaderStructured {
    pub fn to_bytes(&self, buf: &mut [u8]) {
        LittleEndian::write_u32(buf, self.0);
    }
}

impl Default for VDMHeaderStructured {
    fn default() -> Self {
        VDMHeaderStructured(0).with_vdm_type(VDMType::Structured)
    }
}

#[derive(Clone, Copy, Format)]
pub enum VDMVersionMajor {
    Version10,
    Version2x,
}

impl From<VDMVersionMajor> for u8 {
    fn from(value: VDMVersionMajor) -> Self {
        match value {
            VDMVersionMajor::Version10 => 0b00,
            VDMVersionMajor::Version2x => 0b01,
        }
    }
}

impl From<u8> for VDMVersionMajor {
    fn from(value: u8) -> Self {
        match value {
            0b00 => VDMVersionMajor::Version10,
            0b01 => VDMVersionMajor::Version2x,
            _ => panic!("Cannot convert {:} to VDMVersionMajor", value), /* Illegal values shall
                                                                          * panic. */
        }
    }
}

#[derive(Clone, Copy, Format)]
pub enum VDMVersionMinor {
    Version20,
    Version21,
}

impl From<VDMVersionMinor> for u8 {
    fn from(value: VDMVersionMinor) -> Self {
        match value {
            VDMVersionMinor::Version20 => 0b00,
            VDMVersionMinor::Version21 => 0b01,
        }
    }
}

impl From<u8> for VDMVersionMinor {
    fn from(value: u8) -> Self {
        match value {
            0b00 => VDMVersionMinor::Version20,
            0b01 => VDMVersionMinor::Version21,
            _ => panic!("Cannot convert {:} to VDMVersionMinor", value), /* Illegal values shall
                                                                          * panic. */
        }
    }
}

bitfield! {
    #[derive(Clone, Copy, PartialEq, Eq, Format, Debug)]
    pub struct VDMHeaderUnstructured(pub u32): FromStorage, IntoStorage {
        /// VDM Standard or Vendor ID
        pub standard_or_vid: u16 @ 16..=31,
        /// VDM Type (Unstructured/Structured)
        pub vdm_type: bool [VDMType] @ 15,
        /// Message defined
        pub data: u16 @ 0..=14
    }
}

impl VDMHeaderUnstructured {
    pub fn to_bytes(&self, buf: &mut [u8]) {
        LittleEndian::write_u32(buf, self.0);
    }
}

bitfield! {
    #[derive(Clone, Copy, PartialEq, Eq, Format, Debug)]
    pub struct VDMIdentityHeader(pub u32): FromStorage, IntoStorage {
        /// Host data capable
        pub host_data: bool @ 31,
        /// Device data capable
        pub device_data: bool @ 30,
        /// Product type UFP
        pub product_type_ufp: u8 [SOPProductTypeUFP] @ 27..=29,
        /// Modal Operation Supported
        pub modal_supported: bool @ 26,
        /// Product type DFP
        pub product_type_dfp: u8 [SOPProductTypeDFP] @ 23..=25,
        /// Connector type
        pub connector_type: u8 [ConnectorType] @ 21..=22,
        /// VID
        pub vid: u16 @ 0..=15,
    }
}

impl VDMIdentityHeader {
    pub fn to_bytes(&self, buf: &mut [u8]) {
        LittleEndian::write_u32(buf, self.0);
    }
}

#[derive(Clone, Copy, Format)]
pub enum SOPProductTypeUFP {
    NotUFP,
    PDUSBHub,
    PDUSBPeripheral,
    PSD,
}

impl From<SOPProductTypeUFP> for u8 {
    fn from(value: SOPProductTypeUFP) -> Self {
        match value {
            SOPProductTypeUFP::NotUFP => 0b000,
            SOPProductTypeUFP::PDUSBHub => 0b001,
            SOPProductTypeUFP::PDUSBPeripheral => 0b010,
            SOPProductTypeUFP::PSD => 0b011,
        }
    }
}

impl From<u8> for SOPProductTypeUFP {
    fn from(value: u8) -> Self {
        match value {
            0b000 => SOPProductTypeUFP::NotUFP,
            0b001 => SOPProductTypeUFP::PDUSBHub,
            0b010 => SOPProductTypeUFP::PDUSBPeripheral,
            0b011 => SOPProductTypeUFP::PSD,

            _ => panic!("Cannot convert {:} to SOPProductTypeUFP", value), /* Illegal values
                                                                            * shall panic. */
        }
    }
}

#[derive(Clone, Copy, Format)]
pub enum SOPProductTypeDFP {
    NotDFP,
    PDUSBHub,
    PDUSBHost,
    PowerBrick,
}

impl From<SOPProductTypeDFP> for u8 {
    fn from(value: SOPProductTypeDFP) -> Self {
        match value {
            SOPProductTypeDFP::NotDFP => 0b000,
            SOPProductTypeDFP::PDUSBHub => 0b001,
            SOPProductTypeDFP::PDUSBHost => 0b010,
            SOPProductTypeDFP::PowerBrick => 0b011,
        }
    }
}

impl From<u8> for SOPProductTypeDFP {
    fn from(value: u8) -> Self {
        match value {
            0b000 => SOPProductTypeDFP::NotDFP,
            0b001 => SOPProductTypeDFP::PDUSBHub,
            0b010 => SOPProductTypeDFP::PDUSBHost,
            0b011 => SOPProductTypeDFP::PowerBrick,

            _ => panic!("Cannot convert {:} to SOPProductTypeDFP", value), /* Illegal values
                                                                            * shall panic. */
        }
    }
}

pub enum ConnectorType {
    USBTypeCReceptacle,
    USBTypeCPlug,
}

impl From<ConnectorType> for u8 {
    fn from(value: ConnectorType) -> Self {
        match value {
            ConnectorType::USBTypeCReceptacle => 0b10,
            ConnectorType::USBTypeCPlug => 0b11,
        }
    }
}

impl From<u8> for ConnectorType {
    fn from(value: u8) -> Self {
        match value {
            0b10 => ConnectorType::USBTypeCReceptacle,
            0b11 => ConnectorType::USBTypeCPlug,
            _ => panic!("Cannot convert {:} to ConnectorType", value), /* Illegal values shall
                                                                        * panic. */
        }
    }
}
bitfield! {
    #[derive(Clone, Copy, PartialEq, Eq, Format, Debug)]
    pub struct CertStatVDO(pub u32): FromStorage, IntoStorage {
        /// XID
        pub xid: u32 @ 0..=31,
    }
}

impl CertStatVDO {
    pub fn to_bytes(&self, buf: &mut [u8]) {
        LittleEndian::write_u32(buf, self.0);
    }
}

bitfield! {
    #[derive(Clone, Copy, PartialEq, Eq, Format, Debug)]
    pub struct ProductVDO(pub u32): FromStorage, IntoStorage {
        /// USB Product ID
        pub pid: u16 @ 16..=31,
        pub bcd_device: u16 @ 0..=15,
    }
}

impl ProductVDO {
    pub fn to_bytes(&self, buf: &mut [u8]) {
        LittleEndian::write_u32(buf, self.0);
    }
}

bitfield! {
    #[derive(Clone, Copy, PartialEq, Eq, Format, Debug)]
    pub struct UFPTypeVDO(pub u32): FromStorage, IntoStorage {
        /// USB Product ID
        pub version: u8 @ 29..=31,
        pub device_capability: u8 @ 24..=27,
        pub vconn_power: u8 @ 8..=10,
        pub vconn_required: bool @ 7,
        pub vbus_required: bool @ 6,
        pub alternate_modes: u8 @ 3..=5,
        pub usb_highest_speed: u8 @ 0..=2,
    }
}

impl UFPTypeVDO {
    pub fn to_bytes(&self, buf: &mut [u8]) {
        LittleEndian::write_u32(buf, self.0);
    }
}

#[derive(Clone, Copy, Format)]
pub enum USBHighestSpeed {
    USB20Only,
    USB32Gen1,
    USB32Gen2,
    USB40Gen3,
    USB40Gen4,
}

impl From<USBHighestSpeed> for u8 {
    fn from(value: USBHighestSpeed) -> Self {
        match value {
            USBHighestSpeed::USB20Only => 0b000,
            USBHighestSpeed::USB32Gen1 => 0b001,
            USBHighestSpeed::USB32Gen2 => 0b010,
            USBHighestSpeed::USB40Gen3 => 0b011,
            USBHighestSpeed::USB40Gen4 => 0b100,
        }
    }
}

impl From<u8> for USBHighestSpeed {
    fn from(value: u8) -> Self {
        match value {
            0b000 => USBHighestSpeed::USB20Only,
            0b001 => USBHighestSpeed::USB32Gen1,
            0b010 => USBHighestSpeed::USB32Gen2,
            0b011 => USBHighestSpeed::USB40Gen3,
            0b100 => USBHighestSpeed::USB40Gen4,
            _ => panic!("Cannot convert {:} to USBHighestSpeed", value), /* Illegal values shall
                                                                          * panic. */
        }
    }
}

#[derive(Clone, Copy, Format)]
pub enum VconnPower {
    P1W,
    P1_5W,
    P2W,
    P3W,
    P4W,
    P5W,
    P6W,
}

impl From<VconnPower> for u8 {
    fn from(value: VconnPower) -> Self {
        match value {
            VconnPower::P1W => 0b000,
            VconnPower::P1_5W => 0b001,
            VconnPower::P2W => 0b010,
            VconnPower::P3W => 0b011,
            VconnPower::P4W => 0b100,
            VconnPower::P5W => 0b101,
            VconnPower::P6W => 0b110,
        }
    }
}

impl From<u8> for VconnPower {
    fn from(value: u8) -> Self {
        match value {
            0b000 => VconnPower::P1W,
            0b001 => VconnPower::P1_5W,
            0b010 => VconnPower::P2W,
            0b011 => VconnPower::P3W,
            0b100 => VconnPower::P4W,
            0b101 => VconnPower::P5W,
            0b110 => VconnPower::P6W,
            _ => panic!("Cannot convert {:} to VconnPower", value), // Illegal values shall panic.
        }
    }
}

#[derive(Clone, Copy, Format)]
pub enum UFPVDOVersion {
    Version1_3,
}

impl From<UFPVDOVersion> for u8 {
    fn from(value: UFPVDOVersion) -> Self {
        match value {
            UFPVDOVersion::Version1_3 => 0b011,
        }
    }
}

impl From<u8> for UFPVDOVersion {
    fn from(value: u8) -> Self {
        match value {
            0b011 => UFPVDOVersion::Version1_3,
            _ => panic!("Cannot convert {:} to UFPVDOVersion", value), /* Illegal values shall
                                                                        * panic. */
        }
    }
}

bitfield! {
    #[derive(Clone, Copy, PartialEq, Eq, Format)]
    pub struct DisplayPortCapabilities(pub u32): FromStorage, IntoStorage {
        /// UFP_D Pin Assignments Supported
        pub ufp_d_pin_assignments: u8 @ 16..=23,
        /// DFP_D Pin Assignments Supported
        pub dfp_d_pin_assignments: u8 @ 8..=15,
        /// USB r2.0 Signalling Not Used
        pub usb20_signalling_not_used: bool @ 7,
        /// Receptacle Indication
        pub receptacle_indication: bool @ 6,
        /// Signalling for Transport of DisplayPort Protocol
        pub signaling_rate: u8 @ 2..=5,
        /// Port Capability
        pub capability: u8 @ 0..=1,
    }
}

impl DisplayPortCapabilities {
    pub fn to_bytes(&self, buf: &mut [u8]) {
        LittleEndian::write_u32(buf, self.0);
    }
}

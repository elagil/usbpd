use byteorder::{ByteOrder, LittleEndian};
use proc_bitfield::bitfield;

#[derive(Clone, Copy, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum VendorDataObject {
    VdmHeader(VdmHeader),
    IDHeader(VdmIdentityHeader),
    CertStat(CertStatVDO),
    Product(ProductVDO),
    UFPType(UFPTypeVDO),
}

impl VendorDataObject {
    pub fn to_bytes(self, buf: &mut [u8]) {
        match self {
            VendorDataObject::VdmHeader(header) => header.to_bytes(buf),
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
            VendorDataObject::VdmHeader(header) => header.into(),
            VendorDataObject::IDHeader(header) => header.into(),
            VendorDataObject::CertStat(header) => header.into(),
            VendorDataObject::Product(header) => header.into(),
            VendorDataObject::UFPType(header) => header.into(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum VdmCommandType {
    InitiatorREQ,
    ResponderACK,
    ResponderNAK,
    ResponderBSY,
}

impl From<VdmCommandType> for u8 {
    fn from(value: VdmCommandType) -> Self {
        match value {
            VdmCommandType::InitiatorREQ => 0,
            VdmCommandType::ResponderACK => 1,
            VdmCommandType::ResponderNAK => 2,
            VdmCommandType::ResponderBSY => 3,
        }
    }
}

impl From<u8> for VdmCommandType {
    fn from(value: u8) -> Self {
        match value {
            0 => VdmCommandType::InitiatorREQ,
            1 => VdmCommandType::ResponderACK,
            2 => VdmCommandType::ResponderNAK,
            3 => VdmCommandType::ResponderBSY,
            _ => panic!("Cannot convert {} to VdmCommandType", value), /* Illegal values shall
                                                                        * panic. */
        }
    }
}

#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum VdmCommand {
    DiscoverIdentity,
    DiscoverSVIDS,
    DiscoverModes,
    EnterMode,
    ExitMode,
    Attention,
    DisplayPortStatus,
    DisplayPortConfig,
}

impl From<VdmCommand> for u8 {
    fn from(value: VdmCommand) -> Self {
        match value {
            VdmCommand::DiscoverIdentity => 0x1,
            VdmCommand::DiscoverSVIDS => 0x2,
            VdmCommand::DiscoverModes => 0x3,
            VdmCommand::EnterMode => 0x4,
            VdmCommand::ExitMode => 0x5,
            VdmCommand::Attention => 0x6,
            VdmCommand::DisplayPortStatus => 0x10,
            VdmCommand::DisplayPortConfig => 0x11,
        }
    }
}

impl From<u8> for VdmCommand {
    fn from(value: u8) -> Self {
        match value {
            0x01 => VdmCommand::DiscoverIdentity,
            0x02 => VdmCommand::DiscoverSVIDS,
            0x03 => VdmCommand::DiscoverModes,
            0x04 => VdmCommand::EnterMode,
            0x05 => VdmCommand::ExitMode,
            0x06 => VdmCommand::Attention,
            0x10 => VdmCommand::DisplayPortStatus,
            0x11 => VdmCommand::DisplayPortConfig,
            // TODO: Find document that explains what 0x12-0x1f are (DP_SID??)
            _ => panic!("Cannot convert {} to VdmCommand", value), // Illegal values shall panic.
        }
    }
}

#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum VdmType {
    Unstructured,
    Structured,
}

impl From<VdmType> for bool {
    fn from(value: VdmType) -> Self {
        match value {
            VdmType::Unstructured => false,
            VdmType::Structured => true,
        }
    }
}

impl From<bool> for VdmType {
    fn from(value: bool) -> Self {
        match value {
            true => VdmType::Structured,
            false => VdmType::Unstructured,
        }
    }
}

#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum VdmHeader {
    Structured(VdmHeaderStructured),
    Unstructured(VdmHeaderUnstructured),
}

impl VdmHeader {
    pub fn to_bytes(self, buf: &mut [u8]) {
        match self {
            VdmHeader::Structured(header) => header.to_bytes(buf),
            VdmHeader::Unstructured(header) => header.to_bytes(buf),
        }
    }
}

impl From<VdmHeader> for u32 {
    fn from(value: VdmHeader) -> Self {
        match value {
            VdmHeader::Structured(header) => header.into(),
            VdmHeader::Unstructured(header) => header.into(),
        }
    }
}

impl From<u32> for VdmHeader {
    fn from(value: u32) -> Self {
        let header = VdmHeaderRaw(value);
        match header.vdm_type() {
            VdmType::Structured => VdmHeader::Structured(VdmHeaderStructured(value)),
            VdmType::Unstructured => VdmHeader::Unstructured(VdmHeaderUnstructured(value)),
        }
    }
}

bitfield! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    pub struct VdmHeaderRaw(pub u32): FromStorage, IntoStorage {
        /// VDM Standard or Vendor ID
        pub standard_or_vid: u16 @ 16..=31,
        /// VDM Type (Unstructured/Structured)
        pub vdm_type: bool [VdmType] @ 15,
    }
}

impl VdmHeaderRaw {
    pub fn to_bytes(self, buf: &mut [u8]) {
        LittleEndian::write_u32(buf, self.0);
    }
}

bitfield! {
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
    pub struct VdmHeaderStructured(pub u32): FromStorage, IntoStorage {
        /// VDM Standard or Vendor ID
        pub standard_or_vid: u16 @ 16..=31,
        /// VDM Type (Unstructured/Structured)
        pub vdm_type: bool [VdmType] @ 15,
        /// Structured VDM version, major
        pub vdm_version_major: u8 @ 13..=14,
        /// Structured VDM version, minor
        pub vdm_version_minor: u8 @ 11..=12,
        /// Object Position
        pub object_position: u8 @ 8..=10,
        /// Command Type
        pub command_type: u8 [VdmCommandType] @ 6..=7,
        /// Command
        pub command: u8 [VdmCommand] @ 0..=4,
    }
}

impl VdmHeaderStructured {
    pub fn to_bytes(self, buf: &mut [u8]) {
        LittleEndian::write_u32(buf, self.0);
    }
}

impl Default for VdmHeaderStructured {
    fn default() -> Self {
        VdmHeaderStructured(0).with_vdm_type(VdmType::Structured)
    }
}

#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum VdmVersionMajor {
    Version10,
    Version2x,
}

impl From<VdmVersionMajor> for u8 {
    fn from(value: VdmVersionMajor) -> Self {
        match value {
            VdmVersionMajor::Version10 => 0b00,
            VdmVersionMajor::Version2x => 0b01,
        }
    }
}

impl From<u8> for VdmVersionMajor {
    fn from(value: u8) -> Self {
        match value {
            0b00 => VdmVersionMajor::Version10,
            0b01 => VdmVersionMajor::Version2x,
            _ => panic!("Cannot convert {} to VdmVersionMajor", value), // Illegal values shall panic.
        }
    }
}

#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum VdmVersionMinor {
    Version20,
    Version21,
}

impl From<VdmVersionMinor> for u8 {
    fn from(value: VdmVersionMinor) -> Self {
        match value {
            VdmVersionMinor::Version20 => 0b00,
            VdmVersionMinor::Version21 => 0b01,
        }
    }
}

impl From<u8> for VdmVersionMinor {
    fn from(value: u8) -> Self {
        match value {
            0b00 => VdmVersionMinor::Version20,
            0b01 => VdmVersionMinor::Version21,
            _ => panic!("Cannot convert {} to VdmVersionMinor", value), /* Illegal values shall
                                                                         * panic. */
        }
    }
}

bitfield! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
    pub struct VdmHeaderUnstructured(pub u32): FromStorage, IntoStorage {
        /// Vdm Standard or Vendor ID
        pub standard_or_vid: u16 @ 16..=31,
        /// Vdm Type (Unstructured/Structured)
        pub vdm_type: bool [VdmType] @ 15,
        /// Message defined
        pub data: u16 @ 0..=14
    }
}

impl VdmHeaderUnstructured {
    pub fn to_bytes(self, buf: &mut [u8]) {
        LittleEndian::write_u32(buf, self.0);
    }
}

bitfield! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    pub struct VdmIdentityHeader(pub u32): FromStorage, IntoStorage {
        /// Host data capable
        pub host_data: bool @ 31,
        /// Device data capable
        pub device_data: bool @ 30,
        /// Product type UFP
        pub product_type_ufp: u8 [SopProductTypeUfp] @ 27..=29,
        /// Modal Operation Supported
        pub modal_supported: bool @ 26,
        /// Product type DFP
        pub product_type_dfp: u8 [SopProductTypeDfp] @ 23..=25,
        /// Connector type
        pub connector_type: u8 [ConnectorType] @ 21..=22,
        /// VID
        pub vid: u16 @ 0..=15,
    }
}

impl VdmIdentityHeader {
    pub fn to_bytes(self, buf: &mut [u8]) {
        LittleEndian::write_u32(buf, self.0);
    }
}

#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum SopProductTypeUfp {
    NotUFP,
    PdUsbHub,
    PdUsbPeripheral,
    Psd,
}

impl From<SopProductTypeUfp> for u8 {
    fn from(value: SopProductTypeUfp) -> Self {
        match value {
            SopProductTypeUfp::NotUFP => 0b000,
            SopProductTypeUfp::PdUsbHub => 0b001,
            SopProductTypeUfp::PdUsbPeripheral => 0b010,
            SopProductTypeUfp::Psd => 0b011,
        }
    }
}

impl From<u8> for SopProductTypeUfp {
    fn from(value: u8) -> Self {
        match value {
            0b000 => SopProductTypeUfp::NotUFP,
            0b001 => SopProductTypeUfp::PdUsbHub,
            0b010 => SopProductTypeUfp::PdUsbPeripheral,
            0b011 => SopProductTypeUfp::Psd,

            _ => panic!("Cannot convert {} to SopProductTypeUfp", value), /* Illegal values
                                                                           * shall panic. */
        }
    }
}

#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum SopProductTypeDfp {
    NotDFP,
    PDUSBHub,
    PDUSBHost,
    PowerBrick,
}

impl From<SopProductTypeDfp> for u8 {
    fn from(value: SopProductTypeDfp) -> Self {
        match value {
            SopProductTypeDfp::NotDFP => 0b000,
            SopProductTypeDfp::PDUSBHub => 0b001,
            SopProductTypeDfp::PDUSBHost => 0b010,
            SopProductTypeDfp::PowerBrick => 0b011,
        }
    }
}

impl From<u8> for SopProductTypeDfp {
    fn from(value: u8) -> Self {
        match value {
            0b000 => SopProductTypeDfp::NotDFP,
            0b001 => SopProductTypeDfp::PDUSBHub,
            0b010 => SopProductTypeDfp::PDUSBHost,
            0b011 => SopProductTypeDfp::PowerBrick,

            _ => panic!("Cannot convert {} to SopProductTypeDfp", value), /* Illegal values
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
            _ => panic!("Cannot convert {} to ConnectorType", value), // Illegal values shall panic.
        }
    }
}
bitfield! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    pub struct CertStatVDO(pub u32): FromStorage, IntoStorage {
        /// XID
        pub xid: u32 @ 0..=31,
    }
}

impl CertStatVDO {
    pub fn to_bytes(self, buf: &mut [u8]) {
        LittleEndian::write_u32(buf, self.0);
    }
}

bitfield! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    pub struct ProductVDO(pub u32): FromStorage, IntoStorage {
        /// USB Product ID
        pub pid: u16 @ 16..=31,
        pub bcd_device: u16 @ 0..=15,
    }
}

impl ProductVDO {
    pub fn to_bytes(self, buf: &mut [u8]) {
        LittleEndian::write_u32(buf, self.0);
    }
}

bitfield! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
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
    pub fn to_bytes(self, buf: &mut [u8]) {
        LittleEndian::write_u32(buf, self.0);
    }
}

#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
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
            _ => panic!("Cannot convert {} to USBHighestSpeed", value), // Illegal values shall panic.
        }
    }
}

#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
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
            _ => panic!("Cannot convert {} to VconnPower", value), // Illegal values shall panic.
        }
    }
}

#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
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
            _ => panic!("Cannot convert {} to UFPVDOVersion", value), // Illegal values shall panic.
        }
    }
}

bitfield! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
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
    pub fn to_bytes(self, buf: &mut [u8]) {
        LittleEndian::write_u32(buf, self.0);
    }
}

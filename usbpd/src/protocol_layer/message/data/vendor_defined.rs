//! Definitions of vendor defined data message content.
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

impl TryFrom<u8> for VdmCommandType {
    type Error = u8;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(VdmCommandType::InitiatorREQ),
            1 => Ok(VdmCommandType::ResponderACK),
            2 => Ok(VdmCommandType::ResponderNAK),
            3 => Ok(VdmCommandType::ResponderBSY),
            _ => Err(value),
        }
    }
}

impl VdmHeaderStructured {
    pub fn try_command_type(&self) -> Result<VdmCommandType, u8> {
        VdmCommandType::try_from(self.command_type())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

impl TryFrom<u8> for VdmCommand {
    type Error = u8;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x01 => Ok(VdmCommand::DiscoverIdentity),
            0x02 => Ok(VdmCommand::DiscoverSVIDS),
            0x03 => Ok(VdmCommand::DiscoverModes),
            0x04 => Ok(VdmCommand::EnterMode),
            0x05 => Ok(VdmCommand::ExitMode),
            0x06 => Ok(VdmCommand::Attention),
            0x10 => Ok(VdmCommand::DisplayPortStatus),
            0x11 => Ok(VdmCommand::DisplayPortConfig),
            _ => Err(value),
        }
    }
}

impl VdmHeaderStructured {
    pub fn try_command(&self) -> Result<VdmCommand, u8> {
        VdmCommand::try_from(self.command())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
        pub command_type: u8 @ 6..=7,
        /// Command
        pub command: u8 @ 0..=4,
    }
}

impl VdmHeaderStructured {
    pub fn to_bytes(self, buf: &mut [u8]) {
        LittleEndian::write_u32(buf, self.0);
    }
}

impl Default for VdmHeaderStructured {
    fn default() -> Self {
        Self(0).with_vdm_type(VdmType::Structured)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

impl TryFrom<u8> for VdmVersionMajor {
    type Error = u8;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0b00 => Ok(VdmVersionMajor::Version10),
            0b01 => Ok(VdmVersionMajor::Version2x),
            _ => Err(value),
        }
    }
}

impl VdmHeaderStructured {
    pub fn try_vdm_version_major(&self) -> Result<VdmVersionMajor, u8> {
        VdmVersionMajor::try_from(self.vdm_version_major())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

impl TryFrom<u8> for VdmVersionMinor {
    type Error = u8;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0b00 => Ok(VdmVersionMinor::Version20),
            0b01 => Ok(VdmVersionMinor::Version21),
            _ => Err(value),
        }
    }
}

impl VdmHeaderStructured {
    pub fn try_vdm_version_minor(&self) -> Result<VdmVersionMinor, u8> {
        VdmVersionMinor::try_from(self.vdm_version_minor())
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
        pub product_type_ufp: u8 @ 27..=29,
        /// Modal Operation Supported
        pub modal_supported: bool @ 26,
        /// Product type DFP
        pub product_type_dfp: u8 @ 23..=25,
        /// Connector type
        pub connector_type: u8 @ 21..=22,
        /// VID
        pub vid: u16 @ 0..=15,
    }
}

impl VdmIdentityHeader {
    pub fn try_product_type_ufp(&self) -> Result<SopProductTypeUfp, u8> {
        SopProductTypeUfp::try_from(self.product_type_ufp())
    }

    pub fn try_product_type_dfp(&self) -> Result<SopProductTypeDfp, u8> {
        SopProductTypeDfp::try_from(self.product_type_dfp())
    }

    pub fn try_connector_type(&self) -> Result<ConnectorType, u8> {
        ConnectorType::try_from(self.connector_type())
    }

    pub fn to_bytes(self, buf: &mut [u8]) {
        LittleEndian::write_u32(buf, self.0);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

impl TryFrom<u8> for SopProductTypeUfp {
    type Error = u8;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0b000 => Ok(SopProductTypeUfp::NotUFP),
            0b001 => Ok(SopProductTypeUfp::PdUsbHub),
            0b010 => Ok(SopProductTypeUfp::PdUsbPeripheral),
            0b011 => Ok(SopProductTypeUfp::Psd),
            _ => Err(value),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

impl TryFrom<u8> for SopProductTypeDfp {
    type Error = u8;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0b000 => Ok(SopProductTypeDfp::NotDFP),
            0b001 => Ok(SopProductTypeDfp::PDUSBHub),
            0b010 => Ok(SopProductTypeDfp::PDUSBHost),
            0b011 => Ok(SopProductTypeDfp::PowerBrick),
            _ => Err(value),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
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

impl TryFrom<u8> for ConnectorType {
    type Error = u8;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0b10 => Ok(ConnectorType::USBTypeCReceptacle),
            0b11 => Ok(ConnectorType::USBTypeCPlug),
            _ => Err(value),
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
    pub fn try_vconn_power(&self) -> Result<VconnPower, u8> {
        VconnPower::try_from(self.vconn_power())
    }

    pub fn try_usb_highest_speed(&self) -> Result<USBHighestSpeed, u8> {
        USBHighestSpeed::try_from(self.usb_highest_speed())
    }

    pub fn try_version(&self) -> Result<UFPVDOVersion, u8> {
        UFPVDOVersion::try_from(self.version())
    }
}

impl UFPTypeVDO {
    pub fn to_bytes(self, buf: &mut [u8]) {
        LittleEndian::write_u32(buf, self.0);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

impl TryFrom<u8> for USBHighestSpeed {
    type Error = u8;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0b000 => Ok(USBHighestSpeed::USB20Only),
            0b001 => Ok(USBHighestSpeed::USB32Gen1),
            0b010 => Ok(USBHighestSpeed::USB32Gen2),
            0b011 => Ok(USBHighestSpeed::USB40Gen3),
            0b100 => Ok(USBHighestSpeed::USB40Gen4),
            _ => Err(value),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

impl TryFrom<u8> for VconnPower {
    type Error = u8;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0b000 => Ok(VconnPower::P1W),
            0b001 => Ok(VconnPower::P1_5W),
            0b010 => Ok(VconnPower::P2W),
            0b011 => Ok(VconnPower::P3W),
            0b100 => Ok(VconnPower::P4W),
            0b101 => Ok(VconnPower::P5W),
            0b110 => Ok(VconnPower::P6W),
            _ => Err(value),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

impl TryFrom<u8> for UFPVDOVersion {
    type Error = u8;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0b011 => Ok(UFPVDOVersion::Version1_3),
            _ => Err(value),
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

#[cfg(test)]
mod tests {
    use super::*;

    // Reserved VDM values shall not panic when the fields are inspected.
    #[test]
    fn vdm_command_reserved_is_err_not_panic() {
        // Known commands are 0x01..=0x06 and DP-specific 0x10/0x11. The 5-bit command
        // field holds 0..=31; everything else in that range must error (not panic).
        let reserved = (0u8..=0x1f).filter(|raw| !matches!(raw, 0x01..=0x06 | 0x10 | 0x11));
        for raw in reserved {
            let header = VdmHeaderStructured(VdmHeaderStructured::default().0 | raw as u32);
            assert_eq!(header.try_command().is_err(), true, "raw 0x{raw:02x} must error");
            assert_eq!(header.try_command().unwrap_err(), raw);
        }
    }

    #[test]
    fn vdm_command_type_value_checks() {
        // command_type is a 2-bit field @ 6..=7 with four defined encodings.
        let header = VdmHeaderStructured::default().with_command_type(0b01);
        assert_eq!(header.try_command_type(), Ok(VdmCommandType::ResponderACK));
        let header = VdmHeaderStructured::default().with_command_type(0b10);
        assert_eq!(header.try_command_type(), Ok(VdmCommandType::ResponderNAK));
        assert_eq!(header.command_type(), 0b10);
    }

    #[test]
    fn vdm_version_reserved_is_err_not_panic() {
        for raw in [0b10u8, 0b11] {
            assert_eq!(VdmVersionMajor::try_from(raw).is_err(), true);
            assert_eq!(VdmVersionMinor::try_from(raw).is_err(), true);
        }
    }

    #[test]
    fn identity_header_reserved_fields_are_err_not_panic() {
        // UFP product type: only 0b000..=0b011 defined.
        for raw in 0b100u8..=0b111 {
            let header = VdmIdentityHeader((raw as u32) << 27);
            assert_eq!(header.try_product_type_ufp().is_err(), true, "ufp {raw:#04b}");
        }
        // DFP product type: only 0b000..=0b011 defined.
        for raw in 0b100u8..=0b111 {
            let header = VdmIdentityHeader((raw as u32) << 23);
            assert_eq!(header.try_product_type_dfp().is_err(), true, "dfp {raw:#04b}");
        }
        // Connector type: only 0b10/0b11 defined.
        for raw in [0b00u8, 0b01] {
            assert_eq!(ConnectorType::try_from(raw).is_err(), true);
        }
    }

    #[test]
    fn ufp_vdo_reserved_values_are_err_not_panic() {
        // version: only 0b011 defined.
        for raw in [0b000u8, 0b001, 0b010, 0b100, 0b111] {
            let vdo = UFPTypeVDO((raw as u32) << 29);
            assert_eq!(vdo.try_version().is_err(), true, "version {raw:#04b}");
        }
        // USB highest speed: only 0b000..=0b100 defined.
        for raw in [0b101u8, 0b110, 0b111] {
            let vdo = UFPTypeVDO(raw as u32);
            assert_eq!(vdo.try_usb_highest_speed().is_err(), true, "speed {raw:#04b}");
        }
        // Vconn power: only 0b000..=0b110 defined.
        let vdo = UFPTypeVDO((0b111_u32) << 8);
        assert_eq!(vdo.try_vconn_power().is_err(), true);
    }

    #[test]
    fn valid_values_convert() {
        assert_eq!(VdmCommand::try_from(0x01).unwrap(), VdmCommand::DiscoverIdentity);
        for (raw, expected) in [
            (0u8, VdmCommandType::InitiatorREQ),
            (1, VdmCommandType::ResponderACK),
            (2, VdmCommandType::ResponderNAK),
            (3, VdmCommandType::ResponderBSY),
        ] {
            assert_eq!(VdmCommandType::try_from(raw).unwrap(), expected);
        }
        for (raw, expected) in [(0b00u8, VdmVersionMajor::Version10), (0b01, VdmVersionMajor::Version2x)] {
            assert_eq!(VdmVersionMajor::try_from(raw).unwrap(), expected);
        }
        for (raw, expected) in [(0b00u8, VdmVersionMinor::Version20), (0b01, VdmVersionMinor::Version21)] {
            assert_eq!(VdmVersionMinor::try_from(raw).unwrap(), expected);
        }
        assert_eq!(
            ConnectorType::try_from(0b10).unwrap(),
            ConnectorType::USBTypeCReceptacle
        );
        assert_eq!(ConnectorType::try_from(0b11).unwrap(), ConnectorType::USBTypeCPlug);
        assert_eq!(USBHighestSpeed::try_from(0b100).unwrap(), USBHighestSpeed::USB40Gen4);
        assert_eq!(VconnPower::try_from(0b110).unwrap(), VconnPower::P6W);
        assert_eq!(UFPVDOVersion::try_from(0b011).unwrap(), UFPVDOVersion::Version1_3);
        assert_eq!(
            SopProductTypeUfp::try_from(0b010).unwrap(),
            SopProductTypeUfp::PdUsbPeripheral
        );
        assert_eq!(SopProductTypeDfp::try_from(0b001).unwrap(), SopProductTypeDfp::PDUSBHub);
    }
}

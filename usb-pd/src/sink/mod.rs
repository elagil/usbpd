use defmt::Format;
use heapless::Vec;

use crate::messages::pdo::SourceCapabilities;
use crate::messages::vdo::{CertStatVDO, ProductVDO, UFPTypeVDO, VDMHeader, VDMIdentityHeader};

pub mod policy;

/// Sink events
#[derive(Format)]
pub enum Event {
    /// Power delivery protocol has changed
    ProtocolChanged,
    /// Source capabilities have changed (immediately request power)
    SourceCapabilitiesChanged(SourceCapabilities),
    /// Requested power has been accepted (but not ready yet)
    PowerAccepted,
    /// Requested power has been rejected
    PowerRejected,
    /// Requested power is now ready
    PowerReady,
    /// VDM received
    VDMReceived((VDMHeader, Vec<u32, 7>)),
}

/// Requests made to sink
#[derive(Format)]
pub enum Request {
    RequestPower {
        /// Index of the desired PowerDataObject
        index: usize,
        current: u16,
    },
    RequestPPS {
        /// Index of the desired PowerDataObject
        index: usize,
        /// Requested voltage (in mV)
        voltage: u16,
        /// Requested maximum current (in mA)
        current: u16,
    },
    REQDiscoverIdentity,
    ACKDiscoverIdentity {
        identity: VDMIdentityHeader,
        cert_stat: CertStatVDO,
        product: ProductVDO,
        product_type_ufp: UFPTypeVDO,
        // Does not exist yet...        product_type_dfp: Option<DFP>,
    },
    REQDiscoverSVIDS,
}

/// Power supply type
#[derive(Clone, Copy, PartialEq)]
pub enum SupplyType {
    /// Fixed supply (Vmin = Vmax)
    Fixed = 0,
    /// Battery
    Battery = 1,
    /// Variable supply (non-battery)
    Variable = 2,
    /// Programmable power supply
    Pps = 3,
}

/// Power deliver protocol
#[derive(Format, PartialEq, Clone, Copy)]
enum Protocol {
    /// No USB PD communication (5V only)
    Usb20,
    /// USB PD communication
    UsbPd,
}

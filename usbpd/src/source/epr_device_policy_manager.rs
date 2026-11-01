//! The EPR source device policy manager extensions.
//!
//! Per PD 3.2 Sec. 8.3.3.26.

use crate::protocol_layer::message::data::source_capabilities::SourceCapabilities;

/// Extended Power Range Source device implementation
///
/// Leave blank if EPR is not supported by the device.
pub trait EprDevicePolicyManager {
    /// Return `true` if device is EPR capable.
    ///
    /// Also possible to dynamically assess EPR capability
    fn epr_capable(&mut self) -> bool {
        false
    }

    /// Return `true` if the `Cable Plug` supports EPR
    fn epr_cable_good(&mut self) -> bool {
        false
    }

    /// Return device's EPR capabilities
    fn epr_source_capabilities(&mut self) -> SourceCapabilities {
        SourceCapabilities::new_vsafe5v_only(50)
    }
}

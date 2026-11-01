//! The EPR sink device policy manager extensions.
//!
//! Per PD 3.2 Sec. 8.3.3.26.

use core::future::Future;

use crate::protocol_layer::message::data::epr_mode;

/// Extended Power Range device implementations
///
/// Leave blank if the device does not support EPR.
pub trait EprDevicePolicyManager {
    /// Notify the device that EPR mode entry failed.
    ///
    /// Per PD 3.2 Sec. 8.3.3.26.2.1, when the source responds with
    /// EPR_Mode (Enter Failed), the sink transitions to soft reset. This callback
    /// informs the DPM of the failure reason before the soft reset occurs.
    ///
    /// The failure reasons are defined in PD 3.2 Tab. 6.50 and include:
    /// - Cable not EPR capable
    /// - Source failed to become VCONN source
    /// - EPR capable bit not set in RDO
    /// - Source unable to enter EPR mode (sink may retry later)
    /// - EPR capable bit not set in PDO
    fn epr_mode_entry_failed(&mut self, _reason: epr_mode::DataEnterFailed) -> impl Future<Output = ()> {
        async {}
    }
}

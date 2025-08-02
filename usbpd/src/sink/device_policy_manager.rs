//! The device policy manager (DPM) allows a device to control the policy engine, and be informed about status changes.
//!
//! For example, through the DPM, a device can request certain source capabilities (voltage, current),
//! or renegotiate the power contract.
use core::future::Future;

use crate::protocol_layer::message::{pdo, request};

/// Events that the device policy manager can send to the policy engine.
#[derive(Debug)]
pub enum Event {
    /// Empty event.
    None,
    /// Request source capabilities (again).
    RequestSourceCapabilities,
    /// Request a certain power level.
    RequestPower(request::PowerSource),
}

/// Trait for the device policy manager.
///
/// This entity commands the policy engine and enforces device policy.
pub trait DevicePolicyManager {
    /// Request a power source.
    ///
    /// Defaults to 5 V at maximum current.
    fn request(&mut self, source_capabilities: &pdo::SourceCapabilities) -> impl Future<Output = request::PowerSource> {
        async {
            request::PowerSource::new_fixed(
                request::CurrentRequest::Highest,
                request::VoltageRequest::Safe5V,
                source_capabilities,
            )
            .unwrap()
        }
    }

    /// Notify the device that it shall transition to a new power level.
    ///
    /// The device is informed about the request that was accepted by the source.
    fn transition_power(&mut self, _accepted: &request::PowerSource) -> impl Future<Output = ()> {
        async {}
    }

    /// The policy engine gets and evaluates device policy events when ready.
    ///
    /// By default, this is a future that never resolves.
    fn get_event(&mut self, _source_capabilities: &pdo::SourceCapabilities) -> impl Future<Output = Event> {
        async { core::future::pending().await }
    }
}

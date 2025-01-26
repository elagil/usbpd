//! The device policy manager (DPM) allows a device to control the policy engine, and be informed about status changes.
//!
//! For example, through the DPM, a device can request certain source capabilities (voltage, current),
//! or renegotiate the power contract.
use core::future::Future;

use defmt::Format;

use super::{FixedSupplyRequest, PowerSourceRequest};
use crate::protocol_layer::message::pdo::{PowerDataObject, SourceCapabilities};

/// Events that the device policy manager can send to the policy engine.
#[derive(Format)]
pub enum Event {
    /// FIXME: Implement events.
    Reserved,
}

/// Trait for the device policy manager.
///
/// This entity commands the policy engine and enforces device policy.
pub trait DevicePolicyManager {
    /// Request a power source.
    ///
    /// If `None` is returned, the policy engine informs the source of a capability mismatch.
    /// As a default, request the highest advertised voltage.
    fn request(&mut self, source_capabilities: SourceCapabilities) -> impl Future<Output = Option<PowerSourceRequest>> {
        async { request_highest_voltage(source_capabilities) }
    }

    /// Notify the device that it shall transition to a new power level.
    fn transition_power(&mut self) -> impl Future<Output = ()> {
        async {}
    }

    /// The policy engine gets and evaluates device policy events when ready.
    ///
    /// By default, this is a future that never resolves.
    fn get_event(&mut self) -> impl Future<Output = Event> {
        async { core::future::pending().await }
    }
}

/// Request the maximum voltage at its highest allowed current.
///
/// Normally, this yields the highest power.
pub fn request_highest_voltage(source_capabilities: SourceCapabilities) -> Option<PowerSourceRequest> {
    let mut choice = None;

    for (index, pdo) in source_capabilities.pdos().iter().enumerate() {
        match (choice, pdo) {
            (None, PowerDataObject::FixedSupply(supply)) => {
                choice = Some((
                    supply.voltage(),
                    FixedSupplyRequest {
                        index: index as u8,
                        current_10ma: supply.raw_max_current(),
                    },
                ))
            }
            (Some((voltage, _)), PowerDataObject::FixedSupply(supply)) => {
                if supply.voltage() > voltage {
                    choice = Some((
                        supply.voltage(),
                        FixedSupplyRequest {
                            index: index as u8,
                            current_10ma: supply.raw_max_current(),
                        },
                    ));
                }
            }
            _ => (),
        };
    }

    if let Some((_, supply)) = choice {
        Some(PowerSourceRequest::FixedSupply(supply))
    } else {
        None
    }
}

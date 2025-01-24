use crate::protocol_layer::message::pdo::{PowerDataObject, SourceCapabilities};

use super::{FixedSupply, PowerSourceRequest};

/// Trait for the device policy manager.
///
/// This entity commands the policy engine and enforces device policy.
pub trait DevicePolicyManager {
    /// Request a power source.
    ///
    /// If `None` is returned, the policy engine informs the source of a capability mismatch.
    fn request(&mut self, source_capabilities: SourceCapabilities) -> Option<PowerSourceRequest>;
}

/// Request the maximum voltage at its highest allowed current.
///
/// Normally, this yields the highest power.
pub fn request_highest_voltage(source_capabilities: SourceCapabilities) -> Option<PowerSourceRequest> {
    let mut choice = None;

    for (index, pdo) in source_capabilities.pdos().iter().enumerate() {
        match (choice, pdo) {
            (None, PowerDataObject::FixedSupply(supply)) => {
                choice = Some(FixedSupply {
                    index: index as u8,
                    raw_voltage: supply.raw_voltage(),
                    raw_max_current: supply.raw_max_current(),
                })
            }
            (Some(x), PowerDataObject::FixedSupply(supply)) => {
                if supply.raw_voltage() > x.raw_voltage {
                    choice = Some(FixedSupply {
                        index: index as u8,
                        raw_max_current: supply.raw_max_current(),
                        raw_voltage: supply.raw_voltage(),
                    });
                }
            }
            _ => (),
        };
    }

    if let Some(x) = choice {
        Some(PowerSourceRequest::FixedSupply(x))
    } else {
        None
    }
}

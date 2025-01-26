//! The device policy manager (DPM) allows a device to control the policy engine, and be informed about status changes.
//!
//! For example, through the DPM, a device can request certain source capabilities (voltage, current),
//! or renegotiate the power contract.
use core::future::Future;

use uom::si::u16::ElectricPotential;

use super::{FixedSupplyRequest, PowerSourceRequest};
use crate::protocol_layer::message::pdo::{PowerDataObject, SourceCapabilities};

/// Requestable voltages.
#[derive(Debug)]
pub enum FixedVoltageRequest {
    /// The lowest fixed voltage that the source can supply.
    Lowest,
    /// The highest fixed voltage that the source can supply.
    Highest,
    /// A specific voltage.
    Specific(ElectricPotential),
}

/// Events that the device policy manager can send to the policy engine.
#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
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
    /// By default, request the highest advertised voltage.
    fn request(&mut self, source_capabilities: &SourceCapabilities) -> impl Future<Output = PowerSourceRequest> {
        async { request_fixed_voltage(source_capabilities, FixedVoltageRequest::Highest) }
    }

    /// Notify the device that it shall transition to a new power level.
    ///
    /// The device is informed about the request that was accepted by the source.
    fn transition_power(&mut self, _accepted: &PowerSourceRequest) -> impl Future<Output = ()> {
        async {}
    }

    /// The policy engine gets and evaluates device policy events when ready.
    ///
    /// By default, this is a future that never resolves.
    fn get_event(&mut self) -> impl Future<Output = Event> {
        async { core::future::pending().await }
    }
}

/// Request a fixed voltage at maximum current.
///
/// Normally, this yields the highest power.
pub fn request_fixed_voltage(
    source_capabilities: &SourceCapabilities,
    fixed_voltage_request: FixedVoltageRequest,
) -> PowerSourceRequest {
    let mut choice = None;

    for (index, pdo) in source_capabilities.pdos().iter().enumerate() {
        match (choice, pdo) {
            (None, PowerDataObject::FixedSupply(supply)) => {
                choice = Some((
                    supply.voltage(),
                    FixedSupplyRequest {
                        index: index as u8,
                        current: supply.max_current(),
                        voltage: supply.voltage(),
                        capability_mismatch: false,
                    },
                ))
            }
            (Some((chosen_voltage, _)), PowerDataObject::FixedSupply(supply)) => {
                if match fixed_voltage_request {
                    FixedVoltageRequest::Lowest => supply.voltage() < chosen_voltage,
                    FixedVoltageRequest::Highest => supply.voltage() > chosen_voltage,
                    FixedVoltageRequest::Specific(requested_voltage) => supply.voltage() == requested_voltage,
                } {
                    choice = Some((
                        supply.voltage(),
                        FixedSupplyRequest {
                            index: index as u8,
                            current: supply.max_current(),
                            voltage: supply.voltage(),
                            capability_mismatch: false,
                        },
                    ));

                    if matches!(fixed_voltage_request, FixedVoltageRequest::Specific(_)) {
                        // Found requested voltage.
                        break;
                    }
                }
            }
            _ => (),
        };
    }

    if let Some((_, supply)) = choice {
        PowerSourceRequest::FixedSupply(supply)
    } else {
        unreachable!("Must select a valid capability")
    }
}

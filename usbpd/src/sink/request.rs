//! Facilitates generating power request messages for the source.

use uom::si::electric_current;
use uom::si::u16::{ElectricCurrent, ElectricPotential};

use crate::protocol_layer::message::pdo::{FixedSupply, PowerDataObject, SourceCapabilities};
use crate::protocol_layer::message::request::FixedVariableSupply;

/// Errors that can occur during sink requests towards the source.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// A requested (specific) voltage does not exist in the PDOs.
    VoltageMismatch,
}

/// Requestable voltage levels.
#[derive(Debug)]
pub enum VoltageRequest {
    /// The safe 5 V supply.
    Safe5V,
    /// The highest voltage that the source can supply.
    Highest,
    /// A specific voltage.
    Specific(ElectricPotential),
}

/// Requestable currents.
#[derive(Debug)]
pub enum CurrentRequest {
    /// The highest current that the source can supply.
    Highest,
    /// A specific current.
    Specific(ElectricCurrent),
}

/// Types of power source requests that a device can send to the policy engine.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub enum PowerSourceRequest {
    /// Request a fixed or variable supply.
    FixedVariableSupply(FixedVariableSupply),
}

impl PowerSourceRequest {
    /// Find the highest fixed voltage that can be found in the source capabilities.
    ///
    /// Reports the index of the found PDO, and the fixed supply instance, or `None` if there is no fixed supply PDO.
    fn find_highest_fixed_voltage(source_capabilities: &SourceCapabilities) -> Option<(usize, &FixedSupply)> {
        let mut selected_pdo = None;

        for (index, cap) in source_capabilities.pdos().iter().enumerate() {
            if let PowerDataObject::FixedSupply(fixed_supply) = cap {
                selected_pdo = match selected_pdo {
                    None => Some((index, fixed_supply)),
                    Some(x) => {
                        if fixed_supply.voltage() > x.1.voltage() {
                            Some((index, fixed_supply))
                        } else {
                            selected_pdo
                        }
                    }
                };
            }
        }

        selected_pdo
    }

    /// Find a specific fixed voltage within the source capabilities.
    ///
    /// Reports the index of the found PDO, and the fixed supply instance, or `None` if there is no match to the request.
    fn find_specific_fixed_voltage(
        source_capabilities: &SourceCapabilities,
        voltage: ElectricPotential,
    ) -> Option<(usize, &FixedSupply)> {
        for (index, cap) in source_capabilities.pdos().iter().enumerate() {
            if let PowerDataObject::FixedSupply(fixed_supply) = cap {
                if fixed_supply.voltage() == voltage {
                    return Some((index, fixed_supply));
                }
            }
        }

        None
    }

    /// Create a new power source request for a fixed supply.
    ///
    /// Finds a suitable PDO by evaluating the provided current and voltage requests against the source capabilities.
    pub fn new_fixed(
        current_request: CurrentRequest,
        voltage_request: VoltageRequest,
        source_capabilities: &SourceCapabilities,
    ) -> Result<Self, Error> {
        let selected = match voltage_request {
            VoltageRequest::Safe5V => source_capabilities.vsafe_5v().map(|supply| (0, supply)),
            VoltageRequest::Highest => Self::find_highest_fixed_voltage(source_capabilities),
            VoltageRequest::Specific(x) => Self::find_specific_fixed_voltage(source_capabilities, x),
        };

        if selected.is_none() {
            return Err(Error::VoltageMismatch);
        }

        let (index, supply) = selected.unwrap();

        let (current, mismatch) = match current_request {
            CurrentRequest::Highest => (supply.max_current(), false),
            CurrentRequest::Specific(x) => (x, x > supply.max_current()),
        };

        let mut raw_current = current.get::<electric_current::centiampere>();

        if raw_current > 0x3ff {
            error!("Clamping invalid current: {} mA", 10 * raw_current);
            raw_current = 0x3ff;
        }

        let object_position = index + 1;
        assert!(object_position > 0b0000 && object_position <= 0b1110);

        Ok(Self::FixedVariableSupply(
            FixedVariableSupply(0)
                .with_raw_operating_current(raw_current)
                .with_raw_max_operating_current(raw_current)
                .with_object_position(object_position as u8)
                .with_capability_mismatch(mismatch)
                .with_no_usb_suspend(true)
                .with_usb_communications_capable(true), // FIXME: Make adjustable?
        ))
    }
}

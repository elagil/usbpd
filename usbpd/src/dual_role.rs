//! Implementation of a dual role device

use core::marker::PhantomData;

use usbpd_traits::Driver;

use crate::PowerRole;
use crate::sink::device_policy_manager::SinkDpm;
use crate::sink::policy_engine::{Error as SinkError, Sink};
use crate::source::device_policy_manager::SourceDpm;
use crate::source::policy_engine::{Error as SourceError, Source};
use crate::timers::Timer;

/// Errors that can occur in the either the sink or source policy engine state machine.
#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Error {
    /// Error while operating as a Sink
    Sink(SinkError),
    /// Error while operating as a Source
    Source(SourceError),
}

/// Dual Role Port that will automatically undergo role swaps,
/// using the defined functions in the two `DualRoleDevicePolicyManagers`
pub struct DualRolePort<DRIVER, TIMER, DPM>
where
    DRIVER: Driver,
    TIMER: Timer,
    DPM: SourceDpm + SinkDpm,
{
    device_policy_manager: DPM,
    driver: DRIVER,
    timer: PhantomData<TIMER>,
}

impl<DRIVER, TIMER, DPM> DualRolePort<DRIVER, TIMER, DPM>
where
    DRIVER: Driver,
    TIMER: Timer,
    DPM: SourceDpm + SinkDpm,
{
    /// Create a new dual role policy engine with a given `driver`.
    pub fn new(driver: DRIVER, device_policy_manager: DPM) -> Self {
        Self {
            device_policy_manager,
            driver,
            timer: PhantomData,
        }
    }

    /// Run the sink's state machine continuously.
    ///
    /// The loop is only broken for unrecoverable errors, for example if the port partner is unresponsive.
    pub async fn run(&mut self, initial_role: PowerRole) -> Result<(), Error> {
        let mut role = initial_role;
        let mut role_swapped = false;

        loop {
            match role {
                PowerRole::Source => {
                    let mut source = Source::<DRIVER, TIMER, DPM>::new_dual_role(
                        &mut self.driver,
                        &mut self.device_policy_manager,
                        role_swapped,
                    );

                    match source.run().await {
                        Err(SourceError::SwapToSink) => {
                            role = PowerRole::Sink;
                            role_swapped = true;
                            continue;
                        }
                        Err(err) => return Err(Error::Source(err)),
                        Ok(()) => return Ok(()),
                    }
                }
                PowerRole::Sink => {
                    let mut sink =
                        Sink::<DRIVER, TIMER, DPM>::new_dual_role(&mut self.driver, &mut self.device_policy_manager);

                    match sink.run().await {
                        Err(SinkError::SwapToSource) => {
                            role = PowerRole::Source;
                            role_swapped = true;
                            continue;
                        }
                        Err(err) => return Err(Error::Sink(err)),
                        Ok(()) => return Ok(()),
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod test {

    #[tokio::test]
    async fn test_dual_role() {
        // TODO: Add tests
    }
}

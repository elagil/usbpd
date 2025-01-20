//! Policy engine for the implementation of a sink.
use core::marker::PhantomData;

use defmt::{error, trace, Format};

use crate::protocol_layer::message::header::{Header, SpecificationRevision};
use crate::protocol_layer::ProtocolLayer;
use crate::{DataRole, Driver, PowerRole, Timer};

/// Sink states.
#[derive(Debug, Clone, Copy, Format)]
enum State {
    /// Default state at startup.
    Startup,
    Discovery,
    WaitForCapabilities,
    EvaluateCapability,
    SelectCapability,
    TransitionSink,
    Ready,
    HardReset,
    TransitionToDefault,
    GiveSinkCap,
    GetSourceCap,
    EPRKeepAlive,
}

#[derive(Debug)]
pub struct Sink<DRIVER: Driver, TIMER: Timer> {
    protocol_layer: ProtocolLayer<DRIVER>,
    state: State,

    timeout: PhantomData<TIMER>,
}

#[derive(Debug, Format)]
enum Error {
    Startup,
    Timeout,
    Protocol,
}

impl<DRIVER: Driver, TIMER: Timer> Sink<DRIVER, TIMER> {
    /// Create a new sink policy engine with a given `driver`.
    pub fn new(driver: DRIVER) -> Self {
        Self {
            protocol_layer: ProtocolLayer::new(
                driver,
                Header::new_template(DataRole::Dfp, PowerRole::Sink, SpecificationRevision::R3_0),
            ),
            state: State::Startup,

            timeout: Default::default(),
        }
    }

    /// Run the state machine.
    pub async fn run(&mut self) -> Result<(), ()> {
        loop {
            let result = self.update_state().await;

            match result {
                Err(Error::Protocol) => {
                    error!("Protocol error in sink state transition");
                    self.state = State::HardReset
                }
                _ => (),
            }
        }
    }

    async fn update_state(&mut self) -> Result<(), Error> {
        trace!("Handle sink state: {:?}", self.state);

        let new_state = match self.state {
            State::Startup => {
                // Entry: Reset protocol layer (drop?)
                //   Resets messageIdCounter and stored MessageId
                // Transition to Discovery
                State::Discovery
            }
            State::Discovery => {
                // Ask protocol layer
                // let vbus_fut = self.driver.wait_for_vbus();
                // let timeout_fut = TIMER::after_millis(456);

                // pin_mut!(vbus_fut);
                // pin_mut!(timeout_fut);

                // match select(vbus_fut, timeout_fut).await {
                //     Either::Left((_, _)) => State::WaitForCapabilities,
                //     Either::Right((_, _)) => return Err(PolicyError::Timeout),
                // }
                //
                State::WaitForCapabilities
            }
            State::WaitForCapabilities => {
                // let message = self
                //     .receive(
                //         MessageType::Data(DataMessageType::SourceCapabilities),
                //         TIMER::after_millis(250), // SinkWaitCap
                //     )
                //     .await?;

                // debug!("Capabilities: {}", message);

                // Transition to EvaluateCapability if
                // - SPR mode & source_capabilities message, or
                // - EPR mode & epr_source_capabilities message received

                // FIXME: Actually read message
                State::EvaluateCapability
            }
            State::EvaluateCapability => {
                // Entry: Reset HardResetCounter
                // self.hard_reset_count.reset();

                // Evaluate capabilities, and:
                // - Select suitable one, or
                // - Respond with capability mismatch
                // Transition to SelectCapability after capabilities are
                // evaluated
                State::SelectCapability
            }
            State::SelectCapability => {
                // FIXME: Handle TX error.
                // _ = self.request_power(500, 1).await;

                // self.receive(
                //     MessageType::Control(ControlMessageType::GoodCRC),
                //     TIMER::after_millis(100),
                // )
                // .await?;

                // Entry: Send request, as above
                // Entry: Initialize and run SenderResponseTimer
                // Transition to
                // - HardReset, if SenderResponseTimer timeout
                // - WaitForCapabilities, if no explicit contract and reject/wait message
                //   received
                // - Ready, if explicit contract and reject/wait message received
                // - TransitionSink if accept message received

                State::TransitionSink
            }
            State::TransitionSink => {
                // Entry: Initialize and run PSTransitionTimer
                // Exit: Request device policy manager transitions sink power
                // supply to new power (if required)
                // Transition to
                // - HardReset on protocol error??
                // - Ready after PS_RDY message received

                // self.receive(
                //     MessageType::Control(ControlMessageType::PsRdy),
                //     TIMER::after_millis(100),
                // )
                // .await?;

                State::Ready
            }
            State::Ready => {
                // Entry: Init. and run SinkRequestTimer(2) on receiving wait
                // Entry: Init. and run DiscoverIdentityTimer(4)
                // Entry: Init. and run SinkPPSPeriodicTimer(5)
                // Entry: Init. and run SinkEPRKeppAlivaTimer(6) in EPR mode
                // Entry: Send GetSinkCap message if sink supports fast role
                // swap Exit: If initiating an AMS, notify
                // protocol layer??? Transition to
                // - EPRKeepAlive on SinkEPRKeepAliveTimer timeout
                // - EvalueCapability on SPR mode and source_capabilities message, or EPR mode
                //   and EPR_source_capabilities message
                TIMER::after_secs(2).await;

                State::Ready
            }
            State::HardReset => {
                // Signal hard reset, increment hard reset counter
                // In protocol layer?
                // self.hard_reset_count.increment()?;

                // Other causes of entry:
                // - SinkWaitCapTimer timeout or PSTransitionTimer timeout, when reset count <
                //   max
                // - Hard reset request from device policy manager
                // - EPR mode and EPR_Source_Capabilities message with EPR PDO in pos. 1..7
                // - source_capabilities message not requested by get_source_caps
                // Transition to TransitionToDefault

                State::TransitionToDefault
            }
            State::TransitionToDefault => {
                // Entry: Request power sink transition to default
                // Entry: Reset local hardware???
                // Entry: Set port data role to UFP and turn off VConn
                // Exit: Inform protocol layer that hard reset is complete
                // Transition to Startup

                State::Startup
            }
            State::GiveSinkCap => {
                // Entry: Get present sink capabilities
                // Send capabilities message (based on device policy manager
                // response):
                // - If get_sink_cap message received, send sink_capabilities message???
                // - If EPR_get_sink_kap message received, send EPR_sink_cap message
                // Transition to Ready after sink capabilities sent (what about
                // entry/exit?)

                State::Ready
            }
            State::GetSourceCap => {
                // What is this state?
                // FIXME: wrong state
                State::SelectCapability
            }
            State::EPRKeepAlive => {
                // Entry: Send EPRKeepAlive Message
                // Entry: Init. and run SenderReponseTimer
                // Transition to
                // - Ready on EPRKeepAliveAck message
                // - HardReset on SenderResponseTimerTimeout

                State::Ready
            }
        };

        // trace!("Sink state transition: {:?} -> {:?}", self.state, new_state);
        self.state = new_state;

        Ok(())
    }

    // async fn request_power(&mut self, max_current: u16, index: usize) -> Result<(), DRIVER::TxError> {
    //     // Create 'request' message
    //     let mut data = [0; 6];

    //     let header = self.new_data_header(self.tx_message_count, DataMessageType::Request, 1);
    //     header.to_bytes(&mut data[..2]);

    //     self.set_request_payload_fixed(&mut data[2..], index as u8, max_current);

    //     // Send message
    //     self.driver.transmit(&data).await
    // }

    // fn set_request_payload_fixed(&mut self, payload: &mut [u8], obj_pos: u8, mut current: u16) {
    //     current = (current + 5) / 10;

    //     if current > 0x3ff {
    //         current = 0x3ff;
    //     }

    //     let obj_pos = obj_pos + 1;
    //     assert!(obj_pos > 0b0000 && obj_pos <= 0b1110);

    //     FixedVariableRequestDataObject(0)
    //         .with_raw_operating_current(current)
    //         .with_raw_max_operating_current(current)
    //         .with_object_position(obj_pos)
    //         .with_no_usb_suspend(true)
    //         .with_usb_communications_capable(true)
    //         .to_bytes(payload);
    // }
}

pub trait DevicePolicyEngine {}

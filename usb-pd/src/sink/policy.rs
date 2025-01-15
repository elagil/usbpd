use core::fmt;
use core::future::Future;
use core::marker::PhantomData;

use defmt::{debug, error, trace, Format};
use futures::future::{select, Either};
use futures::pin_mut;

use super::Request;
use crate::counter::{Counter, CounterError, CounterType};
use crate::header::{ControlMessageType, DataMessageType, Header, MessageType, SpecificationRevision};
use crate::messages::pdo::{FixedVariableRequestDataObject, PPSRequestDataObject};
use crate::messages::vdo::{
    VDMCommand, VDMCommandType, VDMHeader, VDMHeaderStructured, VDMType, VDMVersionMajor, VDMVersionMinor,
};
use crate::messages::Message;
use crate::{DataRole, PowerRole};

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

#[derive(Debug, Clone, Copy)]
struct Sink<DRIVER: Driver, TIMER: Timer + Future> {
    driver: DRIVER,
    state: State,

    busy_count: Counter,
    caps_count: Counter,
    discover_identity_count: Counter,
    hard_reset_count: Counter,
    rx_message_count: Counter,
    tx_message_count: Counter,
    retry_count: Counter,

    timeout: PhantomData<TIMER>,
    specification_revision: SpecificationRevision,
}

struct Error<DRIVER: Driver> {
    kind: ErrorKind<DRIVER>,
}

enum ErrorKind<DRIVER: Driver> {
    Startup,
    Timeout,
    Protocol,
    RxError(DRIVER::RxError),
    CounterError(CounterError),
}

impl<DRIVER: Driver> From<CounterError> for Error<DRIVER> {
    fn from(value: CounterError) -> Self {
        Error {
            kind: ErrorKind::CounterError(value),
        }
    }
}

impl<DRIVER: Driver, TIMER: Timer + Future> Sink<DRIVER, TIMER> {
    /// Create a new `Sink` state machine with a given `driver`.
    pub fn new(driver: DRIVER) -> Self {
        Self {
            driver,
            state: State::Startup,

            busy_count: Counter::new(CounterType::Busy),
            caps_count: Counter::new(CounterType::Caps),
            discover_identity_count: Counter::new(CounterType::DiscoverIdentity),
            hard_reset_count: Counter::new(CounterType::HardReset),
            rx_message_count: Counter::new(CounterType::MessageId),
            tx_message_count: Counter::new(CounterType::MessageId),
            retry_count: Counter::new(CounterType::Retry),

            timeout: Default::default(),
            specification_revision: SpecificationRevision::R3_0,
        }
    }

    /// Run the state machine.
    pub async fn run(&mut self) -> Result<(), Error<DRIVER>> {
        loop {
            let result = self.try_transition().await;

            match result {
                Err(Error {
                    kind: ErrorKind::Protocol,
                }) => {
                    error!("Protocol error in sink state transition");
                    self.state = State::HardReset
                }
                _ => (),
            }
        }
    }

    /// Wait for a message of a certain type, until a timeout occurs.
    async fn wait_for_message(
        &mut self,
        message_type: MessageType,
        timeout_fut: TIMER,
    ) -> Result<Message, Error<DRIVER>> {
        let receive_fut = self.driver.receive();

        pin_mut!(timeout_fut);
        pin_mut!(receive_fut);

        match select(timeout_fut, receive_fut).await {
            Either::Left((_, _)) => Err(Error {
                kind: ErrorKind::Timeout,
            }),
            Either::Right((receive_result, _)) => match receive_result {
                Ok(message) => {
                    if message_type == message.header.message_type() {
                        Ok(message)
                    } else {
                        // A message was received, but the wrong kind.
                        Err(Error {
                            kind: ErrorKind::Protocol,
                        })
                    }
                }
                Err(error) => Err(Error {
                    kind: ErrorKind::RxError(error),
                }),
            },
        }
    }

    async fn try_transition(&mut self) -> Result<(), Error<DRIVER>> {
        trace!("Handle sink state: {:?}", self.state);

        let new_state = match self.state {
            State::Startup => {
                // Entry: Reset protocol layer (drop?)
                //   Resets messageIdCounter and stored MessageId
                // Transition to Discovery
                State::Discovery
            }
            State::Discovery => {
                let vbus_fut = self.driver.wait_for_vbus();
                let timeout_fut = TIMER::after_millis(456);

                pin_mut!(vbus_fut);
                pin_mut!(timeout_fut);

                match select(vbus_fut, timeout_fut).await {
                    Either::Left((_, _)) => State::WaitForCapabilities,
                    Either::Right((_, _)) => {
                        return Err(Error {
                            kind: ErrorKind::Timeout,
                        })
                    }
                }
            }
            State::WaitForCapabilities => {
                let message = self
                    .wait_for_message(
                        MessageType::Data(DataMessageType::SourceCapabilities),
                        TIMER::after_millis(123), // SinkWaitCap
                    )
                    .await?;

                // Transition to EvaluateCapability if
                // - SPR mode & source_capabilities message, or
                // - EPR mode & epr_source_capabilities message received

                // FIXME: Actually read message
                State::EvaluateCapability
            }
            State::EvaluateCapability => {
                // Entry: Reset HardResetCounter
                self.hard_reset_count.reset();

                // Evaluate capabilities, and:
                // - Select suitable one, or
                // - Respond with capability mismatch
                // Transition to SelectCapability after capabilities are
                // evaluated
                State::SelectCapability
            }
            State::SelectCapability => {
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

                State::Ready
            }
            State::HardReset => {
                // Signal hard reset, increment hard reset counter
                self.hard_reset_count.increment()?;

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

        trace!("Sink state transition: {:?} -> {:?}", self.state, new_state);
        self.state = new_state;

        Ok(())
    }

    fn new_control_header(&self, message_id: Counter, message_type: ControlMessageType) -> Header {
        Header::new_control(
            self.tx_message_count,
            message_type,
            DataRole::Ufp,
            PowerRole::Sink,
            self.specification_revision,
        )
    }

    fn new_data_header(&self, message_id: Counter, message_type: DataMessageType, num_objects: u8) -> Header {
        Header::new_data(
            message_id,
            message_type,
            DataRole::Ufp,
            PowerRole::Sink,
            self.specification_revision,
            num_objects,
        )
    }

    pub async fn good_crc(&mut self) -> Result<(), DRIVER::TxError> {
        trace!("Send GoodCrc");
        let header = self.new_control_header(self.rx_message_count, ControlMessageType::GoodCRC);

        let mut data = [0u8; 2];
        header.to_bytes(&mut data);
        self.driver.send(&data).await
    }

    /// Performs a soft reset.
    pub async fn soft_reset(&mut self) -> Result<(), DRIVER::TxError> {
        let mut buf = [0; 2];
        let header = self.new_control_header(self.tx_message_count, ControlMessageType::SoftReset);
        header.to_bytes(&mut buf);

        self.driver.send(&buf).await
    }

    pub async fn request(&mut self, request: Request) -> Result<(), DRIVER::TxError> {
        match request {
            Request::RequestPower { index, current } => self.request_power(current, index).await,

            Request::RequestPPS {
                index,
                voltage,
                current,
            } => {
                let mut data = [0; 6];

                // Add one to index to account for array offsets starting at 0 and obj_pos
                // starting at 1...
                let obj_pos = index + 1;
                assert!(obj_pos > 0b0000 && obj_pos <= 0b1110);

                // Create header
                let header = self.new_data_header(self.tx_message_count, DataMessageType::Request, 1);

                header.to_bytes(&mut data[..2]);

                // Create PPS request data object
                let pps = PPSRequestDataObject(0)
                    .with_object_position(obj_pos as u8)
                    .with_raw_operating_current(current / 50) // Convert current from millis to 50ma units
                    .with_raw_output_voltage(voltage / 20) // Convert voltage from millis to 20mv units
                    .with_capability_mismatch(false)
                    .with_epr_mode_capable(false)
                    .with_usb_communications_capable(true);
                pps.to_bytes(&mut data[2..]);

                // Send request message
                self.driver.send(&data).await
            }

            Request::ACKDiscoverIdentity {
                identity,
                cert_stat,
                product,
                product_type_ufp,
                //product_type_dfp,
            } => {
                debug!("ACKDiscoverIdentity");
                // The size of this array will actually change depending on data...
                // TODO: Fix this!
                let mut data = [0; 2 + 5 * 4];
                let header = self.new_data_header(
                    self.tx_message_count,
                    DataMessageType::VendorDefined,
                    5, // 5 VDOs, vdm header, id header, cert, product, UFP product type
                );
                header.to_bytes(&mut data[..2]);

                let vdm_header_vdo = VDMHeader::Structured(
                    VDMHeaderStructured(0)
                        .with_command(VDMCommand::DiscoverIdentity)
                        .with_command_type(VDMCommandType::ResponderACK)
                        .with_object_position(0) // 0 Must be used for descover identity
                        .with_standard_or_vid(0xff00) // PD SID must be used with descover identity
                        //.with_vdm_type(VDMType::Structured)
                        .with_vdm_version_major(VDMVersionMajor::Version2x.into())
                        .with_vdm_version_minor(VDMVersionMinor::Version20.into()),
                );
                vdm_header_vdo.to_bytes(&mut data[2..6]);
                identity.to_bytes(&mut data[6..10]);
                cert_stat.to_bytes(&mut data[10..14]);
                product.to_bytes(&mut data[14..18]);
                product_type_ufp.to_bytes(&mut data[18..22]);
                // if let Some(product_type_dfp) = product_type_dfp {
                //     // 20..24 are padding bytes
                //     product_type_dfp.to_bytes(&mut payload[24..32]);
                // }
                debug!("Sending VDM {:x}", data);
                self.driver.send(&data).await?;
                debug!("Sent VDM");
                Ok(())
            }
            Request::REQDiscoverSVIDS => {
                debug!("REQDiscoverSVIDS");
                let mut data = [0; 6];
                let header = self.new_data_header(self.tx_message_count, DataMessageType::VendorDefined, 1);
                header.to_bytes(&mut data[..2]);

                let vdm_header_vdo = VDMHeader::Structured(
                    VDMHeaderStructured(0)
                        .with_command(VDMCommand::DiscoverSVIDS)
                        .with_command_type(VDMCommandType::InitiatorREQ)
                        .with_object_position(0) // 0 Must be used for discover SVIDS
                        .with_standard_or_vid(0xff00) // PD SID must be used with discover SVIDS
                        .with_vdm_type(VDMType::Structured)
                        .with_vdm_version_major(VDMVersionMajor::Version10.into())
                        .with_vdm_version_minor(VDMVersionMinor::Version20.into()),
                );
                vdm_header_vdo.to_bytes(&mut data[2..]);

                debug!("Sending VDM {:x}", data);
                self.driver.send(&data).await?;
                debug!("Sent VDM");

                Ok(())
            }
            Request::REQDiscoverIdentity => {
                debug!("REQDiscoverIdentity");
                let mut data = [0; 6];
                let header = self.new_data_header(self.tx_message_count, DataMessageType::VendorDefined, 1);
                header.to_bytes(&mut data[..2]);

                let vdm_header_vdo = VDMHeader::Structured(
                    VDMHeaderStructured(0)
                        .with_command(VDMCommand::DiscoverIdentity)
                        .with_command_type(VDMCommandType::InitiatorREQ)
                        .with_object_position(0) // 0 Must be used for descover identity
                        .with_standard_or_vid(0xff00) // PD SID must be used with descover identity
                        .with_vdm_type(VDMType::Structured)
                        .with_vdm_version_major(VDMVersionMajor::Version10.into())
                        .with_vdm_version_minor(VDMVersionMinor::Version20.into()),
                );
                vdm_header_vdo.to_bytes(&mut data[2..]);

                debug!("Sending VDM {:x}", data);
                self.driver.send(&data).await?;
                debug!("Sent VDM");

                Ok(())
            }
        }
    }

    async fn request_power(&mut self, max_current: u16, index: usize) -> Result<(), DRIVER::TxError> {
        // Create 'request' message
        let mut data = [0; 6];

        let header = self.new_data_header(self.tx_message_count, DataMessageType::Request, 1);
        header.to_bytes(&mut data[..2]);

        self.set_request_payload_fixed(&mut data[2..], index as u8, max_current);

        // Send message
        self.driver.send(&data).await
    }

    fn set_request_payload_fixed(&mut self, payload: &mut [u8], obj_pos: u8, mut current: u16) {
        current = (current + 5) / 10;

        if current > 0x3ff {
            current = 0x3ff;
        }

        let obj_pos = obj_pos + 1;
        assert!(obj_pos > 0b0000 && obj_pos <= 0b1110);

        FixedVariableRequestDataObject(0)
            .with_raw_operating_current(current)
            .with_raw_max_operating_current(current)
            .with_object_position(obj_pos)
            .with_no_usb_suspend(true)
            .with_usb_communications_capable(true)
            .to_bytes(payload);
    }
}

pub trait Driver {
    type RxError: fmt::Debug;
    type TxError: fmt::Debug;

    fn wait_for_vbus(&self) -> impl Future<Output = ()>;

    fn receive(&mut self) -> impl Future<Output = Result<Message, Self::RxError>>;

    fn send(&mut self, data: &[u8]) -> impl Future<Output = Result<(), Self::TxError>>;
}

pub trait Timer {
    fn after_secs(seconds: u32) -> Self;

    fn after_millis(milliseconds: u32) -> Self;
}

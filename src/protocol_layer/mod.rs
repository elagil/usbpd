//! The protocol layer is controlled by the policy engine, and commands the PHY layer.
//!
//! Handles
//! - construction of messages,
//! - message timers and timeouts,
//! - message retry counters,
//! - reset operation,
//! - error handling,
//! - state behaviour.
//!
//! At this point in time, the protocol layer does not support extended messages.

pub mod message;

use core::future::Future;
use core::marker::PhantomData;

use defmt::{error, trace, Format};
use futures::future::{select, Either};
use futures::pin_mut;
use message::header::{ControlMessageType, DataMessageType, Header, MessageType};
use message::{Data, Message};

use crate::counters::{Counter, CounterType, Error as CounterError};
use crate::sink::{FixedSupplyRequest, PowerSourceRequest};
use crate::timers::{Timer, TimerType};
use crate::{Driver, DriverRxError, DriverTxError, PowerRole};

/// The protocol layer does not support extended messages.
///
/// This is the maximum standard message size.
const MAX_MESSAGE_SIZE: usize = 30;

/// Errors that can occur in the protocol layer.
#[derive(Debug, Format)]
pub enum Error {
    /// Port partner requested soft reset.
    SoftReset,
    /// Driver reported a hard reset.
    HardReset,
    /// A timeout during message reception.
    ReceiveTimeout,
    /// Transmission failed after the maximum number of allowed retries.
    TransmitRetriesExceeded,
    /// An unsupported message was received.
    UnsupportedMessage,
    /// An unexpected message was received.
    UnexpectedMessage,
}

enum RxError {
    /// Port partner requested soft reset.
    SoftReset,
    /// Driver reported a hard reset.
    HardReset,
    /// A timeout during message reception.
    ReceiveTimeout,
    /// An unsupported message was received.
    UnsupportedMessage,
    /// An unexpected message was received.
    UnexpectedMessage,
}

impl From<RxError> for Error {
    fn from(value: RxError) -> Self {
        match value {
            RxError::SoftReset => Error::SoftReset,
            RxError::HardReset => Error::HardReset,
            RxError::ReceiveTimeout => Error::ReceiveTimeout,
            RxError::UnsupportedMessage => Error::UnsupportedMessage,
            RxError::UnexpectedMessage => Error::UnexpectedMessage,
        }
    }
}

enum TxError {
    /// Driver reported a hard reset.
    HardReset,
}

impl From<TxError> for Error {
    fn from(value: TxError) -> Self {
        match value {
            TxError::HardReset => Error::HardReset,
        }
    }
}

#[derive(Debug)]
struct Counters {
    _busy: Counter,
    _caps: Counter, // Unused, optional.
    _discover_identity: Counter,
    rx_message: Option<Counter>,
    tx_message: Counter,
    retry: Counter,
}

impl Default for Counters {
    fn default() -> Self {
        Counters {
            _busy: Counter::new(CounterType::Busy),
            _caps: Counter::new(CounterType::Caps),
            _discover_identity: Counter::new(CounterType::DiscoverIdentity),
            rx_message: None,
            tx_message: Counter::new(CounterType::MessageId),
            retry: Counter::new(CounterType::Retry),
        }
    }
}

/// The USB PD protocol layer.
#[derive(Debug)]
pub struct ProtocolLayer<DRIVER: Driver, TIMER: Timer> {
    driver: DRIVER,
    counters: Counters,
    default_header: Header,
    _timer: PhantomData<TIMER>,
}

impl<DRIVER: Driver, TIMER: Timer> ProtocolLayer<DRIVER, TIMER> {
    /// Create a new protocol layer from a driver and default header.
    pub fn new(driver: DRIVER, default_header: Header) -> Self {
        Self {
            driver,
            counters: Default::default(),
            default_header,
            _timer: PhantomData,
        }
    }

    /// Reset the protocol layer.
    pub fn reset(&mut self) {
        self.counters = Default::default();
    }

    fn get_message_buffer() -> [u8; MAX_MESSAGE_SIZE] {
        [0u8; MAX_MESSAGE_SIZE]
    }

    /// Get a timer future for a given type.
    pub fn get_timer(timer_type: TimerType) -> impl Future<Output = ()> {
        TimerType::new::<TIMER>(timer_type)
    }

    /// Wait until a GoodCrc message is received, or a timeout occurs.
    async fn wait_for_good_crc(&mut self) -> Result<(), RxError> {
        trace!("Wait for GoodCrc");

        let receive_fut = async {
            let message = self.receive_message_inner().await?;

            return if matches!(
                message.header.message_type(),
                MessageType::Control(ControlMessageType::GoodCRC)
            ) {
                trace!(
                    "Received GoodCrc, TX message count: {}, expected: {}",
                    message.header.message_id(),
                    self.counters.tx_message.value()
                );
                if message.header.message_id() == self.counters.tx_message.value() {
                    // See spec, [6.7.1.1]
                    self.counters.retry.reset();
                    _ = self.counters.tx_message.increment();
                    Ok(())
                } else {
                    // Wrong transmitted message was acknowledged.
                    Err(RxError::UnexpectedMessage)
                }
            } else {
                Err(RxError::UnexpectedMessage)
            };
        };

        let timeout_fut = Self::get_timer(TimerType::CRCReceive);
        let result = {
            pin_mut!(timeout_fut);
            pin_mut!(receive_fut);

            match select(timeout_fut, receive_fut).await {
                Either::Left((_, _)) => Err(RxError::ReceiveTimeout),
                Either::Right((receive_result, _)) => receive_result,
            }
        };

        result
    }

    async fn transmit_inner(&mut self, buffer: &[u8]) -> Result<(), TxError> {
        loop {
            match self.driver.transmit(buffer).await {
                Ok(_) => return Ok(()),
                Err(DriverTxError::HardReset) => return Err(TxError::HardReset),
                Err(DriverTxError::Discarded) => {
                    // Retry transmission.
                }
            }
        }
    }

    /// Transmit a message.
    ///
    // GoodCrc message transmission is handled separately.
    // See `transmit_good_crc()` instead.
    pub async fn transmit(&mut self, message: Message) -> Result<(), Error> {
        assert_ne!(
            message.header.message_type(),
            MessageType::Control(ControlMessageType::GoodCRC)
        );

        trace!("Transmit message {}", message);
        self.counters.retry.reset();

        let mut buffer = Self::get_message_buffer();
        let size = message.to_bytes(&mut buffer);

        loop {
            match self.transmit_inner(&buffer[..size]).await {
                Ok(_) => {
                    match self.wait_for_good_crc().await {
                        Ok(()) => (),
                        Err(RxError::ReceiveTimeout) => match self.counters.retry.increment() {
                            Ok(_) => (),
                            Err(CounterError::Exceeded) => return Err(Error::TransmitRetriesExceeded),
                        },
                        Err(other) => return Err(other.into()),
                    }

                    trace!("Transmit success");
                    return Ok(());
                }
                Err(other) => return Err(other.into()),
            }
        }
    }

    /// Send a GoodCrc message to the port partner.
    async fn transmit_good_crc(&mut self) -> Result<(), Error> {
        trace!(
            "Transmit message GoodCrc for RX message count {}",
            self.counters.rx_message.unwrap().value()
        );

        let mut buffer = Self::get_message_buffer();

        let size = Message::new(Header::new_control(
            self.default_header,
            self.counters.rx_message.unwrap(), // A message must have been received before.
            ControlMessageType::GoodCRC,
        ))
        .to_bytes(&mut buffer);

        Ok(self.transmit_inner(&buffer[..size]).await?)
    }

    /// Receive a message.
    async fn receive_message_inner(&mut self) -> Result<Message, RxError> {
        loop {
            let mut buffer = Self::get_message_buffer();

            let length = match self.driver.receive(&mut buffer).await {
                Ok(length) => length,
                Err(DriverRxError::Discarded) => continue,
                Err(DriverRxError::HardReset) => return Err(RxError::HardReset),
            };

            let message = Message::from_bytes(&buffer[..length]);

            // Update specification revision, based on the received frame.
            self.default_header = self.default_header.with_spec_revision(message.header.spec_revision());

            match message.header.message_type() {
                MessageType::Control(ControlMessageType::Reserved) | MessageType::Data(DataMessageType::Reserved) => {
                    return Err(RxError::UnsupportedMessage)
                }
                MessageType::Control(ControlMessageType::SoftReset) => return Err(RxError::SoftReset),
                _ => (),
            }

            return Ok(message);
        }
    }

    /// Receive a message.
    pub async fn receive_message(&mut self) -> Result<Message, Error> {
        self.receive_message_inner().await.map_err(|err| err.into())
    }

    /// Updates the received message counter.
    ///
    /// If receiving the first message after protocol layer reset, copy its ID.
    /// Otherwise, compare the received ID with the stored ID. If they are equal, this is a retransmission.
    ///
    /// Returns `true`, if this was a retransmission.
    fn update_rx_message_counter(&mut self, rx_message: &Message) -> bool {
        match self.counters.rx_message.as_mut() {
            None => {
                trace!(
                    "Received first message after protocol layer reset with RX counter value {}",
                    rx_message.header.message_id()
                );
                self.counters.rx_message = Some(Counter::new_from_value(
                    CounterType::MessageId,
                    rx_message.header.message_id(),
                ));
                false
            }
            Some(counter) => {
                if rx_message.header.message_id() == counter.value() {
                    trace!("Received retransmission of RX counter value {}", counter.value());
                    true
                } else {
                    counter.set(rx_message.header.message_id());
                    false
                }
            }
        }
    }

    /// Wait until a message of one of the chosen types is received, or a timeout occurs.
    pub async fn receive_message_type(
        &mut self,
        message_types: &[MessageType],
        timer_type: TimerType,
    ) -> Result<Message, Error> {
        // GoodCrc message reception is handled separately.
        // See `wait_for_good_crc()` instead.
        for message_type in message_types {
            assert_ne!(*message_type, MessageType::Control(ControlMessageType::GoodCRC));
        }

        let receive_fut = async {
            loop {
                match self.receive_message_inner().await {
                    Ok(message) => {
                        // See spec, [6.7.1.2]
                        let is_retransmission = self.update_rx_message_counter(&message);

                        if !matches!(
                            message.header.message_type(),
                            MessageType::Control(ControlMessageType::GoodCRC)
                        ) {
                            self.transmit_good_crc().await?;
                        }

                        if is_retransmission {
                            // Retry reception.
                            continue;
                        }

                        return if message_types.contains(&message.header.message_type()) {
                            Ok(message)
                        } else {
                            Err(Error::UnexpectedMessage)
                        };
                    }
                    Err(RxError::UnexpectedMessage) => unreachable!(),
                    Err(other) => return Err(other.into()),
                }
            }
        };

        let timeout_fut = Self::get_timer(timer_type);

        pin_mut!(timeout_fut);
        pin_mut!(receive_fut);

        match select(timeout_fut, receive_fut).await {
            Either::Left((_, _)) => Err(Error::ReceiveTimeout),
            Either::Right((receive_result, _)) => receive_result,
        }
    }

    /// Perform a hard-reset procedure.
    ///
    // See spec, [6.7.1.1]
    pub async fn hard_reset(&mut self) -> Result<(), Error> {
        self.counters.tx_message.reset();
        self.counters.retry.reset();

        loop {
            match self.driver.transmit_hard_reset().await {
                Ok(_) | Err(DriverTxError::HardReset) => break,
                Err(DriverTxError::Discarded) => (),
            }
        }

        Ok(())
    }

    /// Wait for VBUS to be available.
    ///
    /// FIXME: Check what the logic should be.
    pub async fn wait_for_vbus(&mut self) {
        self.driver.wait_for_vbus().await
    }

    /// Wait for the source to provide its capabilities.
    pub async fn wait_for_source_capabilities(&mut self) -> Result<Message, Error> {
        self.receive_message_type(
            &[MessageType::Data(message::header::DataMessageType::SourceCapabilities)],
            TimerType::SinkWaitCap,
        )
        .await
    }

    /// Transmit a control message of the provided type.
    pub async fn transmit_control_message(&mut self, control_message_type: ControlMessageType) -> Result<(), Error> {
        let message = Message::new(Header::new_control(
            self.default_header,
            self.counters.tx_message,
            control_message_type,
        ));

        self.transmit(message).await
    }

    /// Request a certain power level from the source.
    pub async fn request_power(&mut self, supply: PowerSourceRequest) -> Result<(), Error> {
        match supply {
            PowerSourceRequest::FixedSupply(fixed_supply) => self.request_fixed_supply(fixed_supply).await,
        }
    }

    async fn request_fixed_supply(&mut self, supply: FixedSupplyRequest) -> Result<(), Error> {
        use message::pdo::FixedVariableRequestDataObject;
        use message::pdo::PowerSourceRequest::FixedSupply;

        // Only sinks can request from a supply.
        assert!(matches!(self.default_header.port_power_role(), PowerRole::Sink));

        let header = Header::new_data(
            self.default_header,
            self.counters.tx_message,
            DataMessageType::Request,
            1,
        );

        let mut current = supply.current_10ma;

        if current > 0x3ff {
            error!("Clamping invalid current: {} mA", 10 * current);
            current = 0x3ff;
        }

        let obj_position = supply.index + 1;
        assert!(obj_position > 0b0000 && obj_position <= 0b1110);

        let mut message = Message::new(header);
        message.data = Some(Data::PowerSourceRequest(FixedSupply(
            FixedVariableRequestDataObject(0)
                .with_raw_operating_current(current)
                .with_raw_max_operating_current(current)
                .with_object_position(obj_position)
                .with_no_usb_suspend(true)
                .with_usb_communications_capable(true),
        )));

        self.transmit(message).await
    }
}

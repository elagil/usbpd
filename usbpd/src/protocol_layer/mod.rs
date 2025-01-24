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

use defmt::{debug, error, trace, Format};
use futures::future::{select, Either};
use futures::{pin_mut, FutureExt};
use message::header::{ControlMessageType, DataMessageType, Header, MessageType};
use message::{Data, Message};

use crate::counters::{Counter, CounterType, Error as CounterError};
use crate::sink::{FixedSupply, PowerSourceRequest};
use crate::timers::{Timer, TimerType};
use crate::{Driver, PowerRole, RxError, TxError};

/// The protocol layer does not support extended messages.
///
/// This is the maximum standard message size.
const MAX_MESSAGE_SIZE: usize = 30;

// FIXME: Internal/externally propagated errors

/// Errors that can occur in the protocol layer.
#[derive(Debug, Format)]
pub enum Error {
    /// Timeouts, e.g. while waiting for GoodCrc.
    /// FIXME: Must never be propagated outside.
    Timeout,
    /// Port partner requested soft reset.
    SoftReset,
    /// An unsupported message was received.
    UnsupportedMessage,
    /// An unexpected message was received.
    UnexpectedMessage,
    /// Message retransmission
    Retransmission,
    /// Counter overruns, e.g. for hard-reset.
    Counter(CounterError),
    /// Errors during reception.
    RxError(RxError),
    /// Errors during transmission.
    TxError(TxError),
}

/// Errors that can occur in the protocol layer.
#[derive(Debug, Format)]
pub enum ExternalError {
    /// Port partner requested soft reset.
    SoftReset,
    /// Driver reported a hard reset.
    HardReset,
    /// An unsupported message was received.
    UnsupportedMessage,
    /// An unexpected message was received.
    UnexpectedMessage,
    /// Counter overruns, e.g. for hard-reset.
    Counter(CounterError),
}

/// Errors that can occur in the protocol layer.
#[derive(Debug, Format)]
pub enum InternalError {
    /// Port partner requested soft reset.
    SoftReset,
    /// Timeouts, e.g. while waiting for GoodCrc.
    Timeout,
    /// An unsupported message was received.
    UnsupportedMessage,
    /// An unexpected message was received.
    UnexpectedMessage,
    /// Message retransmission
    Retransmission,
    /// Counter overruns, e.g. for hard-reset.
    Counter(CounterError),
    /// Errors during reception.
    RxError(RxError),
    /// Errors during transmission.
    TxError(TxError),
}

impl From<RxError> for Error {
    fn from(value: RxError) -> Self {
        Error::RxError(value)
    }
}

impl From<TxError> for Error {
    fn from(value: TxError) -> Self {
        Error::TxError(value)
    }
}

impl From<CounterError> for Error {
    fn from(value: CounterError) -> Self {
        Error::Counter(value)
    }
}

#[derive(Debug)]
pub struct Counters {
    busy: Counter,
    caps: Counter,
    discover_identity: Counter,
    hard_reset: Counter,
    rx_message: Option<Counter>,
    tx_message: Counter,
    retry: Counter,
}

impl Default for Counters {
    fn default() -> Self {
        Counters {
            busy: Counter::new(CounterType::Busy),
            caps: Counter::new(CounterType::Caps),
            discover_identity: Counter::new(CounterType::DiscoverIdentity),
            hard_reset: Counter::new(CounterType::HardReset),
            rx_message: None,
            tx_message: Counter::new(CounterType::MessageId),
            retry: Counter::new(CounterType::Retry),
        }
    }
}

#[derive(Debug)]
pub struct ProtocolLayer<DRIVER: Driver, TIMER: Timer> {
    driver: DRIVER,
    counters: Counters,
    default_header: Header,
    _timer: PhantomData<TIMER>,
}

impl<DRIVER: Driver, TIMER: Timer> ProtocolLayer<DRIVER, TIMER> {
    pub fn new(driver: DRIVER, default_header: Header) -> Self {
        Self {
            driver,
            counters: Default::default(),
            default_header,
            _timer: PhantomData,
        }
    }

    /// Reset the protocol layer.
    pub fn reset(self) -> DRIVER {
        let driver = self.driver;
        driver
    }

    fn get_message_buffer() -> [u8; MAX_MESSAGE_SIZE] {
        [0u8; MAX_MESSAGE_SIZE]
    }

    pub fn get_timer(timer_type: TimerType) -> impl Future<Output = ()> {
        TimerType::new::<TIMER>(timer_type)
    }

    /// Wait until a GoodCrc message is received, or a timeout occurs.
    async fn wait_for_good_crc(&mut self) -> Result<(), Error> {
        trace!("Wait for GoodCrc");

        let receive_fut = async {
            loop {
                match self.receive().await {
                    Ok(message) => {
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
                                Err(Error::UnexpectedMessage)
                            }
                        } else {
                            Err(Error::UnexpectedMessage)
                        };
                    }
                    Err(Error::RxError(RxError::Discarded)) => {
                        // Retry reception.
                    }
                    Err(other) => return Err(other.into()),
                }
            }
        };

        let timeout_fut = Self::get_timer(TimerType::CRCReceive);
        let result = {
            pin_mut!(timeout_fut);
            pin_mut!(receive_fut);

            match select(timeout_fut, receive_fut).await {
                Either::Left((_, _)) => Err(Error::Timeout),
                Either::Right((receive_result, _)) => receive_result,
            }
        };

        result
    }

    async fn transmit_inner(&mut self, buffer: &[u8]) -> Result<(), Error> {
        self.driver.transmit(buffer).await?;
        self.wait_for_good_crc().await
    }

    pub async fn transmit(&mut self, message: Message) -> Result<(), Error> {
        trace!("Transmit message {}", message);

        // GoodCrc message transmission is handled separately.
        // See `transmit_good_crc()` instead.
        assert_ne!(
            message.header.message_type(),
            MessageType::Control(ControlMessageType::GoodCRC)
        );

        let mut buffer = Self::get_message_buffer();
        let size = message.to_bytes(&mut buffer);

        loop {
            match self.transmit_inner(&buffer[..size]).await {
                Ok(_) => {
                    // Transmitted the message, and received a GoodCrc.
                    trace!("Transmit success");
                    return Ok(());
                }
                Err(Error::Counter(_)) => unreachable!("No counters shall be incremented during transmit"),
                Err(Error::Timeout) => self.counters.retry.increment()?,
                Err(Error::RxError(rx_error)) => match rx_error {
                    RxError::Discarded => {
                        // FIXME: What to do?
                    }
                    other => return Err(other.into()),
                },
                Err(Error::TxError(tx_error)) => match tx_error {
                    TxError::Discarded => {
                        // FIXME: What to do?
                    }
                    other => return Err(other.into()),
                },
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

        Ok(self.driver.transmit(&buffer[..size]).await?)
    }

    /// Mostly used internally by the protocol layer itself.
    ///
    /// However, the policy engine can also call this when not expecting a particular message.
    pub async fn receive(&mut self) -> Result<Message, Error> {
        let mut buffer = Self::get_message_buffer();
        let length = self.driver.receive(&mut buffer).await?;
        let message = Message::parse(&buffer[..length]);

        // Update specification revision, based on the received frame.
        self.default_header = self.default_header.with_spec_revision(message.header.spec_revision());

        match message.header.message_type() {
            MessageType::Control(ControlMessageType::Reserved) | MessageType::Data(DataMessageType::Reserved) => {
                return Err(Error::UnsupportedMessage)
            }
            MessageType::Control(ControlMessageType::SoftReset) => return Err(Error::SoftReset),
            _ => (),
        }

        Ok(message)
    }

    /// Wait until a message of one of the chosen types is received, or a timeout occurs.
    async fn wait_for_any_message_inner(
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
                match self.receive().await {
                    Ok(message) => {
                        // See spec, [6.7.1.2]

                        let is_retransmission = match self.counters.rx_message.as_mut() {
                            None => {
                                trace!(
                                    "Received first message after protocol layer reset with RX counter value {}",
                                    message.header.message_id()
                                );
                                self.counters.rx_message = Some(Counter::new_from_value(
                                    CounterType::MessageId,
                                    message.header.message_id(),
                                ));
                                false
                            }
                            Some(counter) => {
                                if message.header.message_id() == counter.value() {
                                    trace!("Received retransmission of RX counter value {}", counter.value());
                                    true
                                } else {
                                    counter.set(message.header.message_id());
                                    false
                                }
                            }
                        };

                        // In case of retransmission: Discard message, but send GoodCrc regardless.
                        if !matches!(
                            message.header.message_type(),
                            MessageType::Control(ControlMessageType::GoodCRC)
                        ) {
                            self.transmit_good_crc().await?;
                        }

                        return if is_retransmission {
                            Err(Error::Retransmission)
                        } else if message_types.contains(&message.header.message_type()) {
                            Ok(message)
                        } else {
                            Err(Error::UnexpectedMessage)
                        };
                    }
                    Err(Error::RxError(RxError::Discarded)) => {
                        // Retry reception.
                    }
                    Err(other) => return Err(other.into()),
                }
            }
        };

        let timeout_fut = Self::get_timer(timer_type);

        pin_mut!(timeout_fut);
        pin_mut!(receive_fut);

        match select(timeout_fut, receive_fut).await {
            Either::Left((_, _)) => Err(Error::Timeout),
            Either::Right((receive_result, _)) => receive_result,
        }
    }

    /// Wait until a message of one of the chosen types is received, or the retry count is exceeded.
    pub async fn wait_for_any_message(
        &mut self,
        message_types: &[MessageType],
        timer_type: TimerType,
    ) -> Result<Message, Error> {
        loop {
            match self.wait_for_any_message_inner(message_types, timer_type).await {
                Ok(message) => return Ok(message),
                Err(Error::Timeout) => {
                    // May cause a counter overrun (retries exceeded).
                    self.counters.retry.increment()?
                }
                Err(other) => return Err(other.into()),
            }
        }
    }

    pub async fn hard_reset(&mut self) -> Result<(), Error> {
        // See spec, [6.7.1.1]
        self.counters.tx_message.reset();
        self.counters.retry.reset();

        self.counters.hard_reset.increment()?;

        // FIXME: Perform actual hard reset.
        self.driver.transmit_hard_reset().await?;

        Ok(())
    }

    /// Wait for VBUS to be available.
    ///
    /// FIXME: Add timeout?
    pub async fn wait_for_vbus(&mut self) {
        self.driver.wait_for_vbus().await
    }

    pub async fn wait_for_source_capabilities(&mut self) -> Result<Message, Error> {
        self.wait_for_any_message(
            &[MessageType::Data(message::header::DataMessageType::SourceCapabilities)],
            TimerType::SourceCapability,
        )
        .await
    }

    pub async fn transmit_control_message(&mut self, control_message_type: ControlMessageType) -> Result<(), Error> {
        let message = Message::new(Header::new_control(
            self.default_header,
            self.counters.tx_message,
            control_message_type,
        ));

        self.transmit(message).await
    }

    pub async fn request_supply(&mut self, supply: PowerSourceRequest) -> Result<(), Error> {
        match supply {
            PowerSourceRequest::FixedSupply(fixed_supply) => self.request_fixed_supply(fixed_supply).await,
            _ => unimplemented!(),
        }
    }

    async fn request_fixed_supply(&mut self, supply: FixedSupply) -> Result<(), Error> {
        use message::pdo::{FixedVariableRequestDataObject, PowerSourceRequest::FixedSupply};

        // Only sinks can request from a supply.
        assert!(matches!(self.default_header.port_power_role(), PowerRole::Sink));

        let header = Header::new_data(
            self.default_header,
            self.counters.tx_message,
            DataMessageType::Request,
            1,
        );

        // Round to nearest 10 mA.
        let mut current = (supply.raw_max_current + 5) / 10;

        if current > 0x3ff {
            current = 0x3ff;
        }

        let obj_position = supply.index + 1;
        assert!(obj_position > 0b0000 && obj_position <= 0b1110);

        let mut message = Message::new(header);
        message.data = Some(Data::Request(FixedSupply(
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

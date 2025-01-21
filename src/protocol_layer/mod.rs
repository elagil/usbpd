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
use crate::timers::{Timer, TimerType};
use crate::{Driver, RxError, TxError};

/// The protocol layer does not support extended messages.
///
/// This is the maximum standard message size.
const MAX_MESSAGE_SIZE: usize = 30;

/// Errors that can occur in the protocol layer.
#[derive(Debug, Format)]
pub enum Error {
    /// Timeouts, e.g. while waiting for GoodCrc.
    Timeout,
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
    rx_message: Counter,
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
            rx_message: Counter::new(CounterType::MessageId),
            tx_message: Counter::new(CounterType::MessageId),
            retry: Counter::new(CounterType::Retry),
        }
    }
}

#[derive(Debug)]
pub struct ProtocolLayer<DRIVER: Driver, TIMER: Timer> {
    driver: DRIVER,
    counters: Counters,
    header_template: Header,
    _timer: PhantomData<TIMER>,
}

impl<DRIVER: Driver, TIMER: Timer> ProtocolLayer<DRIVER, TIMER> {
    pub fn new(driver: DRIVER, default_header: Header) -> Self {
        Self {
            driver,
            counters: Default::default(),
            header_template: default_header,
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

    pub fn get_timer(timer_type: TimerType) -> impl Future<Output = ()> {
        TimerType::new::<TIMER>(timer_type)
    }

    /// Wait until a GoodCrc message is received, or a timeout occurs.
    async fn wait_for_good_crc(&mut self) -> Result<(), Error> {
        trace!("Wait for GoodCrc");

        let receive_fut = async {
            loop {
                if let Ok(message) = self.receive().await {
                    if matches!(
                        message.header.message_type(),
                        MessageType::Control(ControlMessageType::GoodCRC)
                    ) {
                        trace!(
                            "Received GoodCrc, TX message count: {}, expected: {}",
                            message.header.message_id(),
                            self.counters.tx_message.value()
                        );
                        if message.header.message_id() == self.counters.tx_message.value() {
                            _ = self.counters.tx_message.increment();
                            return Ok(());
                        }
                    } else {
                        error!("Unexpected message type: {}", message.header.message_type());
                    }
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
                    RxError::Crc | RxError::Overrun => {
                        // FIXME: What to do?
                    }
                    RxError::HardReset => self.handle_hard_reset()?,
                },
                Err(Error::TxError(tx_error)) => match tx_error {
                    TxError::Discarded => {
                        // FIXME: What to do?
                    }
                    TxError::HardReset => self.handle_hard_reset()?,
                },
            }
        }
    }

    /// Send a GoodCrc message to the port partner.
    async fn transmit_good_crc(&mut self) -> Result<(), Error> {
        trace!("Transmit message GoodCrc");

        let mut buffer = Self::get_message_buffer();

        let size = Message::new(Header::new_control(
            self.header_template,
            self.counters.rx_message,
            ControlMessageType::GoodCRC,
        ))
        .to_bytes(&mut buffer);

        Ok(self.driver.transmit(&buffer[..size]).await?)
    }

    async fn receive(&mut self) -> Result<Message, RxError> {
        let mut buffer = Self::get_message_buffer();
        let length = self.driver.receive(&mut buffer).await?;
        let message = Message::parse(&buffer[..length]);

        // Update specification revision, based on the received frame.
        self.header_template = self.header_template.with_spec_revision(message.header.spec_revision());

        Ok(message)
    }

    /// Wait until a message of a certain type is received, or a timeout occurs.
    pub async fn wait_for_message(
        &mut self,
        message_type: MessageType,
        timeout_fut: impl Future<Output = ()>,
    ) -> Result<Message, Error> {
        // GoodCrc message reception is handled separately.
        // See `wait_for_good_crc()` instead.
        assert_ne!(message_type, MessageType::Control(ControlMessageType::GoodCRC));

        let receive_fut = async {
            loop {
                if let Ok(message) = self.receive().await {
                    if !matches!(
                        message.header.message_type(),
                        MessageType::Control(ControlMessageType::GoodCRC)
                    ) {
                        _ = self.counters.rx_message.set(message.header.message_id());
                        self.transmit_good_crc().await?;
                    }

                    if message_type == message.header.message_type() {
                        return Ok(message);
                    } else {
                        error!("Unexpected message type: {}", message.header.message_type());
                    }
                }
            }
        };

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

    fn handle_hard_reset(&mut self) -> Result<(), Error> {
        self.counters.hard_reset.increment()?;

        // FIXME: Perform actual hard reset.
        Ok(())
    }

    /// Wait for VBUS to be available.
    ///
    /// FIXME: Add timeout?
    pub async fn wait_for_vbus(&mut self) {
        self.driver.wait_for_vbus().await
    }

    pub async fn wait_for_source_capabilities(&mut self) -> Result<Message, Error> {
        self.wait_for_message(
            MessageType::Data(message::header::DataMessageType::SourceCapabilities),
            Self::get_timer(TimerType::SourceCapability),
        )
        .await
    }

    pub async fn request_power(&mut self, max_current: u16, index: u8) -> Result<(), Error> {
        use message::pdo::{FixedVariableRequestDataObject, Request::FixedSupply};

        let header = Header::new_data(
            self.header_template,
            self.counters.tx_message,
            DataMessageType::Request,
            1,
        );

        // FIXME: explain.
        let mut current = (max_current + 5) / 10;

        if current > 0x3ff {
            current = 0x3ff;
        }

        let obj_position = index + 1;
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

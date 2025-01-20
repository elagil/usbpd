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
//! At this point in time, this does not support extended messages.

pub mod counters;
pub mod message;

use core::future::Future;

use counters::{Counter, CounterType};
use defmt::{error, trace};
use futures::future::{select, Either};
use futures::pin_mut;
use message::header::{ControlMessageType, Header, MessageType};
use message::Message;

use crate::{Driver, RxError, TxError};

enum Error {
    Timeout,
    RxError,
    TxError,
    DriverRxError(RxError),
    DriverTxError(TxError),
}

#[derive(Debug)]
pub struct ProtocolLayer<DRIVER: Driver> {
    driver: DRIVER,

    busy_count: Counter,
    caps_count: Counter,
    discover_identity_count: Counter,
    hard_reset_count: Counter,
    rx_message_count: Counter,
    tx_message_count: Counter,
    retry_count: Counter,

    default_header: Header,
}

impl<DRIVER: Driver> ProtocolLayer<DRIVER> {
    pub fn new(driver: DRIVER, default_header: Header) -> Self {
        Self {
            driver,

            busy_count: Counter::new(CounterType::Busy),
            caps_count: Counter::new(CounterType::Caps),
            discover_identity_count: Counter::new(CounterType::DiscoverIdentity),
            hard_reset_count: Counter::new(CounterType::HardReset),
            rx_message_count: Counter::new(CounterType::MessageId),
            tx_message_count: Counter::new(CounterType::MessageId),
            retry_count: Counter::new(CounterType::Retry),

            default_header,
        }
    }

    pub async fn transmit(
        &mut self,
        message_type: MessageType,
        timeout_fut: impl Future<Output = ()>,
    ) -> Result<(), Error> {
        Ok(())
    }

    async fn good_crc(&mut self) -> Result<(), Error> {
        Ok(())
    }

    /// Wait for a message of a certain type, until a timeout occurs.
    async fn receive(
        &mut self,
        message_type: MessageType,
        timeout_fut: impl Future<Output = ()>,
    ) -> Result<Message, Error> {
        let receive_fut = async {
            let mut buffer = [0u8; 30];

            match message_type {
                MessageType::Control(ControlMessageType::GoodCRC) => loop {
                    if let Ok(message) = self.driver.receive(&mut buffer).await {
                        if matches!(
                            message.header.message_type(),
                            MessageType::Control(ControlMessageType::GoodCRC)
                        ) {
                            trace!(
                                "Received GoodCrc, TX message count: {}, expected: {}",
                                message.header.message_id(),
                                self.tx_message_count.value()
                            );
                            if message.header.message_id() == self.tx_message_count.value() {
                                _ = self.tx_message_count.increment();
                                return Ok(message);
                            } else {
                                return Err(Error::TxError);
                            }
                        } else {
                            error!("Unexpected message type: {}", message.header.message_type());
                        }
                    }
                },
                _ => loop {
                    if let Ok(message) = self.driver.receive(&mut buffer).await {
                        if !matches!(
                            message.header.message_type(),
                            MessageType::Control(ControlMessageType::GoodCRC)
                        ) {
                            _ = self.rx_message_count.set(message.header.message_id());
                            self.good_crc().await?;
                        }

                        if message_type == message.header.message_type() {
                            return Ok(message);
                        } else {
                            error!("Unexpected message type: {}", message.header.message_type());
                        }
                    }
                },
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
}

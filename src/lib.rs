#![no_std]
pub mod counters;
pub mod protocol_layer;
pub mod sink;
pub mod timers;

#[macro_use]
extern crate uom;

use core::fmt::Debug;
use core::future::Future;

use protocol_layer::message::Message;

/// Receive Error.
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum RxError {
    /// Incorrect CRC or truncated message (a line becoming static before EOP is met).
    Crc,

    /// Provided buffer was too small for the received message.
    Overrun,

    /// Hard Reset received before or during reception.
    HardReset,
}

/// Transmit Error.
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum TxError {
    /// Concurrent receive in progress or excessive noise on the line.
    Discarded,

    /// Hard Reset received before or during transmission.
    HardReset,
}

pub trait Driver {
    fn wait_for_vbus(&self) -> impl Future<Output = ()>;

    fn receive(&mut self, buffer: &mut [u8]) -> impl Future<Output = Result<usize, RxError>>;

    fn transmit(&mut self, data: &[u8]) -> impl Future<Output = Result<(), TxError>>;
}

#[derive(Clone, Copy, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum PowerRole {
    Source,
    Sink,
}

impl From<bool> for PowerRole {
    fn from(value: bool) -> Self {
        match value {
            false => Self::Sink,
            true => Self::Source,
        }
    }
}

impl From<PowerRole> for bool {
    fn from(role: PowerRole) -> bool {
        match role {
            PowerRole::Sink => false,
            PowerRole::Source => true,
        }
    }
}

#[derive(Clone, Copy, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum DataRole {
    Ufp,
    Dfp,
}

impl From<bool> for DataRole {
    fn from(value: bool) -> Self {
        match value {
            false => Self::Ufp,
            true => Self::Dfp,
        }
    }
}

impl From<DataRole> for bool {
    fn from(role: DataRole) -> bool {
        match role {
            DataRole::Ufp => false,
            DataRole::Dfp => true,
        }
    }
}

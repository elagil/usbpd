//! USB PD library.
//!
//! Includes capabilities for implementing:
//! - SPR sink
//!
//! Provides a policy engine (depending on selected capability), protocol layer,
//! and relevant traits for the user applation to implement.
#![no_std]
#![warn(missing_docs)]

pub mod counters;
pub mod protocol_layer;
pub mod sink;
pub mod timers;

#[macro_use]
extern crate uom;

use core::fmt::Debug;
use core::future::Future;

/// Receive Error.
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum DriverRxError {
    /// Received message discarded, e.g. due to CRC errors.
    Discarded,

    /// Hard Reset received before or during reception.
    HardReset,
}

/// Transmit Error.
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum DriverTxError {
    /// Concurrent receive in progress or excessive noise on the line.
    Discarded,

    /// Hard Reset received before or during transmission.
    HardReset,
}

/// Driver trait, through which the protocol layer talks to the PHY.
pub trait Driver {
    /// Wait for availability of VBus voltage.
    fn wait_for_vbus(&self) -> impl Future<Output = ()>;

    /// Receive a packet.
    fn receive(&mut self, buffer: &mut [u8]) -> impl Future<Output = Result<usize, DriverRxError>>;

    /// Transmit a packet.
    fn transmit(&mut self, data: &[u8]) -> impl Future<Output = Result<(), DriverTxError>>;

    /// Transmit a hard reset signal.
    fn transmit_hard_reset(&mut self) -> impl Future<Output = Result<(), DriverTxError>>;
}

/// The power role of the port.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum PowerRole {
    /// The port is a source.
    /// FIXME: Implement
    Source,
    /// The port is a sink.
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

/// The data role of the port.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum DataRole {
    /// The port is an upstream-facing port.
    Ufp,
    /// The port is a downstream-facing port.
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

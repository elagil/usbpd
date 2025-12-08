//! USB PD library traits.
//!
//! Provides a driver trait that allows to add support for various USB PD PHYs.
#![cfg_attr(not(test), no_std)]
#![warn(missing_docs)]
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
    /// If this is `true`, the protocol layer will not send its own
    /// GoodCRC messages and will instead rely on the hardware.
    const HAS_AUTO_GOOD_CRC: bool = false;

    /// If this is `true`, the hardware automatically retries transmission
    /// when no GoodCRC is received. The protocol layer will skip its own
    /// retry loop but still call wait_for_good_crc() to consume the GoodCRC
    /// from the FIFO and validate the message ID.
    const HAS_AUTO_RETRY: bool = false;

    /// Wait for availability of VBus voltage.
    fn wait_for_vbus(&self) -> impl Future<Output = ()>;

    /// Receive a packet.
    fn receive(&mut self, buffer: &mut [u8]) -> impl Future<Output = Result<usize, DriverRxError>>;

    /// Transmit a packet.
    fn transmit(&mut self, data: &[u8]) -> impl Future<Output = Result<(), DriverTxError>>;

    /// Transmit a hard reset signal.
    fn transmit_hard_reset(&mut self) -> impl Future<Output = Result<(), DriverTxError>>;
}

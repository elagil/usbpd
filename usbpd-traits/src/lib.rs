//! USB PD library traits.
//!
//! Provides a driver trait that allows to add support for various USB PD PHYs.
#![cfg_attr(not(test), no_std)]
#![warn(missing_docs)]
use core::future::Future;

/// Error type of driver receive operations.
#[derive(Debug, Clone, Copy, thiserror::Error)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum DriverRxError {
    /// Received message discarded, e.g. due to CRC errors.
    #[error("received message discarded")]
    Discarded,

    /// Hard Reset received before or during reception.
    #[error("hard reset received")]
    HardReset,
}

/// Error type of driver transmit operations.
#[derive(Debug, Clone, Copy, thiserror::Error)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum DriverTxError {
    /// Concurrent receive in progress or excessive noise on the line.
    #[error("transmission discarded")]
    Discarded,

    /// Hard Reset received before or during transmission.
    #[error("hard reset received")]
    HardReset,
}

/// Driver trait, through which the protocol layer talks to the PHY.
pub trait Driver {
    /// If this is `true`, the protocol layer will not send its own
    /// GoodCRC messages and will instead rely on the hardware.
    const HAS_AUTO_GOOD_CRC: bool = false;

    /// If this is `true`, the hardware automatically retries transmission
    /// when no GoodCRC is received. The protocol layer will skip its own
    /// retry loop and call driver.transmit() directly. On success, it
    /// maintains protocol state (TX message counter) without calling
    /// wait_for_good_crc(), since the hardware already verified GoodCRC.
    const HAS_AUTO_RETRY: bool = false;

    /// Wait for availability of VBus voltage.
    fn wait_for_vbus(&mut self) -> impl Future<Output = ()>;

    /// Receive a packet.
    fn receive(&mut self, buffer: &mut [u8]) -> impl Future<Output = Result<usize, DriverRxError>>;

    /// Transmit a packet.
    fn transmit(&mut self, data: &[u8]) -> impl Future<Output = Result<(), DriverTxError>>;

    /// Transmit a hard reset signal.
    fn transmit_hard_reset(&mut self) -> impl Future<Output = Result<(), DriverTxError>>;
}

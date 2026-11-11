//! USB Type-C configuration channel (CC) library traits.
//!
//! Provides the hardware traits through which the `usbcc` state machine talks to a port.
//! Each trait covers one part of the port and can be implemented independently:
//!
//! - [`CcPhy`]: the CC front end (terminations and CC line states).
//! - [`VbusSense`]: VBUS voltage measurement.
//! - [`VbusSwitch`]: the VBUS power path.
//! - [`VconnSwitch`]: the VCONN supply (optional, see [`NoVconn`]).
//!
//! Section references are to the USB Type-C Cable and Connector Specification, Release 2.5.
#![cfg_attr(not(test), no_std)]
#![warn(missing_docs)]

use core::future::Future;

/// One of the two CC pins of the receptacle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum CcPin {
    /// CC1 (receptacle pin A5).
    Cc1,
    /// CC2 (receptacle pin B5).
    Cc2,
}

/// USB Type-C Current advertisement of an Rp termination (Section 4.6.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum RpLevel {
    /// Default USB Power.
    Default,
    /// 1.5 A.
    Current1A5,
    /// 3.0 A.
    Current3A0,
}

/// Termination that this port presents on a CC pin (Table 4-27).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Termination {
    /// No termination (high impedance).
    Open,
    /// Pull-down, presented by a Sink.
    Rd,
    /// Pull-up with the given current advertisement, presented by a Source.
    Rp(RpLevel),
    /// VCONN discharge resistor, presented in UnattachedWait.SRC on the pin that supplied VCONN
    /// (Section 4.5.2.2.6).
    Rdch,
}

/// State that this port detects on a CC pin.
///
/// Already interpreted for the termination the port presents on that pin: a pin terminated with
/// [`Termination::Rd`] reports [`CcState::Open`] or [`CcState::Rp`], a pin terminated with
/// [`Termination::Rp`] reports [`CcState::Open`], [`CcState::Rd`] or [`CcState::Ra`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum CcState {
    /// Nothing detected (SRC.Open, SNK.Open).
    Open,
    /// Ra detected, e.g. a powered cable or an audio adapter accessory (SRC.Ra).
    Ra,
    /// Rd detected, i.e. a Sink (SRC.Rd).
    Rd,
    /// Rp detected, i.e. a Source with the given current advertisement (SNK.Rp).
    Rp(RpLevel),
}

/// The CC front end of a port.
///
/// Presents terminations on the CC pins and reports what it detects on them.
pub trait CcPhy {
    /// Error type of the implementation.
    type Error;

    /// Presents the given terminations on CC1 and CC2.
    ///
    /// Implementations that can't control the pins independently return an error for
    /// combinations they don't support.
    fn set_termination(&mut self, cc1: Termination, cc2: Termination) -> impl Future<Output = Result<(), Self::Error>>;

    /// Returns the current state of CC1 and CC2.
    ///
    /// Marks the state as seen: [`CcPhy::wait_for_cc_change`] reports changes after this call.
    fn cc_state(&mut self) -> impl Future<Output = Result<(CcState, CcState), Self::Error>>;

    /// Waits until the state of CC1 or CC2 may have changed since it was last read, and returns
    /// the current state of CC1 and CC2.
    ///
    /// Returns immediately if a change happened since the last read, by this method or by
    /// [`CcPhy::cc_state`]. Marks the returned state as seen. Spurious returns, with an unchanged
    /// state, are allowed; the caller compares and debounces.
    fn wait_for_cc_change(&mut self) -> impl Future<Output = Result<(CcState, CcState), Self::Error>>;
}

/// VBUS voltage measurement.
pub trait VbusSense {
    /// Error type of the implementation.
    type Error;

    /// Returns the current VBUS voltage in mV.
    fn vbus_mv(&mut self) -> impl Future<Output = Result<u32, Self::Error>>;

    /// Waits until the VBUS voltage is below `low_mv` or above `high_mv`.
    ///
    /// Returns immediately if it already is. Implementations can use an ADC analog watchdog, a
    /// comparator or polling.
    fn wait_vbus_outside(&mut self, low_mv: u32, high_mv: u32) -> impl Future<Output = Result<(), Self::Error>>;
}

/// The VBUS power path of a port.
pub trait VbusSwitch {
    /// Error type of the implementation.
    type Error;

    /// Tells the power path whether the port is attached.
    ///
    /// Called with `true` on entering Attached.SRC or Attached.SNK, before any other switch is
    /// changed, and with `false` on returning to an unattached state, after VBUS is off. While
    /// unattached, hardware may enter a low-power detection mode (e.g. LowPower on the
    /// TCPP03-M20). The default implementation does nothing.
    fn set_attached(&mut self, attached: bool) -> impl Future<Output = Result<(), Self::Error>> {
        let _ = attached;
        async { Ok(()) }
    }

    /// Connects (`true`) or disconnects (`false`) the sink path, through which the port consumes
    /// VBUS.
    ///
    /// `usbcc` never connects the sink path of a port configured as Source-only. Implementations
    /// for hardware without a sink path may do nothing for `false` and panic for `true`, which
    /// indicates a misconfiguration.
    fn set_sink(&mut self, on: bool) -> impl Future<Output = Result<(), Self::Error>>;

    /// Connects (`true`) or disconnects (`false`) the source path, through which the port supplies
    /// VBUS.
    ///
    /// `usbcc` never connects the source path of a port configured as Sink-only. Implementations
    /// for hardware without a source path may do nothing for `false` and panic for `true`, which
    /// indicates a misconfiguration.
    fn set_source(&mut self, on: bool) -> impl Future<Output = Result<(), Self::Error>>;

    /// Enables (`true`) or disables (`false`) the VBUS discharge load.
    ///
    /// A Source discharges VBUS to vSafe0V after turning it off, e.g. in AttachWait.SRC
    /// (Section 4.5.2.2.8). `usbcc` never enables it on a port configured as Sink-only;
    /// implementations for hardware without a discharge load may do nothing.
    fn set_vbus_discharge(&mut self, on: bool) -> impl Future<Output = Result<(), Self::Error>>;
}

/// The VCONN supply of a port.
pub trait VconnSwitch {
    /// Error type of the implementation.
    type Error;

    /// Supplies VCONN on the given CC pin, or on none.
    fn set_vconn(&mut self, pin: Option<CcPin>) -> impl Future<Output = Result<(), Self::Error>>;

    /// Enables (`true`) or disables (`false`) the VCONN discharge (Section 4.5.2.2.6).
    fn set_vconn_discharge(&mut self, on: bool) -> impl Future<Output = Result<(), Self::Error>>;
}

/// [`VconnSwitch`] for ports that never source VCONN.
///
/// VCONN is optional for ports without USB TX/RX signaling (Section 4.5.2.2.9.1); whether a port
/// sources VCONN is part of the `usbcc` configuration.
///
/// # Panics
///
/// [`VconnSwitch::set_vconn`] panics when asked to supply VCONN, which means that the port is
/// configured to source VCONN but has no VCONN switch.
#[derive(Debug, Clone, Copy, Default)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct NoVconn;

impl VconnSwitch for NoVconn {
    type Error = core::convert::Infallible;

    async fn set_vconn(&mut self, pin: Option<CcPin>) -> Result<(), Self::Error> {
        assert!(
            pin.is_none(),
            "VCONN requested from `NoVconn`: the port is configured to source VCONN but has no VCONN switch"
        );
        Ok(())
    }

    async fn set_vconn_discharge(&mut self, _on: bool) -> Result<(), Self::Error> {
        Ok(())
    }
}

//! USB Type-C configuration channel (CC) library traits.
//!
//! Provides the hardware traits through which the `usbcc` state machine talks to a port.
//! Each trait covers one part of the port and can be implemented independently:
//!
//! - [`CcPhy`]: the CC front end (terminations and CC line states).
//! - [`VbusSense`]: VBUS voltage measurement.
//! - [`PowerPath`]: the VBUS power path and the VCONN supply.
//!
//! Section references are to the USB Type-C Cable and Connector Specification, Release 2.5.
#![cfg_attr(not(test), no_std)]
#![warn(missing_docs)]

use core::future::{self, Future};

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
///
/// Non-exhaustive, as more terminations may be added, e.g. the pull-down of the
/// CorrosionMitigation state (Appendix A.2). Implementations return an error for terminations
/// they don't support.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub enum Termination {
    /// No termination (high impedance).
    Open,
    /// Pull-down, presented by a Sink.
    Rd,
    /// Pull-up with the given current advertisement, presented by a Source.
    Rp(RpLevel),
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
    ///
    /// A [`CcState`] is only meaningful for the termination it was detected with. After this call,
    /// [`CcPhy::cc_state`] and [`CcPhy::wait_for_cc_change`] report states for the new
    /// terminations, once the CC lines have settled.
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

    /// Waits until the VBUS voltage is below `mv`.
    ///
    /// Returns immediately if it already is.
    fn wait_vbus_below(&mut self, mv: u32) -> impl Future<Output = Result<(), Self::Error>> {
        self.wait_vbus_outside(mv, u32::MAX)
    }

    /// Waits until the VBUS voltage is above `mv`.
    ///
    /// Returns immediately if it already is.
    fn wait_vbus_above(&mut self, mv: u32) -> impl Future<Output = Result<(), Self::Error>> {
        self.wait_vbus_outside(0, mv)
    }
}

/// The power path of a port: the VBUS sink and source paths, and the VCONN supply.
///
/// `usbcc` never calls two methods of this trait at the same time.
pub trait PowerPath {
    /// Error type of the implementation.
    type Error;

    /// Tells the power path whether the port is active.
    ///
    /// A port is active from when it leaves Unattached.SNK or Unattached.SRC because it detected
    /// a connection, until it is back in one of them. That includes the states around an attach
    /// in which the power path is already or still needed: AttachWait.SRC discharges VBUS
    /// (Section 4.5.2.2.8) and UnattachedWait.SRC discharges VCONN (Section 4.5.2.2.6).
    ///
    /// Called with `true` before any other method of this trait is called, and with `false` after
    /// VBUS and VCONN are off and discharged. While the port is not active, only the CC detection
    /// has to work, so hardware may enter a low-power detection mode (e.g. LowPower on the
    /// TCPP03-M20). The default implementation does nothing.
    fn set_active(&mut self, active: bool) -> impl Future<Output = Result<(), Self::Error>> {
        let _ = active;
        future::ready(Ok(()))
    }

    /// Connects (`true`) or disconnects (`false`) the sink path, through which the port consumes
    /// VBUS.
    ///
    /// `usbcc` never connects the sink path of a port configured as Source-only. Implementations
    /// for hardware without a sink path may do nothing for `false` and panic for `true`, which
    /// indicates a misconfiguration.
    fn set_vbus_sink(&mut self, on: bool) -> impl Future<Output = Result<(), Self::Error>>;

    /// Prepares the supply behind the source path (VBULK) and returns once it is ready to supply
    /// VBUS at vSafe5V, e.g. once a converter has started up or has come back down to vSafe5V
    /// from a higher voltage. Doesn't connect the source path.
    ///
    /// Called in AttachWait.SRC, which only transitions to Attached.SRC once VBULK is ready
    /// (Section 4.5.2.2.8). `usbcc` waits for the CC lines at the same time and drops the returned
    /// future if the attach is abandoned. The default implementation returns immediately, for
    /// supplies that are always ready.
    fn prepare_vbus_source(&mut self) -> impl Future<Output = Result<(), Self::Error>> {
        future::ready(Ok(()))
    }

    /// Connects (`true`) or disconnects (`false`) the source path, through which the port supplies
    /// VBUS.
    ///
    /// Called with `true` on entering Attached.SRC, after [`PowerPath::prepare_vbus_source`] has
    /// returned; VBUS has to be at vSafe5V within tVBUSON (Section 4.5.2.2.9). Called with `false`
    /// to stop supplying VBUS, and also when an attach is abandoned after
    /// [`PowerPath::prepare_vbus_source`], in which case the supply can be shut down again.
    ///
    /// `usbcc` never connects the source path of a port configured as Sink-only. Implementations
    /// for hardware without a source path may do nothing for `false` and panic for `true`, which
    /// indicates a misconfiguration.
    fn set_vbus_source(&mut self, on: bool) -> impl Future<Output = Result<(), Self::Error>>;

    /// Enables (`true`) or disables (`false`) the VBUS discharge load.
    ///
    /// A Source discharges VBUS to vSafe0V after turning it off, e.g. in AttachWait.SRC
    /// (Section 4.5.2.2.8). `usbcc` never enables it on a port configured as Sink-only;
    /// implementations for hardware without a discharge load may do nothing.
    fn set_vbus_discharge(&mut self, on: bool) -> impl Future<Output = Result<(), Self::Error>>;

    /// Supplies VCONN on the given CC pin, or on none.
    ///
    /// VCONN is optional for ports without USB TX/RX signaling that supply at most 3 A
    /// (Table 4-4); whether a port sources VCONN is part of the `usbcc` configuration. The default
    /// implementation is for hardware that can't source VCONN: it does nothing for `None`.
    ///
    /// # Panics
    ///
    /// The default implementation panics for `Some`, which means that the port is configured to
    /// source VCONN but its power path can't.
    fn set_vconn(&mut self, pin: Option<CcPin>) -> impl Future<Output = Result<(), Self::Error>> {
        assert!(
            pin.is_none(),
            "VCONN requested from a `PowerPath` without VCONN: the port is configured to source VCONN but its power path can't"
        );
        future::ready(Ok(()))
    }

    /// Applies the VCONN discharge resistance (Rdch, Table 4-5) to the given CC pin, or to none.
    ///
    /// In UnattachedWait.SRC, the CC pin that supplied VCONN is discharged (Section 4.5.2.2.6).
    /// `usbcc` presents [`Termination::Open`] on that pin through the [`CcPhy`] meanwhile. The
    /// default implementation does nothing, for hardware that can't source VCONN.
    fn set_vconn_discharge(&mut self, pin: Option<CcPin>) -> impl Future<Output = Result<(), Self::Error>> {
        let _ = pin;
        future::ready(Ok(()))
    }

    /// Waits until VCONN is below vVCONNDischarge.
    ///
    /// UnattachedWait.SRC only transitions to Unattached.SRC once it is (Section 4.5.2.2.6).
    /// Implementations without VCONN measurement may wait for a time that guarantees it, e.g. the
    /// maximum discharge time of the hardware. The default implementation returns immediately, for
    /// hardware that can't source VCONN.
    fn wait_vconn_discharged(&mut self) -> impl Future<Output = Result<(), Self::Error>> {
        future::ready(Ok(()))
    }
}

impl<T: CcPhy + ?Sized> CcPhy for &mut T {
    type Error = T::Error;

    fn set_termination(&mut self, cc1: Termination, cc2: Termination) -> impl Future<Output = Result<(), Self::Error>> {
        T::set_termination(self, cc1, cc2)
    }

    fn cc_state(&mut self) -> impl Future<Output = Result<(CcState, CcState), Self::Error>> {
        T::cc_state(self)
    }

    fn wait_for_cc_change(&mut self) -> impl Future<Output = Result<(CcState, CcState), Self::Error>> {
        T::wait_for_cc_change(self)
    }
}

impl<T: VbusSense + ?Sized> VbusSense for &mut T {
    type Error = T::Error;

    fn vbus_mv(&mut self) -> impl Future<Output = Result<u32, Self::Error>> {
        T::vbus_mv(self)
    }

    fn wait_vbus_outside(&mut self, low_mv: u32, high_mv: u32) -> impl Future<Output = Result<(), Self::Error>> {
        T::wait_vbus_outside(self, low_mv, high_mv)
    }

    fn wait_vbus_below(&mut self, mv: u32) -> impl Future<Output = Result<(), Self::Error>> {
        T::wait_vbus_below(self, mv)
    }

    fn wait_vbus_above(&mut self, mv: u32) -> impl Future<Output = Result<(), Self::Error>> {
        T::wait_vbus_above(self, mv)
    }
}

impl<T: PowerPath + ?Sized> PowerPath for &mut T {
    type Error = T::Error;

    fn set_active(&mut self, active: bool) -> impl Future<Output = Result<(), Self::Error>> {
        T::set_active(self, active)
    }

    fn set_vbus_sink(&mut self, on: bool) -> impl Future<Output = Result<(), Self::Error>> {
        T::set_vbus_sink(self, on)
    }

    fn prepare_vbus_source(&mut self) -> impl Future<Output = Result<(), Self::Error>> {
        T::prepare_vbus_source(self)
    }

    fn set_vbus_source(&mut self, on: bool) -> impl Future<Output = Result<(), Self::Error>> {
        T::set_vbus_source(self, on)
    }

    fn set_vbus_discharge(&mut self, on: bool) -> impl Future<Output = Result<(), Self::Error>> {
        T::set_vbus_discharge(self, on)
    }

    fn set_vconn(&mut self, pin: Option<CcPin>) -> impl Future<Output = Result<(), Self::Error>> {
        T::set_vconn(self, pin)
    }

    fn set_vconn_discharge(&mut self, pin: Option<CcPin>) -> impl Future<Output = Result<(), Self::Error>> {
        T::set_vconn_discharge(self, pin)
    }

    fn wait_vconn_discharged(&mut self) -> impl Future<Output = Result<(), Self::Error>> {
        T::wait_vconn_discharged(self)
    }
}

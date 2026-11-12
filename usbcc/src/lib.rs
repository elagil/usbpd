#![doc = include_str!("../README.md")]
#![cfg_attr(not(test), no_std)]
#![warn(missing_docs)]

// This mod MUST go first, so that the others see its macros.
pub(crate) mod fmt;

use core::fmt::Debug;

use core::future::Future;
use core::marker::PhantomData;

use usbcc_traits::{CcPhy, PowerPath, VbusSense};
pub use usbcc_traits::{CcPin, RpLevel};

/// The timer trait to implement by the user application.
pub trait Timer {
    /// Expire after the specified number of milliseconds.
    fn after_millis(milliseconds: u64) -> impl Future<Output = ()>;
}

/// The power role of a port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum PowerRole {
    /// The port supplies VBUS.
    Source,
    /// The port consumes VBUS.
    Sink,
}

/// The role that a DRP prefers, per Type-C 2.5 Sec. 4.5.1.4.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum TryRole {
    /// Try.SRC: prefer the Source role.
    Source,
    /// Try.SNK: prefer the Sink role.
    Sink,
}

/// The type of a port, per Type-C 2.5 Tab. 4-15.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum PortType {
    /// Sink-only port.
    Sink,
    /// Source-only port.
    Source,
    /// Dual-role port, optionally with a preferred role.
    Drp {
        /// The preferred role, if any.
        try_role: Option<TryRole>,
    },
}

/// Port configuration: everything that Type-C 2.5 leaves to the implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Config {
    /// The type of the port.
    pub port_type: PortType,
    /// The USB Type-C Current that the port advertises as a Source.
    pub source_current: RpLevel,
    /// Whether the port supplies VCONN as a Source when it detects Ra.
    ///
    /// Requires a power path that can supply VCONN.
    pub vconn_source: bool,
    /// VBUS threshold in mV below which a Sink detects a detach (vSinkDisconnect).
    ///
    /// Must be within 800 mV to 3670 mV, per Type-C 2.5 Tab. 4-3.
    pub vsink_disconnect_mv: u32,
}

impl Default for Config {
    /// A Sink-only port.
    fn default() -> Self {
        Self {
            port_type: PortType::Sink,
            source_current: RpLevel::Default,
            vconn_source: false,
            vsink_disconnect_mv: 3500,
        }
    }
}

/// Connection states, per Type-C 2.5 Sec. 4.5.2.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub enum State {
    /// Disabled, per Sec. 4.5.2.2.1.
    Disabled,
    /// ErrorRecovery, per Sec. 4.5.2.2.2.
    ErrorRecovery,
    /// Unattached.SNK, per Sec. 4.5.2.2.3.
    UnattachedSnk,
    /// AttachWait.SNK, per Sec. 4.5.2.2.4.
    AttachWaitSnk,
    /// Attached.SNK, per Sec. 4.5.2.2.5.
    AttachedSnk,
    /// UnattachedWait.SRC, per Sec. 4.5.2.2.6.
    UnattachedWaitSrc,
    /// Unattached.SRC, per Sec. 4.5.2.2.7.
    UnattachedSrc,
    /// AttachWait.SRC, per Sec. 4.5.2.2.8.
    AttachWaitSrc,
    /// Attached.SRC, per Sec. 4.5.2.2.9.
    AttachedSrc,
    /// Try.SRC, per Sec. 4.5.2.2.10.
    TrySrc,
    /// TryWait.SNK, per Sec. 4.5.2.2.11.
    TryWaitSnk,
    /// Try.SNK, per Sec. 4.5.2.2.12.
    TrySnk,
    /// TryWait.SRC, per Sec. 4.5.2.2.13.
    TryWaitSrc,
}

/// An established connection.
///
/// The data role follows from the power role: a port that attaches as a Source is the DFP (USB
/// host), a port that attaches as a Sink is the UFP (USB device), per Type-C 2.5 Sec. 4.5.2.2.9
/// and Sec. 4.5.2.2.5. Only USB PD can change that afterwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Attachment {
    /// The power role of this port.
    pub power_role: PowerRole,
    /// The CC pin that is connected through the cable, i.e. the plug orientation.
    pub cc: CcPin,
    /// The USB Type-C Current that the Source advertises. `None` if this port is the Source.
    pub type_c_current: Option<RpLevel>,
}

/// Events that the device policy manager can send to the state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub enum Event {
    /// Direct the port to ErrorRecovery, per Type-C 2.5 Sec. 4.5.2.2.2.
    ErrorRecovery,
    /// Direct the port to Disabled, per Type-C 2.5 Sec. 4.5.2.2.1.
    Disable,
    /// Direct the port out of Disabled.
    Enable,
}

/// Information that the state machine passes to the device policy manager.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub enum Info {
    /// As a Sink: the Source changed the USB Type-C Current that it advertises.
    ///
    /// The device has to adapt its consumption within tSinkAdj, per Type-C 2.5 Sec. 4.5.2.3.
    TypeCCurrent(RpLevel),
}

/// Trait for the device policy manager.
///
/// This entity commands the state machine and enforces device policy.
pub trait DevicePolicyManager {
    /// Inform the device about a change of the connection.
    fn inform(&mut self, _info: Info) -> impl Future<Output = ()> {
        async {}
    }

    /// The state machine awaits an event from the device.
    ///
    /// By default, this is a future that never resolves.
    ///
    /// <div class="warning">
    /// The function must be safe to cancel.
    /// </div>
    fn get_event(&mut self) -> impl Future<Output = Event> {
        async { core::future::pending().await }
    }
}

/// Errors of a [`Port`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Error<C, V, P> {
    /// Error of the [`CcPhy`].
    CcPhy(C),
    /// Error of the [`VbusSense`].
    VbusSense(V),
    /// Error of the [`PowerPath`].
    PowerPath(P),
}

/// Result of a [`Port`] operation.
pub type PortResult<T, CC, VBUS, POWER> =
    Result<T, Error<<CC as CcPhy>::Error, <VBUS as VbusSense>::Error, <POWER as PowerPath>::Error>>;

/// A USB Type-C port.
pub struct Port<CC: CcPhy, VBUS: VbusSense, POWER: PowerPath, TIMER: Timer> {
    cc: CC,
    vbus: VBUS,
    power: POWER,
    config: Config,
    state: State,
    _timer: PhantomData<TIMER>,
}

impl<CC: CcPhy, VBUS: VbusSense, POWER: PowerPath, TIMER: Timer> Port<CC, VBUS, POWER, TIMER> {
    /// Create a new port.
    pub fn new(cc: CC, vbus: VBUS, power: POWER, config: Config) -> Self {
        let _ = (cc, vbus, power, config);
        todo!()
    }

    /// The current connection state.
    pub fn state(&self) -> State {
        self.state
    }

    /// Run the state machine until the port is attached.
    ///
    /// Returns in Attached.SNK or Attached.SRC.
    pub async fn wait_attached<DPM: DevicePolicyManager>(
        &mut self,
        device_policy_manager: &mut DPM,
    ) -> PortResult<Attachment, CC, VBUS, POWER> {
        let _ = device_policy_manager;
        todo!()
    }

    /// Run the state machine while the port is attached, until it is detached.
    ///
    /// Returns when the port has left Attached.SNK or Attached.SRC.
    pub async fn wait_detached<DPM: DevicePolicyManager>(
        &mut self,
        device_policy_manager: &mut DPM,
    ) -> PortResult<(), CC, VBUS, POWER> {
        let _ = device_policy_manager;
        todo!()
    }
}

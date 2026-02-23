//! The device policy manager (DPM) allows a device to control the policy engine, and be informed about status changes.
//!
//! For example, through the DPM, a device can request certain source capabilities (voltage, current),
//! or renegotiate the power contract.
use core::future::Future;

use crate::DataRole;
use crate::protocol_layer::message::data::request;
use crate::protocol_layer::message::data::sink_capabilities::SinkCapabilities;
use crate::protocol_layer::message::data::source_capabilities::SourceCapabilities;

/// Events that the device policy manager can send to the policy engine.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Event {
    /// Empty event.
    None,
    /// Source capabilities have changed
    UpdatedSourceCapabilities,
    /// Get Sink capabilities
    RequestSinkCapabilities,
    /// Get the Remote DRP's Source capabilities
    RequestSourceCapabilities,
    /// Indicate a Vconn swap needs to be done
    RequestVconnSwap,
    /// **EPR** Get the Remote DRP's Source EPR capabilities
    RequestEprSourceCapabilities,
    /// **EPR**
    ExitEprMode,
    /// **DRP** Indicate a data swap needs to be done
    RequestDataRoleSwap,
    /// **DRP** Indicate a power role swap needs to be done
    RequestPowerRoleSwap,
}

#[derive(Debug)]
/// Information that the policy engine will publish to the DPM
pub enum Info {
    /// SPR Sink Capabilities
    SprSinkCapabilities(Option<SinkCapabilities>),
    /// EPR Sink Capabilities
    EprSinkCapabilities(Option<SinkCapabilities>),
    /// Remote Source Capabilities
    RemoteSourceCapabilities(Option<SourceCapabilities>),
}

#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
/// How the source DPM can respond to a request from the sink
pub enum CapabilityResponse {
    /// Request is rejected
    Reject,
    /// Request could be met later from the power reserve & present contract is still valid
    Wait,
    /// Requast can be met now
    Accept,
}

#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
/// For defining DPM swap behavior
pub enum SwapType {
    /// **DRP** Data Role Swap (UFP <---> DFP)
    Data,
    /// **DRP** Power Role Swap (Source --> Sink)
    Power,
}

/// Trait for the device policy manager.
/// Functions labeled **EPR** do not need to be implemented on non-epr sources.
/// Functions labeled **DRP** do not need to be implemented on source-only devices.
///
/// This entity commands the policy engine and enforces device policy.
pub trait DevicePolicyManager {
    /// Respond to the Policy Engine's request for this port's current source capabilities
    ///
    /// Defaults to only default usb capability (5v @ 3A)
    fn source_capabilities(&mut self) -> SourceCapabilities {
        SourceCapabilities::new_vsafe5v_only(3 * 100)
    }

    /// Evaluate a request from the Sink
    ///
    /// The policy engine will use this evaluation to determine PD control flow
    fn evaluate_request(&mut self, _request: &request::PowerSource) -> impl Future<Output = CapabilityResponse> {
        async { CapabilityResponse::Reject }
    }

    /// Transition source power to a new power level.
    ///
    /// After, the DPM will notify the Sink on the outcome of the transition
    fn transition_power(&mut self, _power_level: &request::PowerSource) -> impl Future<Output = Result<(), ()>> {
        async { Ok(()) }
    }

    /// Hard reset power supply to vSafe5V via vSafe0V
    ///
    /// A Hard Reset shall not cause any change to either the Rp/Rd resistor being asserted
    fn hard_reset(&mut self) -> impl Future<Output = Result<(), ()>> {
        async { Ok(()) }
    }

    /// Evaluate whether a vconn swap can be done by the device or not.
    fn evaluate_vconn_swap_request(&mut self) -> impl Future<Output = bool> {
        async { false }
    }

    /// Set port to drive VCONN to 5V (true) or not (false)
    fn drive_vconn(&mut self, _on: bool) -> impl Future<Output = Result<(), ()>> {
        async { Ok(()) }
    }

    /// The policy engine gets and evaluates device policy events when ready.
    ///
    /// By default, this is a future that never resolves.
    ///
    /// <div class="warning">
    /// The function must be safe to cancel. To determine whether your own methods are cancellation safe,
    /// look for the location of uses of .await. This is because when an asynchronous method is cancelled,
    /// that always happens at an .await. If your function behaves correctly even if it is restarted while waiting
    /// at an .await, then it is cancellation safe.
    /// </div>
    fn get_event(&mut self) -> impl Future<Output = Event> {
        async { core::future::pending().await }
    }

    /// The policy engine publishes events for the device policy manager to handle here.
    ///
    /// By default, this is a future that returns immediately
    fn inform(&mut self, _info: Info) -> impl Future<Output = ()> {
        async {}
    }
}

/// **EPR** Extended Power Range mode device implementation
///
/// Leave unimplemented if EPR is not supported by the device.
pub trait EprDevicePolicyManager {
    /// **EPR** Return `true` if device is EPR capable.
    ///
    /// Also possible to dynamically assess EPR capability
    fn epr_capable(&mut self) -> bool {
        false
    }

    /// **EPR** Return `true` if the `Cable Plug` supports EPR
    fn epr_cable_good(&mut self) -> bool {
        false
    }

    /// **EPR** Return device's EPR capabilities
    fn epr_source_capabilities(&mut self) -> SourceCapabilities {
        SourceCapabilities::new_vsafe5v_only(50)
    }
}

/// **DRP** Dual Role Port device implementations
///
/// Leave unimplemented if the device is not a DRP.
pub trait DualRoleDevicePolicyManager {
    /// **DRP** Respond to the Policy Engine's request for this port's current sink capabilities
    ///
    /// Defaults to only default usb capability (5v @ 500 mA)
    fn sink_capabilities(&mut self) -> impl Future<Output = SinkCapabilities> {
        async { SinkCapabilities::new_vsafe5v_only(3 * 100) }
    }

    /// Evaluate a swap request:
    /// - **DRP**: Data, Power, Fast Power
    fn evaluate_swap_request(&mut self, _swap_request: SwapType) -> impl Future<Output = bool> {
        async { false }
    }

    /// **DRP** Detect whether a fast role swap is signaled on the cc lines
    ///
    /// Table 1.4 - Fast Role Swap Request:
    ///
    /// An indication from an Initial Source to the Initial Sink that a
    /// Fast Role Swap is needed. The Fast Role Swap Request is indicated by
    /// driving the CC line to ground for a short period.
    fn fr_swap_signaled(&mut self) -> impl Future<Output = bool> {
        async { false }
    }

    /// **DRP** Discharge the VBUS to vSafe5V
    fn discharge_vbus(&mut self) -> impl Future<Output = ()> {
        async {}
    }

    /// **DRP** Turn the Source off.
    ///
    /// This will be requested before a Role Swap to Sink
    fn disable_source(&mut self) -> impl Future<Output = ()> {
        async {}
    }

    /// **DRP** Set the CC lines to sink configuration
    ///
    /// This will be requested before a Role Swap to Sink
    fn cc_sink(&mut self) -> impl Future<Output = ()> {
        async {}
    }

    /// **DRP** Swap data role
    fn swap_data_role(&mut self, _role: DataRole) -> impl Future<Output = ()> {
        async {}
    }
}

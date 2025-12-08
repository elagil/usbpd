//! The device policy manager (DPM) allows a device to control the policy engine, and be informed about status changes.
//!
//! For example, through the DPM, a device can request certain source capabilities (voltage, current),
//! or renegotiate the power contract.
use core::future::Future;

use crate::protocol_layer::message::data::{request, source_capabilities};

/// Events that the device policy manager can send to the policy engine.
#[derive(Debug)]
pub enum Event {
    /// Empty event.
    None,
    /// Request SPR source capabilities.
    RequestSprSourceCapabilities,
    /// Request EPR source capabilities (when already in EPR mode).
    ///
    /// Sends EprGetSourceCap extended control message.
    /// See [8.3.3.8.1]
    RequestEprSourceCapabilities,
    /// Enter EPR mode.
    ///
    /// Initiates EPR mode entry sequence (EPR_Mode Enter -> EnterAcknowledged -> EnterSucceeded).
    /// After successful entry, source automatically sends EPR_Source_Capabilities.
    /// See spec Table 8.39: "Steps for Entering EPR Mode (Success)"
    EnterEprMode,
    /// Exit EPR mode (sink-initiated).
    ///
    /// Sends EPR_Mode (Exit) message to source, then waits for Source_Capabilities.
    /// After receiving caps, negotiation proceeds as normal SPR negotiation.
    /// See spec Table 8.46: "Steps for Exiting EPR Mode (Sink Initiated)"
    ExitEprMode,
    /// Request a certain power level.
    RequestPower(request::PowerSource),
}

/// Trait for the device policy manager.
///
/// This entity commands the policy engine and enforces device policy.
pub trait DevicePolicyManager {
    /// Inform the device about source capabilities, e.g. after a request.
    fn inform(&mut self, _source_capabilities: &source_capabilities::SourceCapabilities) -> impl Future<Output = ()> {
        async {}
    }

    /// Request a power source.
    ///
    /// Defaults to 5 V at maximum current.
    fn request(
        &mut self,
        source_capabilities: &source_capabilities::SourceCapabilities,
    ) -> impl Future<Output = request::PowerSource> {
        async {
            request::PowerSource::new_fixed(
                request::CurrentRequest::Highest,
                request::VoltageRequest::Safe5V,
                source_capabilities,
            )
            .unwrap()
        }
    }

    /// Notify the device that it shall transition to a new power level.
    ///
    /// The device is informed about the request that was accepted by the source.
    fn transition_power(&mut self, _accepted: &request::PowerSource) -> impl Future<Output = ()> {
        async {}
    }

    /// Notify the device that a hard reset has occurred.
    ///
    /// Per USB PD Spec R3.2 Section 8.3.3.3.9, on entry to PE_SNK_Transition_to_default:
    /// - The sink shall transition to default power level (vSafe5V)
    /// - Local hardware should be reset
    /// - Port data role should be set to UFP
    ///
    /// The device should prepare for VBUS going to vSafe0V and then back to vSafe5V.
    /// This callback should return when the device has reached the default level.
    fn hard_reset(&mut self) -> impl Future<Output = ()> {
        async {}
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
    fn get_event(
        &mut self,
        _source_capabilities: &source_capabilities::SourceCapabilities,
    ) -> impl Future<Output = Event> {
        async { core::future::pending().await }
    }
}

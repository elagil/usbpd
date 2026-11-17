//! Device policy manager hooks shared between the sink and source roles.
//!
//! The role-specific trait bundles live in [`crate::sink::device_policy_manager`]
//! and [`crate::source::device_policy_manager`]. The hooks here mean the same
//! thing on both sides of the protocol, so their defaults are defined once.

use core::future::Future;

use crate::{DataRole, SwapType};

/// Hooks used while executing a Vconn Swap, per PD 3.2 Sec. 8.3.3.20.
pub trait VconnSwapHooks {
    /// Evaluate whether a vconn swap can be done by the device or not.
    ///
    /// Note: Wait responses are not supported. The PE answers `Accept` (true) or `Reject` (false).
    fn evaluate_vconn_swap_request(&mut self) -> impl Future<Output = bool> {
        async { false }
    }

    /// Set port to drive VCONN to 5 V (true) or not (false).
    fn drive_vconn(&mut self, _on: bool) -> impl Future<Output = Result<(), ()>> {
        async { Ok(()) }
    }
}

/// Hooks shared by the Data Role Swap executor, per PD 3.2 Sec. 8.3.3.19.
pub trait DrpSwapHooks {
    /// DRP Evaluate a swap request.
    ///
    /// Note: Wait responses are not supported. The PE answers `Accept` (true) or `Reject` (false).
    fn evaluate_swap_request(&mut self, _swap_request: SwapType) -> impl Future<Output = bool> {
        async { false }
    }

    /// DRP Turn the port's current power role off.
    ///
    /// Requested by the policy engine before the CC configuration of the other
    /// role is asserted.
    fn disable(&mut self) -> impl Future<Output = ()> {
        async {}
    }

    /// DRP Swap data role.
    fn swap_data_role(&mut self, _role: DataRole) -> impl Future<Output = ()> {
        async {}
    }
}

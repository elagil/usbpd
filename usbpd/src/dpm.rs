//! Device policy manager functionality that is shared between the sink and source roles.
//!
//! The role-specific traits live in [`crate::sink::device_policy_manager`]
//! and [`crate::source::device_policy_manager`].

use core::future::Future;

use crate::{DataRole, SwapType};

/// Functions used while executing a Vconn Swap, per PD 3.2 Sec. 8.3.3.20.
pub trait VconnSwap {
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

/// Functions shared by the Data Role Swap executor, per PD 3.2 Sec. 8.3.3.19.
pub trait DrpSwap {
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

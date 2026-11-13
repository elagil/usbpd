//! Shared dual-role-port (DRP) state types.
//!
//! Used by both the sink and source policy engines, which implement the two
//! sides of the same PD 3.2 Sec. 8.3.3.19 and 8.3.3.20 state machines.
//!
//! Power Role Swap and Fast Role Swap states are deliberately not shared:
//! per PD 3.2, each role runs its own state machine family with different states on each side:
//! - sink: PE_PRS_SNK_SRC, PE_FRS_SNK_SRC
//! - source: PE_PRS_SRC_SNK, PE_FRS_SRC_SNK
//!
//! The policy engine [`crate::Error`] enum moves with the state machines and lives
//! at the crate root.

/// State during a Data Role Swap execution (for both directions),
/// per PD 3.2 Sec. 8.3.3.19.1 and 8.3.3.19.2.
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) enum DataRoleSwap {
    Evaluate,
    Accept,
    Change,
    Send,
    Reject,
}

/// PD 3.2 Sec. 8.3.3.20 Vconn Swap. State during a Vconn Swap execution
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) enum VcsState {
    SendSwap,
    EvaluateSwap,
    AcceptSwap,
    RejectSwap,
    WaitForVconn,
    TurnOffVconn,
    TurnOnVconn,
    SendPsRdy,
    // FIXME: For now, forcing a different state traversal, resulting in this being unused
    #[allow(unused)]
    ForceVconn,
}

use embassy_stm32::ucpd::{self, CcPhy, CcVState};
use embassy_time::Duration;

#[derive(Debug, Clone, Copy, defmt::Format)]
enum CableOrientation {
    Normal,
    Flipped,
    DebugAccessoryMode,
}

/// `tCCDebounce` Wait time for CC lines to be stable after an attach
const CC_DEBOUNCE_DUR: Duration = Duration::from_millis(200);

pub(crate) async fn run_while_attached<T: ucpd::Instance>(cc_phy: &mut CcPhy<'_, T>) {
    // TODO: Also implement CC_PHY alteration requests from the DPM!
    loop {
        let (cc1, cc2) = cc_phy.vstate();
        if cc1 == CcVState::LOWEST && cc2 == CcVState::LOWEST {
            return;
        }
        cc_phy.wait_for_vstate_change().await;
    }
}

// Returns true when the cable was attached.
#[cfg(feature = "attach_sink")]
pub(crate) async fn wait_attached<T: ucpd::Instance>(
    cc_phy: &mut CcPhy<'_, T>,
) -> Result<(ucpd::CcSel, usbpd::PowerRole), ()> {
    loop {
        let (cc1, cc2) = cc_phy.vstate();
        if cc1 == CcVState::LOWEST && cc2 == CcVState::LOWEST {
            // Detached, wait until attached by monitoring the CC lines.
            cc_phy.wait_for_vstate_change().await;
            continue;
        }

        // Attached, wait for CC lines to be stable for tCCDebounce (100..200ms).
        if embassy_time::with_timeout(Duration::from_millis(100), cc_phy.wait_for_vstate_change())
            .await
            .is_ok()
        {
            // State has changed, restart detection procedure.
            continue;
        };

        // State was stable for the complete debounce period, check orientation.
        return match (cc1, cc2) {
            (_, CcVState::LOWEST) => Ok((ucpd::CcSel::CC1, usbpd::PowerRole::Sink)), // CC1 connected
            (CcVState::LOWEST, _) => Ok((ucpd::CcSel::CC2, usbpd::PowerRole::Sink)), // CC2 connected
            _ => Err(()),                                                            // Both connected (special cable)
        };
    }
}

#[cfg(feature = "attach_source")]
pub(crate) async fn wait_attached<T: ucpd::Instance>(
    cc_phy: &mut CcPhy<'_, T>,
) -> Result<(ucpd::CcSel, usbpd::PowerRole), ()> {
    todo!("Implement source only attaching!");
}

#[cfg(feature = "attach_alternate")]
pub(crate) async fn wait_attached<T: ucpd::Instance>(
    cc_phy: &mut CcPhy<'_, T>,
) -> Result<(ucpd::CcSel, usbpd::PowerRole), ()> {
    todo!("Implement alternating source-sink CC configuration attaching!");
}

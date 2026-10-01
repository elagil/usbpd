//! Handles USB PD negotiation as a dual role port.

use defmt::{Format, info, warn};
#[allow(dead_code)]
use defmt_rtt as _;
use embassy_stm32::ucpd::{self, CcPhy, CcPull, CcSel, CcVState, PdPhy, Ucpd};
use embassy_stm32::{Peri, bind_interrupts, dma, peripherals};
use embassy_time::{Duration, Timer};
use usbpd::PowerRole;
use usbpd::dual_role::DualRolePort;
use usbpd::protocol_layer::message::data::request::PowerSource;
use usbpd::protocol_layer::message::data::source_capabilities::SourceCapabilities;
use usbpd::sink::device_policy_manager::{
    DevicePolicyManager as SinkDevicePolicyManager, DrpDevicePolicyManager as SinkDrpDpm,
    EprDevicePolicyManager as SinkEprDpm, Event as SinkEvent, SinkDpm,
};
use usbpd::source::device_policy_manager::{
    CapabilityResponse, DevicePolicyManager as SourceDevicePolicyManager, DrpDevicePolicyManager as SourceDrpDpm,
    EprDevicePolicyManager as SourceEprDpm, SourceDpm,
};
use usbpd::timers::Timer as UsbpdTimer;
use usbpd_tcpp03_m20::{PdRole, Tcpp, TcppResources};
use usbpd_traits::Driver;

bind_interrupts!(struct Irqs {
    UCPD1 => ucpd::InterruptHandler<peripherals::UCPD1>;
    DMA1_CHANNEL1 => dma::InterruptHandler<peripherals::DMA1_CH1>;
    DMA1_CHANNEL2 => dma::InterruptHandler<peripherals::DMA1_CH2>;
});

pub struct UcpdResources {
    pub ucpd: Peri<'static, peripherals::UCPD1>,
    pub pin_cc1: Peri<'static, peripherals::PB6>,
    pub pin_cc2: Peri<'static, peripherals::PB4>,
    pub rx_dma: Peri<'static, peripherals::DMA1_CH1>,
    pub tx_dma: Peri<'static, peripherals::DMA1_CH2>,
    pub tcpp: TcppResources,
}

#[derive(Debug, Format)]
enum CableOrientation {
    Normal,
    Flipped,
    DebugAccessoryMode,
}

/// Unified PD PHY driver, valid for both the sink and source policy engines.
///
/// Owns the TCPP03 reference for VBUS observation via the shield's ADC.
struct UcpdDrpDriver<'d> {
    pd_phy: PdPhy<'d, peripherals::UCPD1>,
    tcpp: &'d mut Tcpp,
}

impl<'d> UcpdDrpDriver<'d> {
    fn new(pd_phy: PdPhy<'d, peripherals::UCPD1>, tcpp: &'d mut Tcpp) -> Self {
        Self { pd_phy, tcpp }
    }
}

/// Time to wait for VBUS before proceeding, and the interval between ADC
/// samples. Sufficient for a partner in Normal mode, which applies VBUS
/// almost instantly.
const VBUS_WAIT_TIMEOUT: Duration = Duration::from_secs(1);
const VBUS_POLL_INTERVAL: Duration = Duration::from_millis(10);

impl Driver for UcpdDrpDriver<'_> {
    async fn wait_for_vbus(&mut self) {
        // VBUS observation is mode-independent only via the ADC; the device's
        // VBUS_OK flag does not assert in Normal mode.
        let deadline = embassy_time::Instant::now() + VBUS_WAIT_TIMEOUT;
        loop {
            match self.tcpp.vbus_mv().await {
                Ok(mv) if u32::from(mv) >= usbpd_tcpp03_m20::VBUS_PRESENT_MV => {
                    info!("TCPP03: vbus present ({} mV)", mv);
                    return;
                }
                Ok(mv) => warn!("TCPP03: waiting for vbus, seeing {} mV", mv),
                Err(err) => warn!("TCPP03: vbus read failed: {}", err),
            }
            if embassy_time::Instant::now() >= deadline {
                warn!("TCPP03: vbus not observed within {:?}, continuing", VBUS_WAIT_TIMEOUT);
                return;
            }
            Timer::after(VBUS_POLL_INTERVAL).await;
        }
    }

    async fn receive(&mut self, buffer: &mut [u8]) -> Result<usize, usbpd_traits::DriverRxError> {
        self.pd_phy.receive(buffer).await.map_err(|err| match err {
            ucpd::RxError::Crc | ucpd::RxError::Overrun => usbpd_traits::DriverRxError::Discarded,
            ucpd::RxError::HardReset => usbpd_traits::DriverRxError::HardReset,
        })
    }

    async fn transmit(&mut self, data: &[u8]) -> Result<(), usbpd_traits::DriverTxError> {
        self.pd_phy.transmit(data).await.map_err(|err| match err {
            ucpd::TxError::Discarded => usbpd_traits::DriverTxError::Discarded,
            ucpd::TxError::HardReset => usbpd_traits::DriverTxError::HardReset,
        })
    }

    async fn transmit_hard_reset(&mut self) -> Result<(), usbpd_traits::DriverTxError> {
        self.pd_phy.transmit_hardreset().await.map_err(|err| match err {
            ucpd::TxError::Discarded => usbpd_traits::DriverTxError::Discarded,
            ucpd::TxError::HardReset => usbpd_traits::DriverTxError::HardReset,
        })
    }
}

struct EmbassyTimer {}

impl UsbpdTimer for EmbassyTimer {
    async fn after_millis(milliseconds: u64) {
        Timer::after_millis(milliseconds).await
    }
}

/// Device policy manager for both the sink and source policies.
///
/// Owns the CC phy (Rp/Rd swaps) and the TCPP03 reference (role gates).
struct Device<'d> {
    cc_phy: Option<CcPhy<'d, peripherals::UCPD1>>,
    tcpp: &'d mut Tcpp,
    /// Initiator only: whether a PR_Swap still needs to be requested.
    #[cfg_attr(not(feature = "role-initiator"), allow(dead_code))]
    request_swap: bool,
    /// First contract observed in any role.
    contract_seen: bool,
    /// The DPM's CC pull hooks are invoked by the library only while executing
    /// a power role swap, so this reliably marks a swap in progress.
    swapped: bool,
}

impl Device<'_> {
    /// Distinguish the initial contract from the one re-established after a swap.
    fn contract_established(&mut self, role: PowerRole) {
        if !self.contract_seen {
            self.contract_seen = true;
            info!("CI:PASS role:{}", role);
        } else if self.swapped {
            self.swapped = false;
            info!("CI:SWAP role:{}", role);
        } else {
            info!("Contract renegotiated role:{}", role);
        }
    }
}

// ---------------------------------------------------------------------------
// Source policy
// ---------------------------------------------------------------------------

impl SourceDevicePolicyManager for Device<'_> {
    fn source_capabilities(&mut self) -> SourceCapabilities {
        SourceCapabilities::new_vsafe5v_only(3 * 100)
    }

    async fn evaluate_request(&mut self, _request: &PowerSource) -> CapabilityResponse {
        CapabilityResponse::Accept
    }

    async fn transition_power(&mut self, _power_level: &PowerSource) -> Result<(), ()> {
        self.contract_established(PowerRole::Source);
        Ok(())
    }

    async fn hard_reset(&mut self) -> Result<(), ()> {
        Ok(())
    }

    async fn get_event(&mut self) -> usbpd::source::device_policy_manager::Event {
        // Initiator: request one PR_Swap after the first contract. Must be
        // cancellation safe, so clear the flag only right before returning.
        #[cfg(feature = "role-initiator")]
        if self.contract_seen && self.request_swap {
            Timer::after_millis(500).await;
            self.request_swap = false;
            info!("Requesting power role swap");
            return usbpd::source::device_policy_manager::Event::RequestPowerRoleSwap;
        }
        core::future::pending().await
    }
}

impl SourceEprDpm for Device<'_> {}

impl SourceDrpDpm for Device<'_> {
    async fn evaluate_swap_request(&mut self, _swap_request: usbpd::SwapType) -> bool {
        true
    }

    async fn cc_sink(&mut self) {
        self.swapped = true;
        if let Some(cc_phy) = &mut self.cc_phy {
            cc_phy.set_pull(CcPull::Sink);
            info!("CC pull asserted: Rd (becoming sink)");
        }
        // Close the consumer (VBUS sink path).
        if let Err(err) = self.tcpp.set_pd_role(PdRole::Sink).await {
            warn!("TCPP03 gate sink failed: {}", err);
        }
    }

    async fn disable(&mut self) {
        info!("Source power off");
    }

    async fn swap_data_role(&mut self, _role: usbpd::DataRole) {}
}

// ---------------------------------------------------------------------------
// Sink policy
// ---------------------------------------------------------------------------

impl SinkDevicePolicyManager for Device<'_> {
    async fn transition_power(&mut self, _accepted: &PowerSource) {
        self.contract_established(PowerRole::Sink);
    }

    async fn get_event(&mut self, _source_capabilities: &SourceCapabilities) -> SinkEvent {
        core::future::pending().await
    }
}

impl SinkEprDpm for Device<'_> {}

impl SinkDrpDpm for Device<'_> {
    async fn evaluate_swap_request(&mut self, _swap_request: usbpd::SwapType) -> bool {
        true
    }

    async fn cc_source(&mut self) {
        self.swapped = true;
        if let Some(cc_phy) = &mut self.cc_phy {
            cc_phy.set_pull(CcPull::Source3_0A);
            info!("CC pull asserted: Rp (becoming source)");
        }
        // Close the provider (VBUS source path).
        if let Err(err) = self.tcpp.set_pd_role(PdRole::Source).await {
            warn!("TCPP03 gate source failed: {}", err);
        }
    }

    async fn source_on(&mut self) {
        info!("Source on (VBUS applied)");
    }

    async fn disable(&mut self) {
        info!("Sink power off");
    }

    async fn swap_data_role(&mut self, _role: usbpd::DataRole) {}
}

// This device implements the full DPM supertrait bundle of each role, so
// the SinkDpm/SourceDpm trait impls for both remain empty (default)
impl SinkDpm for Device<'_> {}
impl SourceDpm for Device<'_> {}

// ---------------------------------------------------------------------------
// Attach detection & main loop
// ---------------------------------------------------------------------------

async fn wait_attached<T: ucpd::Instance>(cc_phy: &mut CcPhy<'_, T>) -> CableOrientation {
    loop {
        let (cc1, cc2) = cc_phy.vstate();
        if cc1 == CcVState::Lowest && cc2 == CcVState::Lowest {
            cc_phy.wait_for_vstate_change().await;
            continue;
        }

        // Attached. Wait for the CC lines to be stable for tCCDebounce (100ms).
        if embassy_time::with_timeout(Duration::from_millis(100), cc_phy.wait_for_vstate_change())
            .await
            .is_ok()
        {
            continue;
        }

        // State was stable for the debounce period, determine orientation.
        let (cc1, cc2) = cc_phy.vstate();
        return match (cc1, cc2) {
            (_, CcVState::Lowest) => CableOrientation::Normal,  // CC1 connected
            (CcVState::Lowest, _) => CableOrientation::Flipped, // CC2 connected
            _ => CableOrientation::DebugAccessoryMode,          // Both lines pulled (special cable)
        };
    }
}

/// Handle USB PD negotiation as a dual role port.
#[embassy_executor::task]
pub async fn ucpd_task(ucpd_resources: UcpdResources) {
    let mut tcpp = Tcpp::new(ucpd_resources.tcpp);
    loop {
        let ucpd_peri = unsafe { peripherals::UCPD1::steal() };
        let pin_cc1 = unsafe { peripherals::PB6::steal() };
        let pin_cc2 = unsafe { peripherals::PB4::steal() };
        let rx_dma = unsafe { peripherals::DMA1_CH1::steal() };
        let tx_dma = unsafe { peripherals::DMA1_CH2::steal() };

        let mut ucpd = Ucpd::new(ucpd_peri, pin_cc1, pin_cc2, Irqs {}, Default::default());

        // The initiator boots presenting Rp (source), the acceptor Rd (sink).
        let initial_role = if cfg!(feature = "role-initiator") {
            PowerRole::Source
        } else {
            PowerRole::Sink
        };

        let (initial_pd_role, initial_pull) = match initial_role {
            PowerRole::Source => (PdRole::Source, CcPull::SourceDefaultUsb),
            PowerRole::Sink => (PdRole::Sink, CcPull::Sink),
        };
        ucpd.cc_phy().set_pull(initial_pull);
        if let Err(err) = tcpp.init().await {
            warn!("TCPP03 init failed: {}", err);
        }

        info!("Waiting for USB connection");
        let cable_orientation = wait_attached(ucpd.cc_phy()).await;
        if matches!(cable_orientation, CableOrientation::DebugAccessoryMode) {
            continue;
        }
        info!("USB cable attached, orientation: {}", cable_orientation);

        let cc_sel = match cable_orientation {
            CableOrientation::Normal => {
                info!("Starting PD communication on CC1 pin");
                CcSel::Cc1
            }
            CableOrientation::Flipped => {
                info!("Starting PD communication on CC2 pin");
                CcSel::Cc2
            }
            CableOrientation::DebugAccessoryMode => unreachable!(),
        };

        if let Err(err) = tcpp.attach().await {
            warn!("TCPP03 attach failed: {}", err);
        }
        if let Err(err) = tcpp.set_pd_role(initial_pd_role).await {
            warn!("TCPP03 initial role gates failed: {}", err);
        }

        let (cc_phy, pd_phy) = ucpd.split_pd_phy(rx_dma, tx_dma, Irqs, cc_sel);

        let dpm = Device {
            cc_phy: Some(cc_phy),
            tcpp: &mut tcpp,
            request_swap: cfg!(feature = "role-initiator"),
            contract_seen: false,
            swapped: false,
        };

        let dpm_tcpp = dpm.tcpp as *mut Tcpp;

        let result = {
            // The driver and DPM share the TCPP03 reference (never concurrently).
            let driver = UcpdDrpDriver::new(pd_phy, unsafe { &mut *dpm_tcpp });
            let drp: DualRolePort<UcpdDrpDriver<'_>, EmbassyTimer, _> = DualRolePort::new(driver, dpm);
            info!("Run dual role port, initial role: {}", initial_role);

            drp.run(initial_role).await
        };

        if result.is_err() {
            warn!("Dual role port exited: {:?}", result.err());
            warn!("CI:FAIL");
            Timer::after_secs(1).await;
        }

        tcpp.check_faults().await;
        if let Err(err) = tcpp.detach().await {
            warn!("TCPP03 detach failed: {}", err);
        }
    }
}

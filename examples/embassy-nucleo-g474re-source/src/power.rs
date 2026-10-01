//! Handles USB PD negotiation as a Source.
use defmt::{Format, info, warn};
use defmt_rtt as _;
use embassy_futures::select::{Either, select};
use embassy_stm32::ucpd::{self, CcPhy, CcPull, CcSel, CcVState, PdPhy, Ucpd};
use embassy_stm32::{Peri, bind_interrupts, dma, peripherals};
use embassy_time::{Duration, Timer, with_timeout};
use panic_probe as _;
use usbpd::protocol_layer::message::data::request::PowerSource;
use usbpd::protocol_layer::message::data::source_capabilities::SourceCapabilities;
use usbpd::source::device_policy_manager::{
    CapabilityResponse, DevicePolicyManager, DrpDevicePolicyManager, EprDevicePolicyManager, Info, SourceDpm,
};
use usbpd::source::policy_engine::Source;
use usbpd::timers::Timer as SourceTimer;
use usbpd_tcpp03_m20::{PdRole, Tcpp, TcppResources};
use usbpd_traits::Driver as SourceDriver;

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

struct UcpdSourceDriver<'d> {
    pd_phy: PdPhy<'d, peripherals::UCPD1>,
    /// The TCPP03 gate controller. Measure VBUS with ADC.
    tcpp: &'d mut Tcpp,
}

impl<'d> UcpdSourceDriver<'d> {
    fn new(pd_phy: PdPhy<'d, peripherals::UCPD1>, tcpp: &'d mut Tcpp) -> Self {
        Self { pd_phy, tcpp }
    }
}

/// Generous time to wait for VBUS before proceeding, and the interval between ADC samples.
const VBUS_WAIT_TIMEOUT: Duration = Duration::from_secs(10);
const VBUS_POLL_INTERVAL: Duration = Duration::from_millis(100);

impl SourceDriver for UcpdSourceDriver<'_> {
    async fn wait_for_vbus(&mut self) {
        // VBUS observation is mode-independent only via the ADC.
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

async fn wait_detached<T: ucpd::Instance>(cc_phy: &mut CcPhy<'_, T>) {
    loop {
        let (cc1, cc2) = cc_phy.vstate();
        if cc1 == CcVState::Lowest && cc2 == CcVState::Lowest {
            return;
        }
        cc_phy.wait_for_vstate_change().await;
    }
}

async fn wait_attached<T: ucpd::Instance>(cc_phy: &mut CcPhy<'_, T>) -> CableOrientation {
    loop {
        let (cc1, cc2) = cc_phy.vstate();
        if cc1 == CcVState::Lowest && cc2 == CcVState::Lowest {
            cc_phy.wait_for_vstate_change().await;
            continue;
        }

        if with_timeout(Duration::from_millis(100), cc_phy.wait_for_vstate_change())
            .await
            .is_ok()
        {
            continue;
        };

        return match (cc1, cc2) {
            (_, CcVState::Lowest) => CableOrientation::Normal,
            (CcVState::Lowest, _) => CableOrientation::Flipped,
            _ => CableOrientation::DebugAccessoryMode,
        };
    }
}

struct EmbassySourceTimer {}

impl SourceTimer for EmbassySourceTimer {
    async fn after_millis(milliseconds: u64) {
        Timer::after_millis(milliseconds).await
    }
}

struct Device {
    contract_established: bool,
}

impl Device {
    fn new() -> Self {
        Self {
            contract_established: false,
        }
    }
}

impl SourceDpm for Device {}

impl DrpDevicePolicyManager for Device {}
impl EprDevicePolicyManager for Device {}

impl DevicePolicyManager for Device {
    fn source_capabilities(&mut self) -> SourceCapabilities {
        SourceCapabilities::new_vsafe5v_only(3 * 100)
    }

    async fn evaluate_request(&mut self, _request: &PowerSource) -> CapabilityResponse {
        info!("Evaluating sink request");
        CapabilityResponse::Accept
    }

    async fn transition_power(&mut self, _power_level: &PowerSource) -> Result<(), ()> {
        info!("Transitioning source power");
        Ok(())
    }

    async fn hard_reset(&mut self) -> Result<(), ()> {
        info!("Hard reset");
        Ok(())
    }

    async fn inform(&mut self, _info: Info) {
        if !self.contract_established {
            self.contract_established = true;
            info!("CI:PASS");
        }
    }
}

#[embassy_executor::task]
pub async fn ucpd_task(mut ucpd_resources: UcpdResources) {
    let mut tcpp = Tcpp::new(ucpd_resources.tcpp);
    loop {
        let mut ucpd = Ucpd::new(
            ucpd_resources.ucpd.reborrow(),
            ucpd_resources.pin_cc1.reborrow(),
            ucpd_resources.pin_cc2.reborrow(),
            Irqs {},
            Default::default(),
        );

        ucpd.cc_phy().set_pull(CcPull::Source3_0A);
        if let Err(err) = tcpp.init().await {
            warn!("TCPP03 init failed: {}", err);
        }
        info!("Waiting for USB connection");
        let cable_orientation = wait_attached(ucpd.cc_phy()).await;
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
            CableOrientation::DebugAccessoryMode => panic!("No PD communication in DAM"),
        };

        if let Err(err) = tcpp.attach().await {
            warn!("TCPP03 attach failed: {}", err);
        }
        if let Err(err) = tcpp.set_pd_role(PdRole::Source).await {
            warn!("TCPP03 set source role failed: {}", err);
        }

        let (mut cc_phy, pd_phy) = ucpd.split_pd_phy(
            ucpd_resources.rx_dma.reborrow(),
            ucpd_resources.tx_dma.reborrow(),
            Irqs,
            cc_sel,
        );

        {
            let driver = UcpdSourceDriver::new(pd_phy, &mut tcpp);
            let dpm = Device::new();
            let mut source: Source<UcpdSourceDriver<'_>, EmbassySourceTimer, _> = Source::new(driver, dpm, false);
            info!("Run source");

            match select(source.run(), wait_detached(&mut cc_phy)).await {
                Either::First(result) => warn!("Source loop broken with result: {}", result),
                Either::Second(_) => {
                    info!("Detached");
                }
            }
        }

        tcpp.check_faults().await;
        if let Err(err) = tcpp.detach().await {
            warn!("TCPP03 detach failed: {}", err);
        }
    }
}

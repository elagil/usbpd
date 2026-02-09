//! Handles USB PD negotiation.
use defmt::{Format, info, warn};
use embassy_futures::select::{Either, select};
use embassy_stm32::gpio::Output;
use embassy_stm32::ucpd::{self, CcPhy, CcPull, CcSel, CcVState, PdPhy, Ucpd};
use embassy_stm32::{Peri, bind_interrupts, dma, peripherals};
use embassy_time::{Duration, Ticker, Timer, with_timeout};
use uom::si::electric_potential;
use usbpd::protocol_layer::message::data::request::{self, CurrentRequest, VoltageRequest};
use usbpd::protocol_layer::message::data::source_capabilities::SourceCapabilities;
use usbpd::sink::device_policy_manager::{DevicePolicyManager, Event};
use usbpd::sink::policy_engine::Sink;
use usbpd::timers::Timer as SinkTimer;
use usbpd::units::ElectricPotential;
use usbpd_traits::Driver as SinkDriver;
use {defmt_rtt as _, panic_probe as _};

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
    pub tcpp01_m12_ndb: Output<'static>,
}

#[derive(Debug, Format)]
enum CableOrientation {
    Normal,
    Flipped,
    DebugAccessoryMode,
}

struct UcpdSinkDriver<'d> {
    /// The UCPD PD phy instance.
    pd_phy: PdPhy<'d, peripherals::UCPD1>,
}

impl<'d> UcpdSinkDriver<'d> {
    fn new(pd_phy: PdPhy<'d, peripherals::UCPD1>) -> Self {
        Self { pd_phy }
    }
}

impl SinkDriver for UcpdSinkDriver<'_> {
    async fn wait_for_vbus(&mut self) {
        // The sink policy engine is only running when attached. Therefore VBus is present.
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
        if cc1 == CcVState::LOWEST && cc2 == CcVState::LOWEST {
            return;
        }
        cc_phy.wait_for_vstate_change().await;
    }
}

// Returns true when the cable was attached.
async fn wait_attached<T: ucpd::Instance>(cc_phy: &mut CcPhy<'_, T>) -> CableOrientation {
    loop {
        let (cc1, cc2) = cc_phy.vstate();
        if cc1 == CcVState::LOWEST && cc2 == CcVState::LOWEST {
            // Detached, wait until attached by monitoring the CC lines.
            cc_phy.wait_for_vstate_change().await;
            continue;
        }

        // Attached, wait for CC lines to be stable for tCCDebounce (100..200ms).
        if with_timeout(Duration::from_millis(100), cc_phy.wait_for_vstate_change())
            .await
            .is_ok()
        {
            // State has changed, restart detection procedure.
            continue;
        };

        // State was stable for the complete debounce period, check orientation.
        return match (cc1, cc2) {
            (_, CcVState::LOWEST) => CableOrientation::Normal,  // CC1 connected
            (CcVState::LOWEST, _) => CableOrientation::Flipped, // CC2 connected
            _ => CableOrientation::DebugAccessoryMode,          // Both connected (special cable)
        };
    }
}

struct EmbassySinkTimer {}

impl SinkTimer for EmbassySinkTimer {
    async fn after_millis(milliseconds: u64) {
        Timer::after_millis(milliseconds).await
    }
}

/// Capabilities that are tested (cycled through).
#[derive(Format)]
enum TestCapabilities {
    Safe5V,
    Pps3V6,
    Pps5V5,
    Fixed9V,
    Max,
}

struct Device {
    ticker: Ticker,
    test_capabilities: TestCapabilities,
}

impl Default for Device {
    fn default() -> Self {
        Self {
            ticker: Ticker::every(Duration::from_secs(8)),
            test_capabilities: TestCapabilities::Safe5V,
        }
    }
}

impl DevicePolicyManager for Device {
    async fn request(&mut self, source_capabilities: &SourceCapabilities) -> request::PowerSource {
        info!("Found capabilities: {}", source_capabilities);

        request::PowerSource::new_fixed(
            request::CurrentRequest::Highest,
            request::VoltageRequest::Safe5V,
            source_capabilities,
        )
        .unwrap()
    }

    async fn get_event(&mut self, source_capabilities: &SourceCapabilities) -> Event {
        // Periodically request another power level.
        self.ticker.next().await;

        info!("Test capabilities: {}", self.test_capabilities);
        let (power_source, new_test_capabilities) = match self.test_capabilities {
            TestCapabilities::Safe5V => (
                request::PowerSource::new_fixed(CurrentRequest::Highest, VoltageRequest::Safe5V, source_capabilities),
                TestCapabilities::Fixed9V,
            ),
            TestCapabilities::Fixed9V => (
                request::PowerSource::new_fixed(
                    CurrentRequest::Highest,
                    VoltageRequest::Specific(ElectricPotential::new::<electric_potential::volt>(9)),
                    source_capabilities,
                ),
                TestCapabilities::Pps3V6,
            ),
            TestCapabilities::Pps3V6 => (
                request::PowerSource::new_pps(
                    request::CurrentRequest::Highest,
                    ElectricPotential::new::<electric_potential::millivolt>(3600),
                    source_capabilities,
                ),
                TestCapabilities::Pps5V5,
            ),
            TestCapabilities::Pps5V5 => (
                request::PowerSource::new_pps(
                    request::CurrentRequest::Highest,
                    ElectricPotential::new::<electric_potential::millivolt>(5500),
                    source_capabilities,
                ),
                TestCapabilities::Max,
            ),
            TestCapabilities::Max => (
                request::PowerSource::new_fixed(CurrentRequest::Highest, VoltageRequest::Highest, source_capabilities),
                TestCapabilities::Safe5V,
            ),
        };

        let event = if let Ok(power_source) = power_source {
            info!("Requesting power source {}", power_source);
            Event::RequestPower(power_source)
        } else {
            warn!("Capabilities not available {}", self.test_capabilities);
            Event::None
        };

        self.test_capabilities = new_test_capabilities;
        event
    }
}

/// Handle USB PD negotiation.
#[embassy_executor::task]
pub async fn ucpd_task(mut ucpd_resources: UcpdResources) {
    loop {
        let mut ucpd = Ucpd::new(
            ucpd_resources.ucpd.reborrow(),
            Irqs {},
            ucpd_resources.pin_cc1.reborrow(),
            ucpd_resources.pin_cc2.reborrow(),
            Default::default(),
        );

        ucpd.cc_phy().set_pull(CcPull::Sink);
        ucpd_resources.tcpp01_m12_ndb.set_high();

        info!("Waiting for USB connection");
        let cable_orientation = wait_attached(ucpd.cc_phy()).await;
        info!("USB cable attached, orientation: {}", cable_orientation);

        let cc_sel = match cable_orientation {
            CableOrientation::Normal => {
                info!("Starting PD communication on CC1 pin");
                CcSel::CC1
            }
            CableOrientation::Flipped => {
                info!("Starting PD communication on CC2 pin");
                CcSel::CC2
            }
            CableOrientation::DebugAccessoryMode => panic!("No PD communication in DAM"),
        };
        let (mut cc_phy, pd_phy) = ucpd.split_pd_phy(
            ucpd_resources.rx_dma.reborrow(),
            ucpd_resources.tx_dma.reborrow(),
            Irqs,
            cc_sel,
        );

        let driver = UcpdSinkDriver::new(pd_phy);
        let mut sink: Sink<UcpdSinkDriver<'_>, EmbassySinkTimer, _> = Sink::new(driver, Device::default());
        info!("Run sink");

        match select(sink.run(), wait_detached(&mut cc_phy)).await {
            Either::First(result) => warn!("Sink loop broken with result: {}", result),
            Either::Second(_) => {
                info!("Detached");
                continue;
            }
        }
    }
}

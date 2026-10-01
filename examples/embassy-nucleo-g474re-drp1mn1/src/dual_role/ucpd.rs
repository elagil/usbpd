//! UCPD Peripheral

use embassy_stm32::ucpd::{PdPhy, RxError, TxError};
use embassy_stm32::{Peri, peripherals};
use embassy_time::Timer;

pub(crate) struct EmbassyTimer;

impl usbpd::timers::Timer for EmbassyTimer {
    async fn after_millis(milliseconds: u64) {
        Timer::after_millis(milliseconds).await
    }
}

pub(crate) struct Ucpd {
    pub ucpd: Peri<'static, peripherals::UCPD1>,
    pub pin_cc1: Peri<'static, peripherals::PB6>,
    pub pin_cc2: Peri<'static, peripherals::PB4>,
    pub rx_dma: Peri<'static, peripherals::DMA1_CH3>,
    pub tx_dma: Peri<'static, peripherals::DMA1_CH4>,
}

pub(crate) struct UcpdDriver<'d> {
    /// The UCPD PD phy instance.
    pd_phy: PdPhy<'d, peripherals::UCPD1>,
}

impl<'d> UcpdDriver<'d> {
    pub(crate) fn new(pd_phy: PdPhy<'d, peripherals::UCPD1>) -> Self {
        Self { pd_phy }
    }
}

impl usbpd_traits::Driver for UcpdDriver<'_> {
    async fn wait_for_vbus(&mut self) {
        // The policy engine is only running when attached. Therefore VBus is present.
    }

    async fn receive(&mut self, buffer: &mut [u8]) -> Result<usize, usbpd_traits::DriverRxError> {
        self.pd_phy.receive(buffer).await.map_err(|err| match err {
            RxError::Crc | RxError::Overrun => usbpd_traits::DriverRxError::Discarded,
            RxError::HardReset => usbpd_traits::DriverRxError::HardReset,
        })
    }

    async fn transmit(&mut self, data: &[u8]) -> Result<(), usbpd_traits::DriverTxError> {
        self.pd_phy.transmit(data).await.map_err(|err| match err {
            TxError::Discarded => usbpd_traits::DriverTxError::Discarded,
            TxError::HardReset => usbpd_traits::DriverTxError::HardReset,
        })
    }

    async fn transmit_hard_reset(&mut self) -> Result<(), usbpd_traits::DriverTxError> {
        self.pd_phy.transmit_hardreset().await.map_err(|err| match err {
            TxError::Discarded => usbpd_traits::DriverTxError::Discarded,
            TxError::HardReset => usbpd_traits::DriverTxError::HardReset,
        })
    }
}

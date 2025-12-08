#![no_std]
#![no_main]

use defmt::info;
use embassy_executor::Spawner;
use usbpd_epr_example::power::{self, UcpdResources};
use {defmt_rtt as _, panic_probe as _};

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let mut stm32_config = embassy_stm32::Config::default();
    // HSI must be enabled for UCPD
    stm32_config.rcc.hsi = true;

    let p = embassy_stm32::init(stm32_config);

    info!("USB PD EPR Example");

    let ucpd_resources = UcpdResources {
        pin_cc1: p.PB6,
        pin_cc2: p.PB4,
        ucpd: p.UCPD1,
        rx_dma: p.DMA1_CH1,
        tx_dma: p.DMA1_CH2,
    };
    spawner.spawn(power::ucpd_task(ucpd_resources).unwrap());
}

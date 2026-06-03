#![no_std]
#![no_main]

use defmt::{info, unwrap};
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_stm32::gpio::{Input, Level, Pull, Speed};
use panic_probe as _;
use usbpd_g474re_source::power::{self, UcpdResources};

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let mut stm32_config = embassy_stm32::Config::default();
    stm32_config.rcc.hsi = true;

    let p = embassy_stm32::init(stm32_config);

    info!("USB PD Source Example (NUCLEO-G474RE + X-NUCLEO-DRP1MN1)");

    {
        let tcpp_pwren = embassy_stm32::gpio::Output::new(p.PC8, Level::Low, Speed::Low);
        let _tcpp_flgn = Input::new(p.PC5, Pull::Up);

        let ucpd_resources = UcpdResources {
            pin_cc1: p.PB6,
            pin_cc2: p.PB4,
            ucpd: p.UCPD1,
            rx_dma: p.DMA1_CH1,
            tx_dma: p.DMA1_CH2,
            tcpp_pwren,
        };
        spawner.spawn(unwrap!(power::ucpd_task(ucpd_resources)));
    }
}

#![no_std]
#![no_main]

use defmt::{info, unwrap};
use embassy_executor::Spawner;
use embassy_stm32::gpio::Output;
use embassy_stm32::Config;
use usbpd_testing::power::{self, UcpdResources};
use {defmt_rtt as _, panic_probe as _};

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let config = Config::default();
    let p = embassy_stm32::init(config);

    info!("Hi");

    // Launch UCPD task.
    {
        // This pin controls the dead-battery mode on the attached TCPP01-M12.
        let tcpp01_m12_ndb = Output::new(p.PA9, embassy_stm32::gpio::Level::Low, embassy_stm32::gpio::Speed::Low);

        let ucpd_resources = UcpdResources {
            pin_cc1: p.PB13,
            pin_cc2: p.PB14,
            ucpd: p.UCPD1,
            rx_dma: p.GPDMA1_CH0,
            tx_dma: p.GPDMA1_CH1,
            tcpp01_m12_ndb,
        };
        unwrap!(spawner.spawn(power::ucpd_task(ucpd_resources)));
    }
}

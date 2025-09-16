#![no_std]
#![no_main]

use defmt::{info, unwrap};
use embassy_executor::Spawner;
use embassy_stm32::gpio::Output;
use usbpd_testing::power::{self, UcpdResources};
use {defmt_rtt as _, panic_probe as _};

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_stm32::init(Default::default());

    info!("Hi");

    // Launch UCPD task.
    {
        // This pin controls the dead-battery mode on the attached TCPP01-M12.
        let tcpp01_m12_ndb = Output::new(p.PB5, embassy_stm32::gpio::Level::Low, embassy_stm32::gpio::Speed::Low);

        let ucpd_resources = UcpdResources {
            pin_cc1: p.PB6,
            pin_cc2: p.PB4,
            ucpd: p.UCPD1,
            rx_dma: p.DMA1_CH1,
            tx_dma: p.DMA1_CH2,
            tcpp01_m12_ndb,
        };
        spawner.spawn(unwrap!(power::ucpd_task(ucpd_resources)));
    }
}

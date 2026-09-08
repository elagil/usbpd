#![no_std]
#![no_main]

use defmt::{info, unwrap};
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_stm32::gpio::Output;
use panic_probe as _;
use usbpd_g474re_sink::power::{self, UcpdResources};

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_stm32::init(Default::default());

    info!("Hi");

    // Launch UCPD task.
    {
        // This pin controls the dead-battery mode on the attached TCPP01-M12.
        let tcpp_pwren = Output::new(p.PC8, embassy_stm32::gpio::Level::Low, embassy_stm32::gpio::Speed::Low);

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

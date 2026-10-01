#![no_std]
#![no_main]

use defmt::*;
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_stm32::gpio::{Input, Level, Output, Pull, Speed};
use embassy_stm32::time::Hertz;
use embassy_stm32::{bind_interrupts, dma, gpio, i2c, peripherals, ucpd};
use panic_probe as _;

mod dual_role;
mod led;

bind_interrupts!(pub struct Irqs {
    I2C1_EV => i2c::EventInterruptHandler<peripherals::I2C1>;
    I2C1_ER => i2c::ErrorInterruptHandler<peripherals::I2C1>;
    UCPD1 => ucpd::InterruptHandler<peripherals::UCPD1>;
    DMA1_CHANNEL1 => dma::InterruptHandler<peripherals::DMA1_CH1>;
    DMA1_CHANNEL2 => dma::InterruptHandler<peripherals::DMA1_CH2>;
    DMA1_CHANNEL3 => dma::InterruptHandler<peripherals::DMA1_CH3>;
    DMA1_CHANNEL4 => dma::InterruptHandler<peripherals::DMA1_CH4>;
    // EXTI0,
    // EXTI1,
    // USB_HP,
    // USB_LP,
});

const TCPP_ADDR_VDDIO: bool = false;

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_stm32::init(Default::default());

    info!("Hi");

    let ucpd = dual_role::Ucpd {
        ucpd: p.UCPD1.into(),
        pin_cc1: p.PB6.into(),
        pin_cc2: p.PB4.into(),
        rx_dma: p.DMA1_CH3.into(),
        tx_dma: p.DMA1_CH4.into(),
    };

    let gate_controller = {
        let enable = Output::new(p.PB2, Level::Low, Speed::High);
        let flgn = Input::new(p.PB1, Pull::Up);
        let i2c = {
            let mut config = embassy_stm32::i2c::Config::default();
            config.frequency = Hertz::khz(400);
            embassy_stm32::i2c::I2c::new(
                p.I2C1, // I2C
                p.PB8, // SCL
                p.PB7, // SDA
                p.DMA1_CH1,
                p.DMA1_CH2,
                Irqs,
                config,
            )
        };

        tcpp03_m20::Device::new(
            embassy_time::Delay,
            i2c,
            enable,
            flgn,
            TCPP_ADDR_VDDIO,
            tcpp03_m20::PdRole::Sink,
        )
    };

    let led = gpio::Output::new(p.PB0, gpio::Level::High, gpio::Speed::High);

    spawner.spawn(unwrap!(led::run(led)));

    spawner.spawn(unwrap!(dual_role::run(
        ucpd,
        gate_controller // source controller here
    )));
}

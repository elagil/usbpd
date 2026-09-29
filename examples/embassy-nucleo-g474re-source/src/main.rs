#![no_std]
#![no_main]

use defmt::{info, unwrap};
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_stm32::exti::{self, ExtiInput};
use embassy_stm32::gpio::{Level, Output, Pull, Speed};
use embassy_stm32::i2c::Config as I2cConfig;
use embassy_stm32::{bind_interrupts, i2c, interrupt, peripherals};
use panic_probe as _;
use usbpd_g474re_source::power::{self, UcpdResources};
use usbpd_tcpp03_m20::TcppResources;

bind_interrupts!(struct Irqs {
    I2C1_EV => i2c::EventInterruptHandler<peripherals::I2C1>;
    I2C1_ER => i2c::ErrorInterruptHandler<peripherals::I2C1>;
    EXTI9_5 => exti::InterruptHandler<interrupt::typelevel::EXTI9_5>;
});

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let mut stm32_config = embassy_stm32::Config::default();
    stm32_config.rcc.hsi = true;
    // ADC12 clock mux must be set explicitly for the VBUS sense ADC.
    stm32_config.rcc.mux.adc12sel = embassy_stm32::rcc::mux::Adcsel::SYS;

    let p = embassy_stm32::init(stm32_config);

    info!("USB PD Source Example (NUCLEO-G474RE + X-NUCLEO-DRP1M1)");

    {
        // TCPP03-M20 on the X-NUCLEO-DRP1M1: I2C1 (PB8/PB9), EN (PC8),
        // FLGn (PC5, active-low interrupt output), VBUS sense (PA0 -> ADC1).
        let i2c = embassy_stm32::i2c::I2c::new_blocking(p.I2C1, p.PB8, p.PB9, I2cConfig::default());
        let tcpp_pwren = Output::new(p.PC8, Level::Low, Speed::Low);
        let tcpp_flgn_exti = ExtiInput::new(p.PC5, p.EXTI5, Pull::Up, Irqs);

        let ucpd_resources = UcpdResources {
            pin_cc1: p.PB6,
            pin_cc2: p.PB4,
            ucpd: p.UCPD1,
            rx_dma: p.DMA1_CH1,
            tx_dma: p.DMA1_CH2,
            tcpp: TcppResources {
                i2c,
                enable: tcpp_pwren,
                flgn_exti: tcpp_flgn_exti,
                adc: p.ADC1,
                vbus_sense: p.PA0,
                adc_dma: p.DMA1_CH4,
            },
        };
        spawner.spawn(unwrap!(power::ucpd_task(ucpd_resources)));
    }
}

//! TCPP03-M20 gate controller driver on the X-NUCLEO-DRP1M1 shield, shared by all demos running on the NUCLEO-G474RE.
//!
//! Bring-up sequence mirrors ST's X-CUBE-TCPP BSP (`drp1m1_usbpd_pwr.c`).
//!
//! Register map (ST datasheet DS12566 / x-cube-tcpp `tcpp0203.h`):
//!     0x00 Control (WO), 0x01 Ack (RO), 0x02 Flag (RO).
//!
//! VBUS is measured through the shield's Vsense divider, which works in all power modes.
//!     PA0 -> ADC1_IN1, 200k/40k per BSP constants `DRP1M1_VSENSE_RA`/`_RB`, factor of six
//!
//! Note that the device's `VBUS_OK` flag (reg 0x02 bit 5) is only meaningful in Hibernate/LowPower.
//! In Normal mode the FLGn signal reports fault events only (ST BSP `PWR_TCPP0203_EventCallback`).
#![no_std]

/// Defmt-gated logging macro, so the crate builds without any logging feature.
macro_rules! log_info {
    ($s:literal $(, $x:expr)* $(,)?) => {
        {
            #[cfg(feature = "defmt")]
            ::defmt::info!($s $(, $x)*);
            #[cfg(not(feature = "defmt"))]
            let _ = ($( & $x ),*);
        }
    };
}

use embassy_stm32::adc::{self, Adc, AdcChannel as _, SampleTime};
use embassy_stm32::exti::ExtiInput;
use embassy_stm32::gpio::Output;
use embassy_stm32::i2c::{self, I2c};
use embassy_stm32::{Peri, bind_interrupts, dma};
use embassy_time::Timer;
use embedded_hal::i2c::I2c as _;

/// Device address.
const ADDRESS: u8 = 0x34;

// Serves the DMA transfers of VBUS ADC conversions.
bind_interrupts!(struct AdcIrqs {
    ADC1_2 => adc::InterruptHandler<embassy_stm32::peripherals::ADC1>;
    DMA1_CHANNEL4 => dma::InterruptHandler<embassy_stm32::peripherals::DMA1_CH4>;
});

/// VBUS voltage divider bottom resistor (BSP `DRP1M1_VSENSE_RB`) in kOhm.
const VBUS_DIV_RB_KOHM: u32 = 40;
/// VBUS voltage divider top resistor (BSP `DRP1M1_VSENSE_RA`) in kOhm.
const VBUS_DIV_RA_KOHM: u32 = 200;
/// Voltage reference for converting ADC readings, in mV.
const VREF_MV: u32 = 3300;
/// ADC full scale at 12-bit resolution.
const ADC_FULL_SCALE: u32 = 4095;

/// VBUS considered present by the BSP above this level (`USBPD_PWR_HIGH_VBUS_THRESHOLD`).
pub const VBUS_PRESENT_MV: u32 = 2800;
/// VBUS considered discharged by the BSP below this level (`USBPD_PWR_LOW_VBUS_THRESHOLD`).
pub const VBUS_DISCHARGED_MV: u32 = 750;
/// VBUS considered vSafe5V by the BSP above this level (`USBPD_PWR_VBUS_THRESHOLD_5V`).
pub const VBUS_5V_OK_MV: u32 = 3900;

#[derive(thiserror::Error, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Error {
    /// I2c error.
    #[error("i2c error")]
    I2c(#[from] i2c::Error),
    /// Incorrect device type detected.
    #[error("incorrect device type")]
    DeviceType,
    /// The requested state transition is not valid for the current state,
    /// e.g. gating a role while not attached.
    #[error("invalid command")]
    InvalidCommand,
}

/// Control register (0x00) bits. Reset value: 0b0000_1000
pub mod control {
    pub const VCONN_DISCHARGE: u8 = 1 << 7;
    pub const VBUS_DISCHARGE: u8 = 1 << 6;
    /// 00 = Hibernate, 01 = Normal, 10 = LowPower
    pub const POWER_MODE_HIBERNATE: u8 = 0b00 << 4;
    pub const POWER_MODE_NORMAL: u8 = 0b01 << 4;
    pub const POWER_MODE_LOWPOWER: u8 = 0b10 << 4;
    /// Gate driver consumer (VBUS sink path). Closed = 0.
    pub const GD_CONSUMER_CLOSED: u8 = 0;
    pub const GD_CONSUMER_OPEN: u8 = 1 << 3;
    /// Gate driver provider (VBUS source path). Closed = 1.
    pub const GD_PROVIDER_CLOSED: u8 = 1 << 2;
    pub const GD_PROVIDER_OPEN: u8 = 0;
    /// VCONN switch select: 00 = both open, 01 = V1 (CC2), 10 = V2 (CC1)
    pub const VCONN_OPEN: u8 = 0b00;
    pub const VCONN_CC1: u8 = 0b10;
    pub const VCONN_CC2: u8 = 0b01;
    /// ST reset value: consumer switch closed, everything else off.
    pub const RESET: u8 = GD_CONSUMER_CLOSED | POWER_MODE_HIBERNATE;
}

/// Flag register (0x02) bits. `true` = latched fault active.
///
/// Note: `VBUS_OK` (bit 5) only asserts in Hibernate/LowPower mode. In Normal mode FLGn reports fault events only.
/// VBUS presence is therefore measured via the ADC.
pub mod flag {
    /// TCPP03 = 0, TCPP02 = 1.
    pub const DEVICE_TYPE_TCPP03: bool = false;
    /// Only meaningful in Hibernate/LowPower mode.
    pub const VBUS_OK: u8 = 1 << 5;
    pub const OVP_CC: u8 = 1 << 4;
    pub const OTP: u8 = 1 << 3;
    pub const OVP_VBUS: u8 = 1 << 2;
    pub const OCP_VBUS: u8 = 1 << 1;
    pub const OCP_VCONN: u8 = 1 << 0;
}

/// Gate-driver power delivery role.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum PdRole {
    /// Both gate drivers open.
    #[default]
    None,
    /// Consumer gate closed (VBUS sink path).
    Sink,
    /// Provider gate closed (VBUS source path).
    Source,
}

/// Protection faults currently latched in the device.
#[derive(Debug, Clone, Copy, Default)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct DeviceProtectionFlags(pub u8);

impl DeviceProtectionFlags {
    pub const fn is_empty(&self) -> bool {
        self.0 == 0
    }

    pub const fn vbus_bad(&self) -> bool {
        self.0 & flag::VBUS_OK == 0
    }

    pub const fn over_voltage_cc(&self) -> bool {
        self.0 & flag::OVP_CC != 0
    }

    pub const fn over_temperature(&self) -> bool {
        self.0 & flag::OTP != 0
    }

    pub const fn over_voltage_vbus(&self) -> bool {
        self.0 & flag::OVP_VBUS != 0
    }

    pub const fn over_current_vbus(&self) -> bool {
        self.0 & flag::OCP_VBUS != 0
    }

    pub const fn over_current_vconn(&self) -> bool {
        self.0 & flag::OCP_VCONN != 0
    }
}

impl From<u8> for DeviceProtectionFlags {
    fn from(flags: u8) -> Self {
        Self(flags)
    }
}

/// Tracked driver state.
#[derive(Debug, Clone, Copy)]
struct State {
    mode: Mode,
    pd: PdRole,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Mode {
    Hibernate,
    Normal,
    LowPower,
}

/// Hardware handles for the TCPP03 on the DRP1M1 shield.
pub struct TcppResources {
    pub i2c: I2c<'static, embassy_stm32::mode::Blocking, i2c::mode::Master>,
    pub enable: Output<'static>,
    pub flgn_exti: ExtiInput<'static, embassy_stm32::mode::Async>,
    /// ADC that the VBUS sense line is routed to (ADC1 on the DRP1M1).
    pub adc: Peri<'static, embassy_stm32::peripherals::ADC1>,
    /// VBUS connector sense pin, routed through the shield's divider (PA0).
    pub vbus_sense: Peri<'static, embassy_stm32::peripherals::PA0>,
    /// DMA channel used for ADC conversions (DMA1_CH4 in ST's DRP1M1 projects).
    pub adc_dma: Peri<'static, embassy_stm32::peripherals::DMA1_CH4>,
}

pub struct Tcpp {
    i2c: I2c<'static, embassy_stm32::mode::Blocking, i2c::mode::Master>,
    enable: Output<'static>,
    flgn: ExtiInput<'static, embassy_stm32::mode::Async>,
    adc: Adc<'static, embassy_stm32::peripherals::ADC1, embassy_stm32::mode::Async>,
    vbus_sense: embassy_stm32::adc::BorrowedAdcChannel<'static, embassy_stm32::peripherals::ADC1>,
    adc_dma: Peri<'static, embassy_stm32::peripherals::DMA1_CH4>,
    state: State,
}

impl Tcpp {
    /// Create the driver, leaving the chip in reset (EN low) until [`Tcpp::init`].
    pub fn new(resources: TcppResources) -> Self {
        let TcppResources {
            i2c,
            enable,
            flgn_exti,
            adc,
            vbus_sense,
            adc_dma,
        } = resources;
        let mut enable = enable;
        enable.set_low();

        let adc_config = adc::Config::default();
        let adc = Adc::new(adc, AdcIrqs, adc_config);
        let vbus_sense = vbus_sense.degrade_adc();

        Self {
            i2c,
            enable,
            flgn: flgn_exti,
            adc,
            vbus_sense,
            adc_dma,
            state: State {
                mode: Mode::Hibernate,
                pd: PdRole::None,
            },
        }
    }

    async fn write_control(&mut self, control: u8) -> Result<(), Error> {
        self.i2c.write(ADDRESS, &[0x00, control])?;
        Ok(())
    }

    async fn read(&mut self, reg: u8) -> Result<u8, Error> {
        let mut buffer = [0u8; 1];
        self.i2c.write_read(ADDRESS, &[reg], &mut buffer)?;
        Ok(buffer[0])
    }

    /// Reset and bring up the TCPP03.
    ///
    /// Enters [`Mode::LowPower`] mode with both gate drivers open: the MCU's `UCPD` pull is exposed on the connector,
    /// enabling attach detection, while the connector CC pull observed by the MCU comes from the peer port.
    pub async fn init(&mut self) -> Result<(), Error> {
        // EN low -> high resets the device register state.
        self.enable.set_low();
        Timer::after_millis(10).await;
        self.enable.set_high();
        Timer::after_millis(10).await;

        // Reg2 bit 7 must read 0 for a TCPP03 (bit set = TCPP02).
        let flags = self.read(0x02).await?;
        if flags & (1 << 7) != 0 {
            log_info!("TCPP03: unexpected device type, flag reg: {:#b}", flags);
            return Err(Error::DeviceType);
        }

        // Restore register defaults, then enter LowPower with both gate drivers open
        // and the VCONN switch open (dead-battery handling off).
        self.write_control(control::RESET | control::POWER_MODE_LOWPOWER | control::GD_CONSUMER_OPEN)
            .await?;
        self.state.mode = Mode::LowPower;
        self.state.pd = PdRole::None;
        log_info!("TCPP03: initialized, lowpower mode");

        Ok(())
    }

    /// Switch to Normal mode so VBUS/BMC pass switches engage. Requires [`Mode::LowPower`].
    pub async fn attach(&mut self) -> Result<(), Error> {
        if self.state.mode != Mode::LowPower {
            return Err(Error::InvalidCommand);
        }

        self.read_ack_then(|control| {
            *control = (*control & !0b11_0000) | control::POWER_MODE_NORMAL;
        })
        .await?;
        self.state.mode = Mode::Normal;
        log_info!("TCPP03: attached, normal mode");

        Ok(())
    }

    /// Return to [`Mode::LowPower`] with gate drivers open. Re-applies the default role.
    pub async fn detach(&mut self) -> Result<(), Error> {
        if self.state.mode != Mode::Normal {
            return Err(Error::InvalidCommand);
        }

        self.write_control(
            control::GD_CONSUMER_OPEN | control::GD_PROVIDER_OPEN | control::VCONN_OPEN | control::POWER_MODE_LOWPOWER,
        )
        .await?;
        self.state.mode = Mode::LowPower;
        self.state.pd = PdRole::None;
        log_info!("TCPP03: detached, lowpower mode");

        Ok(())
    }

    /// Gate the power path for the given role. Requires [`Mode::Normal`].
    pub async fn set_pd_role(&mut self, role: PdRole) -> Result<(), Error> {
        if self.state.mode != Mode::Normal {
            return Err(Error::InvalidCommand);
        }

        if self.state.pd == role {
            return Ok(());
        }

        self.read_ack_then(|control| match role {
            PdRole::None => {
                *control = (*control & !control::GD_PROVIDER_CLOSED) | control::GD_CONSUMER_OPEN;
            }
            PdRole::Sink => {
                *control = (*control & !control::GD_PROVIDER_CLOSED) & !control::GD_CONSUMER_OPEN;
            }
            PdRole::Source => {
                *control = (*control | control::GD_PROVIDER_CLOSED) & !control::GD_CONSUMER_OPEN;
            }
        })
        .await?;
        self.state.pd = role;

        log_info!("TCPP03: pd role {:?}", role);

        Ok(())
    }

    /// Current VBUS level in mV, from a single ADC conversion.
    pub async fn vbus_mv(&mut self) -> Result<u16, Error> {
        let mut buffer = [0u16; 1];
        self.adc
            .read_sequence(
                self.adc_dma.reborrow(),
                AdcIrqs,
                [(self.vbus_sense.reborrow_adc(), SampleTime::Cycles2475)].into_iter(),
                None,
                &mut buffer,
            )
            .await;

        let mv = ((buffer[0] as u32 * VREF_MV * (VBUS_DIV_RA_KOHM + VBUS_DIV_RB_KOHM))
            / (VBUS_DIV_RB_KOHM * ADC_FULL_SCALE)) as u16;
        Ok(mv)
    }

    /// Await the next FLGn assertion and return the decoded flags.
    pub async fn wait_flgn(&mut self) -> Result<DeviceProtectionFlags, Error> {
        self.flgn.wait_for_low().await;
        Ok(self.read(0x02).await?.into())
    }

    /// Log any latched protection faults (FLGn events, log-only in this port).
    pub async fn check_faults(&mut self) {
        if let Ok(flags) = self.read(0x02).await {
            let active = flags & (flag::OVP_CC | flag::OTP | flag::OVP_VBUS | flag::OCP_VBUS | flag::OCP_VCONN);
            if active != 0 {
                log_info!("TCPP03: fault flags {:#b}", active);
            }
        }
    }

    /// Ack-to-Control write-back (ST BSP `TCPP0203_ModifyReg0` pattern).
    async fn read_ack_then(&mut self, modify: impl FnOnce(&mut u8)) -> Result<(), Error> {
        let ack = self.read(0x01).await?;
        let mut control = ack;
        modify(&mut control);
        self.write_control(control).await
    }
}

//! Handles USB PD negotiation with EPR (Extended Power Range) support.
use defmt::{Format, info, warn};
use embassy_futures::select::{Either, select};
use embassy_stm32::ucpd::{self, CcPhy, CcPull, CcSel, CcVState, PdPhy, Ucpd};
use embassy_stm32::{Peri, bind_interrupts, dma, peripherals};
use embassy_time::{Duration, Timer, with_timeout};
use uom::si::electric_current::{centiampere, milliampere};
use uom::si::electric_potential::millivolt;
use uom::si::power::{milliwatt, watt};
#[cfg(not(feature = "avs"))]
use usbpd::_50millivolts_mod::_50millivolts;
#[allow(unused_imports)] // Avs is used in AVS feature mode
use usbpd::protocol_layer::message::data::request::{
    Avs, CurrentRequest, EprRequestDataObject, FixedVariableSupply, PowerSource, VoltageRequest,
};
use usbpd::protocol_layer::message::data::source_capabilities::{Augmented, PowerDataObject, SourceCapabilities};
use usbpd::sink::device_policy_manager::{DevicePolicyManager, Event};
use usbpd::sink::policy_engine::Sink;
use usbpd::timers::Timer as SinkTimer;
use usbpd::units::Power;
use usbpd_traits::Driver as SinkDriver;
use {defmt_rtt as _, panic_probe as _};

bind_interrupts!(struct Irqs {
    UCPD1 => ucpd::InterruptHandler<peripherals::UCPD1>;
    DMA1_CHANNEL1 => dma::InterruptHandler<peripherals::DMA1_CH1>;
    DMA1_CHANNEL2 => dma::InterruptHandler<peripherals::DMA1_CH2>;
});

// ============================================================================
// Configuration for Fixed EPR mode (default)
// ============================================================================
#[cfg(not(feature = "avs"))]
mod config {
    /// Target voltage for EPR Fixed request (28V in 50 mV units)
    pub const TARGET_EPR_VOLTAGE_RAW: u16 = 28 * 20;
    /// Target current for EPR Fixed request (4A in 10 mA units)
    pub const TARGET_EPR_CURRENT_RAW: u16 = 4 * 100;
    /// Operational PDP for EPR mode entry (28V × 4A = 112W)
    pub const OPERATIONAL_PDP_WATTS: u32 = 112;
}

// ============================================================================
// Configuration for AVS (Adjustable Voltage Supply) mode
// ============================================================================
#[cfg(feature = "avs")]
mod config {
    /// Target voltage for AVS request in volts
    pub const TARGET_AVS_VOLTAGE_V: u32 = 24;
    /// Target current for AVS request (5A in 50 mA units)
    pub const TARGET_AVS_CURRENT_RAW: u16 = 5 * 20; // 5A = 100 in 50 mA units
    /// Operational PDP for EPR mode entry with AVS (24V × 5A = 120W)
    pub const OPERATIONAL_PDP_WATTS: u32 = 120;
}

use config::*;

/// Print source capabilities in a nice format using defmt
fn print_capabilities(caps: &SourceCapabilities) {
    let is_epr = caps.is_epr_capabilities();
    if is_epr {
        info!("=== EPR Source Capabilities ({} PDOs) ===", caps.pdos().len());
    } else {
        info!("=== SPR Source Capabilities ({} PDOs) ===", caps.pdos().len());
    }

    for (i, pdo) in caps.pdos().iter().enumerate() {
        let position = i + 1;
        print_pdo(position as u8, pdo);
    }
    info!("=========================================");
}

/// Print a single PDO
fn print_pdo(position: u8, pdo: &PowerDataObject) {
    match pdo {
        PowerDataObject::FixedSupply(f) => {
            // Check for separator (null PDO)
            if f.0 == 0 {
                info!("  PDO[{}]: --- (separator) ---", position);
                return;
            }

            let voltage_mv = f.voltage().get::<millivolt>();
            let current_ma = f.max_current().get::<milliampere>();
            let power_mw = voltage_mv * current_ma / 1000;

            let drp = if f.dual_role_power() { " DRP" } else { "" };
            let usb = if f.usb_communications_capable() { " USB" } else { "" };
            let drd = if f.dual_role_data() { " DRD" } else { "" };
            let up = if f.unconstrained_power() { " UP" } else { "" };
            let epr = if f.epr_mode_capable() { " EPR" } else { "" };

            info!(
                "  PDO[{}]: Fixed {}mV @ {}mA ({}mW){}{}{}{}{}",
                position, voltage_mv, current_ma, power_mw, drp, usb, drd, up, epr
            );
        }
        PowerDataObject::Battery(b) => {
            let min_mv = b.min_voltage().get::<millivolt>();
            let max_mv = b.max_voltage().get::<millivolt>();
            let power_mw = b.max_power().get::<milliwatt>();
            info!("  PDO[{}]: Battery {}-{}mV @ {}mW", position, min_mv, max_mv, power_mw);
        }
        PowerDataObject::VariableSupply(v) => {
            let min_mv = v.min_voltage().get::<millivolt>();
            let max_mv = v.max_voltage().get::<millivolt>();
            let current_ma = v.max_current().get::<milliampere>();
            info!(
                "  PDO[{}]: Variable {}-{}mV @ {}mA",
                position, min_mv, max_mv, current_ma
            );
        }
        PowerDataObject::Augmented(aug) => match aug {
            Augmented::Spr(pps) => {
                let min_mv = pps.min_voltage().get::<millivolt>();
                let max_mv = pps.max_voltage().get::<millivolt>();
                let current_ma = pps.max_current().get::<milliampere>();
                let limited = if pps.pps_power_limited() { " (limited)" } else { "" };
                info!(
                    "  PDO[{}]: PPS {}-{}mV @ {}mA{}",
                    position, min_mv, max_mv, current_ma, limited
                );
            }
            Augmented::Epr(avs) => {
                let min_mv = avs.min_voltage().get::<millivolt>();
                let max_mv = avs.max_voltage().get::<millivolt>();
                let power_mw = avs.pd_power().get::<milliwatt>();
                info!("  PDO[{}]: EPR AVS {}-{}mV @ {}mW", position, min_mv, max_mv, power_mw);
            }
            Augmented::Unknown(raw) => {
                info!("  PDO[{}]: Augmented(0x{:08X})", position, raw);
            }
        },
        PowerDataObject::Unknown(u) => {
            info!("  PDO[{}]: Unknown(0x{:08X})", position, u.0);
        }
    }
}

pub struct UcpdResources {
    pub ucpd: Peri<'static, peripherals::UCPD1>,
    pub pin_cc1: Peri<'static, peripherals::PB6>,
    pub pin_cc2: Peri<'static, peripherals::PB4>,
    pub rx_dma: Peri<'static, peripherals::DMA1_CH1>,
    pub tx_dma: Peri<'static, peripherals::DMA1_CH2>,
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

#[derive(Default)]
struct Device {
    /// Tracks whether we've requested to enter EPR mode
    entered_epr_mode: bool,
}

impl DevicePolicyManager for Device {
    async fn inform(&mut self, source_capabilities: &SourceCapabilities) {
        // Print capabilities when we receive them
        print_capabilities(source_capabilities);
    }

    async fn get_event(&mut self, source_capabilities: &SourceCapabilities) -> Event {
        // After initial SPR negotiation, enter EPR mode if source is EPR capable
        if !self.entered_epr_mode
            && let Some(PowerDataObject::FixedSupply(fixed)) = source_capabilities.pdos().first()
            && fixed.epr_mode_capable()
        {
            info!("Source is EPR capable, entering EPR mode");
            self.entered_epr_mode = true;
            return Event::EnterEprMode(Power::new::<watt>(OPERATIONAL_PDP_WATTS));
        }
        core::future::pending().await
    }

    async fn request(&mut self, source_capabilities: &SourceCapabilities) -> PowerSource {
        // Check if source is EPR capable (from first PDO)
        let source_epr_capable = source_capabilities
            .pdos()
            .first()
            .map(|pdo| {
                if let PowerDataObject::FixedSupply(fixed) = pdo {
                    fixed.epr_mode_capable()
                } else {
                    false
                }
            })
            .unwrap_or(false);

        // If we have EPR capabilities, look for 28V EPR PDO
        if source_capabilities.is_epr_capabilities() {
            // Find 28V EPR PDO (EPR PDOs start at position 8)
            for (position, pdo) in source_capabilities.epr_pdos() {
                if pdo.is_zero_padding() {
                    continue;
                }

                // Fixed EPR mode (default)
                #[cfg(not(feature = "avs"))]
                if let PowerDataObject::FixedSupply(fixed) = pdo {
                    let voltage_raw = fixed.voltage().get::<_50millivolts>() as u16;

                    // Check if this is 28V (560 in 50 mV units)
                    if voltage_raw == TARGET_EPR_VOLTAGE_RAW {
                        // Request our target current, but cap at source's max
                        let source_max = fixed.max_current().get::<centiampere>() as u16;
                        let current = if TARGET_EPR_CURRENT_RAW > source_max {
                            warn!(
                                "Source max {} mA < target {} mA, using source max",
                                source_max as u32 * 10,
                                TARGET_EPR_CURRENT_RAW as u32 * 10
                            );
                            source_max
                        } else {
                            TARGET_EPR_CURRENT_RAW
                        };

                        info!(
                            "Requesting 28V EPR PDO at position {} with {}mA",
                            position,
                            current as u32 * 10
                        );

                        let rdo = FixedVariableSupply(0)
                            .with_object_position(position)
                            .with_usb_communications_capable(true)
                            .with_no_usb_suspend(true)
                            .with_epr_mode_capable(true)
                            .with_raw_operating_current(current)
                            .with_raw_max_operating_current(current);

                        return PowerSource::EprRequest(EprRequestDataObject { rdo: rdo.0, pdo: *pdo });
                    }
                }

                // AVS (Adjustable Voltage Supply) mode
                #[cfg(feature = "avs")]
                if let PowerDataObject::Augmented(Augmented::Epr(avs)) = pdo {
                    let min_mv = avs.min_voltage().get::<millivolt>();
                    let max_mv = avs.max_voltage().get::<millivolt>();
                    let target_mv = TARGET_AVS_VOLTAGE_V * 1000;

                    // Check if this AVS PDO supports our target voltage
                    if min_mv <= target_mv && target_mv <= max_mv {
                        // Calculate max current from PDP (in 50 mA units)
                        let pdp_mw = avs.pd_power().get::<milliwatt>();
                        let max_current_ma = pdp_mw / TARGET_AVS_VOLTAGE_V;
                        let max_current_raw = (max_current_ma / 50) as u16;

                        let current = if TARGET_AVS_CURRENT_RAW > max_current_raw {
                            warn!(
                                "Source max {}mA < target {}mA at {}V, using source max",
                                max_current_raw as u32 * 50,
                                TARGET_AVS_CURRENT_RAW as u32 * 50,
                                TARGET_AVS_VOLTAGE_V
                            );
                            max_current_raw
                        } else {
                            TARGET_AVS_CURRENT_RAW
                        };

                        // AVS voltage is in 25 mV units with LSB 2 bits = 0 (effective 100 mV steps)
                        // Per USB PD 3.2 Table 6.26
                        let voltage_raw = ((TARGET_AVS_VOLTAGE_V * 1000 / 25) & !0x3) as u16;

                        info!(
                            "Requesting {}V AVS at position {} with {}mA",
                            TARGET_AVS_VOLTAGE_V,
                            position,
                            current as u32 * 50
                        );

                        let rdo = Avs(0)
                            .with_object_position(position)
                            .with_usb_communications_capable(true)
                            .with_no_usb_suspend(true)
                            .with_epr_mode_capable(true)
                            .with_raw_output_voltage(voltage_raw)
                            .with_raw_operating_current(current);

                        return PowerSource::EprRequest(EprRequestDataObject { rdo: rdo.0, pdo: *pdo });
                    }
                }
            }

            #[cfg(not(feature = "avs"))]
            warn!("28V EPR PDO not found, falling back to SPR");

            #[cfg(feature = "avs")]
            warn!("AVS PDO with suitable voltage range not found, falling back to SPR");
        }

        // For SPR request: set epr_mode_capable bit if source supports EPR
        // This is required before EPR mode entry - the source checks this bit
        if source_epr_capable
            && let Ok(PowerSource::FixedVariableSupply(rdo)) =
                PowerSource::new_fixed(CurrentRequest::Highest, VoltageRequest::Highest, source_capabilities)
        {
            info!("Requesting SPR PDO {} with EPR capable flag", rdo.object_position());
            // Set epr_mode_capable bit for EPR mode entry
            let rdo = rdo.with_epr_mode_capable(true);
            return PowerSource::FixedVariableSupply(rdo);
        }

        // Fall back to standard request (no EPR)
        match PowerSource::new_fixed(CurrentRequest::Highest, VoltageRequest::Highest, source_capabilities) {
            Ok(ps) => {
                info!("Requesting highest SPR voltage (PDO {})", ps.object_position());
                ps
            }
            Err(_) => {
                warn!("No suitable PDO found, falling back to 5V");
                PowerSource::new_fixed(CurrentRequest::Highest, VoltageRequest::Safe5V, source_capabilities).unwrap()
            }
        }
    }

    async fn transition_power(&mut self, accepted: &PowerSource) {
        info!("Power transition accepted: PDO position {}", accepted.object_position());
    }
}

/// Handle USB PD negotiation with EPR support.
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

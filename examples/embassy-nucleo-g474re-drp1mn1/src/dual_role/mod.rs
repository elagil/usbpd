//! Dual-Role-Port implementation

use embassy_futures::select::{Either, select};
use embassy_stm32::gpio::{Input, Output};
use embassy_stm32::i2c::{self, I2c, Master};
use embassy_stm32::mode::Async;
use embassy_stm32::ucpd::{CcSel, Config as UcpdConfig, Ucpd as UcpdPeri};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::watch::Watch;
use embassy_time::{Delay, Timer};
use usbpd::dual_role::DualRolePort;

mod attach;
mod dpm;
mod ucpd;

pub(crate) use ucpd::Ucpd;
use ucpd::{EmbassyTimer, UcpdDriver};

#[derive(Debug, Clone, Copy)]
pub(crate) enum Connection {
    Disconnected,
    Sink,
    Source,
}

impl From<usbpd::PowerRole> for Connection {
    fn from(value: usbpd::PowerRole) -> Self {
        match value {
            usbpd::PowerRole::Source => Connection::Source,
            usbpd::PowerRole::Sink => Connection::Sink,
        }
    }
}

pub(crate) static CONNECTION_STATUS: Watch<CriticalSectionRawMutex, Connection, 2> =
    Watch::new_with(Connection::Disconnected);

pub(crate) type GateController =
    tcpp03_m20::Device<Delay, I2c<'static, Async, Master>, i2c::Error, Output<'static>, Input<'static>>;

#[embassy_executor::task]
pub async fn run(
    mut ucpd: Ucpd,
    mut gate_controller: GateController,
    // TODO: source power controller
) {
    loop {
        // Re-borrow ucpd peripheral
        let mut ucpd_peri = UcpdPeri::new(
            ucpd.ucpd.reborrow(),
            crate::Irqs,
            ucpd.pin_cc1.reborrow(),
            ucpd.pin_cc2.reborrow(),
            UcpdConfig::default(),
        );

        // Initialize gate controller
        if let Err(_) = gate_controller.init().await {
            defmt::error!("Could not initialize gate controller!");
            Timer::after_millis(100).await;
            continue;
        }

        // Wait for an attachment on a CC line with an initial power role
        let Ok((cc_sel, initial_role)) = attach::wait_attached(ucpd_peri.cc_phy()).await else {
            defmt::error!("Could not attach!");
            Timer::after_millis(100).await;
            continue;
        };

        // TODO: This would be the location to attempt a power role switch the initial role is undesirable

        // Attach the gate controller with the CC line
        if let Err(_) = gate_controller
            .attach(match cc_sel {
                CcSel::CC1 => tcpp03_m20::CcState::CC1,
                CcSel::CC2 => tcpp03_m20::CcState::CC2,
            })
            .await
        {
            defmt::error!("Could not attach gate controller!");
            Timer::after_millis(100).await;
            continue;
        }

        // Set the gate controller to the initial role
        if let Err(_) = gate_controller
            .set_pd(match initial_role {
                usbpd::PowerRole::Source => tcpp03_m20::PdRole::Source,
                usbpd::PowerRole::Sink => tcpp03_m20::PdRole::Sink,
            })
            .await
        {
            defmt::error!("Could not set gate controller pd mode!");
            Timer::after_millis(100).await;
            continue;
        }

        defmt::info!("DRP: Attached as {}", initial_role);
        CONNECTION_STATUS.sender().send(initial_role.into());

        // Create the DRP runtime objects and tasks
        let (mut cc_phy, pd_phy) =
            ucpd_peri.split_pd_phy(ucpd.rx_dma.reborrow(), ucpd.tx_dma.reborrow(), crate::Irqs, cc_sel);
        let dpm = dpm::DevicePolicyManager::new(
            // TODO: cc_phy requester,
            &mut gate_controller,
            // TODO: source power controller
        );
        let driver = UcpdDriver::new(pd_phy);
        let dual_role_port: DualRolePort<UcpdDriver<'_>, EmbassyTimer, dpm::DevicePolicyManager<'_>> =
            DualRolePort::new(driver, dpm);

        // Run the DRP runtime
        match select(
            dual_role_port.run(initial_role),
            attach::run_while_attached(&mut cc_phy),
        )
        .await
        {
            Either::First(result) => {
                if let Err(err) = result {
                    defmt::warn!("DPM: PD loop exited with error! {}", err);
                } else {
                    defmt::info!("DPM: PD loop ended!");
                }
            }
            Either::Second(_) => defmt::info!("USB disconnected!"),
        }

        CONNECTION_STATUS.sender().send(Connection::Disconnected);

        let _ = gate_controller.discharge().await;
        let _ = gate_controller.detach().await;

        Timer::after_millis(500).await;
    }
}

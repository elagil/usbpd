use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::rwlock::RwLock;
use embassy_time::Timer;
use heapless::Vec;
use uom::si::electric_potential::volt;
use usbpd::protocol_layer::message::data::request;
use usbpd::protocol_layer::message::data::sink_capabilities::{self, SinkCapabilities, SinkPowerDataObject};
use usbpd::protocol_layer::message::data::source_capabilities::{self, PowerDataObject, SourceCapabilities};
use usbpd::sink;
use usbpd::sink::device_policy_manager::{
    DevicePolicyManager as SinkDpm, DrpDevicePolicyManager as SinkDrpDpm, EprDevicePolicyManager as SinkEprDpm,
    Event as SinkEvent, Info as SinkInfo, SinkDpm as FullSinkDpm,
};
use usbpd::source::device_policy_manager::{
    CapabilityResponse, DevicePolicyManager as SourceDpm, DrpDevicePolicyManager as SourceDrpDpm,
    EprDevicePolicyManager as SourceEprDpm, Event as SourceEvent, Info as SourceInfo, SourceDpm as FullSourceDpm,
};
use usbpd::units::ElectricPotential;

use super::GateController;

pub struct DevicePolicyManager<'a> {
    // TODO: cc_phy state change requester
    gate_controller: &'a mut GateController,
    // TODO: source controller here
}

impl<'a> DevicePolicyManager<'a> {
    pub fn new(gate_controller: &'a mut GateController) -> Self {
        Self { gate_controller }
    }
}

// --- S I N K DEVICE POLICY MANAGER ---

#[derive(Debug, Clone, Copy, defmt::Format)]
pub enum SinkCapability {
    Safe5V,
    Fixed9V,
}

impl SinkCapability {
    fn all() -> SinkCapabilities {
        SinkCapabilities::new(Vec::from_slice(&[Self::Safe5V.into(), Self::Fixed9V.into()]).unwrap())
    }

    const fn from_object_pos(pos: u8) -> Self {
        if pos == 0 { Self::Safe5V } else { Self::Fixed9V }
    }
}

impl From<SinkCapability> for SinkPowerDataObject {
    fn from(value: SinkCapability) -> Self {
        match value {
            SinkCapability::Safe5V => Self::FixedSupply(sink_capabilities::FixedSupply::new_vsafe5v(500 / 10)),
            SinkCapability::Fixed9V => Self::FixedSupply(sink_capabilities::FixedSupply::new(9000 / 50, 500 / 10)),
        }
    }
}

static SINK_STATUS: RwLock<CriticalSectionRawMutex, SinkCapability> = RwLock::new(SinkCapability::Safe5V);

impl SinkDpm for DevicePolicyManager<'_> {
    fn sink_capabilities(&self) -> SinkCapabilities {
        SinkCapability::all()
    }

    async fn request(&mut self, source_capabilities: &SourceCapabilities) -> request::PowerSource {
        request::PowerSource::new_fixed(
            request::CurrentRequest::Highest,
            request::VoltageRequest::Safe5V,
            source_capabilities,
        )
        .unwrap()
    }

    async fn transition_power(&mut self, accepted: &request::PowerSource) {
        // This method is only OK because we own both the source & sink
        let sink_capability = match accepted {
            request::PowerSource::FixedVariableSupply(fvs) => SinkCapability::from_object_pos(fvs.object_position()),
            _ => defmt::unreachable!("Sink: Transitioned to a non FVS?!"),
        };

        *SINK_STATUS.write().await = sink_capability;

        defmt::info!("Sink: Transitioned power to {:?}! ({:?})", sink_capability, accepted);
    }

    /// Request to swap between Safe5V and Fixed9V every 10 seconds
    async fn get_event(&mut self, source_capabilities: &source_capabilities::SourceCapabilities) -> SinkEvent {
        Timer::after_secs(10).await;
        let status = SINK_STATUS.read().await;

        let power_request = match *status {
            SinkCapability::Safe5V => request::PowerSource::new_fixed(
                request::CurrentRequest::Highest,
                request::VoltageRequest::Safe5V,
                source_capabilities,
            ),
            SinkCapability::Fixed9V => request::PowerSource::new_fixed(
                request::CurrentRequest::Highest,
                request::VoltageRequest::Specific(ElectricPotential::new::<volt>(9)),
                source_capabilities,
            ),
        };

        match power_request {
            Ok(request) => sink::device_policy_manager::Event::RequestPower(request),
            Err(_) => {
                defmt::warn!("Sink: Could not match new request to source capability!");
                sink::device_policy_manager::Event::None
            }
        }
    }

    async fn inform(&mut self, source_capabilities: &SourceCapabilities, info: SinkInfo) {
        defmt::info!(
            "Sink DPM informed by Source:
                Info: {:?},
                Capabilities: {:?}",
            info,
            source_capabilities
        );
    }

    async fn hard_reset(&mut self) {
        // This matters more on the `SourceDPM` side.
        //
        // This is a good place to reset ICs that depend more on the
        // supplied power being more than `vSafe5V`, say for example, a USB hub!
    }

    async fn evaluate_vconn_swap_request(&mut self) -> bool {
        false
    }

    async fn drive_vconn(&mut self, _on: bool) -> Result<(), ()> {
        // This is a TODO in the `tcpp03-m20` crate!
        // self.gate_controller.vconn_drive(on);
        Err(())
    }
}

impl SinkDrpDpm for DevicePolicyManager<'_> {
    async fn evaluate_swap_request(&mut self, swap_request: usbpd::SwapType) -> bool {
        match swap_request {
            usbpd::SwapType::Data => false,
            usbpd::SwapType::Power => true,
        }
    }

    async fn set_fr_swap_detect(enable: bool) {
        todo!("Set fast role swap detection to `enable`");
    }

    async fn disable(&mut self) {
        // Do nothing, since sink isn't actually powering anything
    }

    async fn cc_source(&mut self) {
        todo!("Swap CC pins to source config!");
    }

    async fn source_on(&mut self) {
        todo!("Turn on the source controller to Safe5V!");
    }
}

impl SinkEprDpm for DevicePolicyManager<'_> {}

impl FullSinkDpm for DevicePolicyManager<'_> {}

// --- S O U R C E DEVICE POLICY MANAGER ---

#[derive(Debug, Clone, Copy, defmt::Format)]
pub enum SourceCapability {
    Safe5V,
    Fixed9V,
}

impl SourceCapability {
    fn all() -> SourceCapabilities {
        SourceCapabilities::new_with_pdos(Vec::from_slice(&[Self::Safe5V.into(), Self::Fixed9V.into()]).unwrap())
    }

    const fn from_object_pos(pos: u8) -> Self {
        if pos == 0 { Self::Safe5V } else { Self::Fixed9V }
    }
}

impl From<SourceCapability> for PowerDataObject {
    fn from(value: SourceCapability) -> Self {
        match value {
            SourceCapability::Safe5V => {
                let safe_5v = source_capabilities::FixedSupply::v_safe_5v(50);
                Self::FixedSupply(safe_5v)
            }
            SourceCapability::Fixed9V => {
                let fixed_9v = source_capabilities::FixedSupply::default()
                    .with_raw_voltage(9000 / 50)
                    .with_raw_max_current(300 / 10);
                Self::FixedSupply(fixed_9v)
            }
        }
    }
}

impl SourceDpm for DevicePolicyManager<'_> {
    fn source_capabilities(&mut self) -> SourceCapabilities {
        SourceCapability::all()
    }

    async fn evaluate_request(&mut self, request: &request::PowerSource) -> CapabilityResponse {
        // Only matching to the object position is kinda cheaty and not technically correct
        match request {
            request::PowerSource::FixedVariableSupply(fvs) => {
                if fvs.object_position() == 0 || fvs.object_position() == 1 {
                    defmt::info!("Source: FVS request accepted! {:?}", fvs);
                    CapabilityResponse::Accept
                } else {
                    defmt::warn!("Source: Unrecognized FVS object position requested!");
                    CapabilityResponse::Reject
                }
            }
            _ => {
                defmt::warn!("Source: Non-FVS power source requested!");
                CapabilityResponse::Reject
            }
        }
    }

    async fn transition_power(&mut self, power_level: &request::PowerSource) -> Result<(), ()> {
        let source_capability = match *power_level {
            request::PowerSource::FixedVariableSupply(fvs) => SourceCapability::from_object_pos(fvs.object_position()),
            _ => {
                defmt::error!("Source: Attempted to transition to impossible power source!");
                return Err(());
            }
        };

        todo!("Transition power to `source_capability`");

        Ok(())
    }

    async fn hard_reset(&mut self) -> Result<(), ()> {
        todo!("Reset power to safe5v!")
    }

    async fn drive_vconn(&mut self, on: bool) -> Result<(), ()> {
        // This is a TODO in the `tcpp03-m20` crate!
        // self.gate_controller.vconn_drive(on);
        Err(())
    }

    async fn get_event(&mut self) -> SourceEvent {
        Timer::after_secs(10).await;
        defmt::info!("Source: Requesting sink capabilities...");
        SourceEvent::RequestSinkCapabilities
    }

    async fn inform(&mut self, info: SourceInfo) {
        defmt::info!("Source: Informed by sink of information! {:?}", info);
    }
}

impl SourceDrpDpm for DevicePolicyManager<'_> {
    async fn sink_capabilities(&mut self) -> SinkCapabilities {
        SinkCapability::all()
    }

    async fn evaluate_swap_request(&mut self, swap_request: usbpd::SwapType) -> bool {
        match swap_request {
            usbpd::SwapType::Data => false,
            usbpd::SwapType::Power => true,
        }
    }

    async fn fr_swap_signaled(&mut self) -> bool {
        todo!("Wait for an FR swap to be signaled by the sink!");
    }

    async fn discharge_vbus(&mut self) {
        if self.gate_controller.discharge_vbus().await.is_err() {
            defmt::warn!("Source: Could not discharge vbus!")
        }
    }

    async fn disable(&mut self) {
        todo!("Disable the source controller.");
    }

    async fn cc_sink(&mut self) {
        todo!("Set the CC pins to sink configuration");
    }
}

impl SourceEprDpm for DevicePolicyManager<'_> {}

impl FullSourceDpm for DevicePolicyManager<'_> {}

//! The mocks fit a `usbcc::Port`, by value and by `&mut`.
//!
//! `Port::new` is still a skeleton, so these tests only type-check the combinations.

use std::future::Future;

use usbcc::{Attachment, DevicePolicyManager, Error, Port};
use usbcc_sim::{MockCcPhy, MockDpm, MockPowerPath, MockVbusSense, Mocks, Sim, SimError, SimPort, SimTimer};

type SimResult<T> = Result<T, Error<SimError, SimError, SimError>>;

fn by_value(mocks: Mocks) -> (SimPort, MockDpm) {
    (
        Port::new(mocks.cc, mocks.vbus, mocks.power, Default::default()),
        mocks.dpm,
    )
}

fn by_mut_ref<'a>(
    cc: &'a mut MockCcPhy,
    vbus: &'a mut MockVbusSense,
    power: &'a mut MockPowerPath,
) -> Port<&'a mut MockCcPhy, &'a mut MockVbusSense, &'a mut MockPowerPath, SimTimer> {
    Port::new(cc, vbus, power, Default::default())
}

fn wait_attached<'a, DPM: DevicePolicyManager>(
    port: &'a mut SimPort,
    dpm: &'a mut DPM,
) -> impl Future<Output = SimResult<Attachment>> + 'a {
    port.wait_attached(dpm)
}

fn wait_attached_by_mut_ref<'a>(
    port: &'a mut Port<&'a mut MockCcPhy, &'a mut MockVbusSense, &'a mut MockPowerPath, SimTimer>,
    dpm: &'a mut MockDpm,
) -> impl Future<Output = SimResult<Attachment>> + 'a {
    port.wait_attached(dpm)
}

fn assert_send<T: Send>(_: &T) {}

#[test]
fn mocks_fit_a_port() {
    // Not called: `Port::new` is `todo!()` for now.
    let _ = by_value;
    let _ = by_mut_ref;
    let _ = wait_attached::<MockDpm>;
    let _ = wait_attached_by_mut_ref;
    let _ = Mocks::into_port;
}

#[tokio::test(start_paused = true)]
async fn mocks_and_their_futures_are_send() {
    use usbcc_traits::{CcPhy, PowerPath, VbusSense};

    let (sim, mut mocks) = Sim::new();
    assert_send(&sim);
    assert_send(&mocks);
    assert_send(&mocks.cc.wait_for_cc_change());
    assert_send(&mocks.vbus.wait_vbus_outside(0, 1));
    assert_send(&mocks.power.prepare_vbus_source());
    assert_send(&mocks.dpm.get_event());
}

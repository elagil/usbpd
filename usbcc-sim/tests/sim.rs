//! The behavior of the mocks that `usbcc` relies on, one test per property.

use std::pin::pin;
use std::time::Duration;

use usbcc::{DevicePolicyManager, Event};
use usbcc_sim::{Sim, SimError, SimFutureExt};
use usbcc_traits::{CcPhy, CcState, PowerPath, RpLevel, Termination, VbusPath, VbusSense};

const RP: CcState = CcState::Rp(RpLevel::Default);
const OPEN: CcState = CcState::Open;

#[tokio::test(start_paused = true)]
async fn cc_change_is_reported_once_per_read() {
    let (sim, mut mocks) = Sim::new_open();
    let cc = &mut mocks.cc;

    // Nothing changed yet.
    assert_eq!(cc.wait_for_cc_change().settle().await, None);

    // An unread change returns immediately, and then counts as read.
    sim.cc.set_cc(RP, OPEN);
    assert_eq!(cc.wait_for_cc_change().poll_once().await, Some(Ok((RP, OPEN))));
    assert_eq!(cc.wait_for_cc_change().settle().await, None);

    // `cc_state` reads too.
    sim.cc.set_cc(OPEN, RP);
    assert_eq!(cc.cc_state().await, Ok((OPEN, RP)));
    assert_eq!(cc.wait_for_cc_change().settle().await, None);

    // A reverted change is still a change; setting the same state is not.
    sim.cc.set_cc(RP, RP);
    sim.cc.set_cc(OPEN, RP);
    assert_eq!(cc.wait_for_cc_change().poll_once().await, Some(Ok((OPEN, RP))));
    sim.cc.set_cc(OPEN, RP);
    assert_eq!(cc.wait_for_cc_change().settle().await, None);
}

#[tokio::test(start_paused = true)]
async fn initial_terminations() {
    let (open, _) = Sim::new_open();
    assert_eq!(open.cc.terminations(), (Termination::Open, Termination::Open));
    let (dead_battery, _) = Sim::new_dead_battery();
    assert_eq!(dead_battery.cc.terminations(), (Termination::Rd, Termination::Rd));
}

#[tokio::test(start_paused = true)]
async fn pending_waits_wake_on_stimulus() {
    let (sim, mut mocks) = Sim::new_open();

    let mut cc = pin!(mocks.cc.wait_for_cc_change());
    assert_eq!(cc.as_mut().settle().await, None);
    sim.cc.set_cc(RP, OPEN);
    assert_eq!(cc.as_mut().poll_once().await, Some(Ok((RP, OPEN))));

    sim.vbus.set_mv(5000);
    let mut vbus = pin!(mocks.vbus.wait_vbus_below(3500));
    assert_eq!(vbus.as_mut().settle().await, None);
    sim.vbus.set_mv(0);
    assert_eq!(vbus.as_mut().poll_once().await, Some(Ok(())));
}

#[tokio::test(start_paused = true)]
async fn vbus_waits_are_level_based_with_strict_bounds() {
    let (sim, mut mocks) = Sim::new_open();
    let vbus = &mut mocks.vbus;

    sim.vbus.set_mv(800);
    assert_eq!(vbus.wait_vbus_outside(800, 5500).settle().await, None);
    sim.vbus.set_mv(799);
    assert_eq!(vbus.wait_vbus_outside(800, 5500).poll_once().await, Some(Ok(())));

    // An excursion reverted before the wait runs again isn't seen.
    sim.vbus.set_mv(5000);
    let mut wait = pin!(vbus.wait_vbus_above(5500));
    assert_eq!(wait.as_mut().settle().await, None);
    sim.vbus.set_mv(6000);
    sim.vbus.set_mv(5000);
    assert_eq!(wait.as_mut().settle().await, None);
}

#[tokio::test(start_paused = true)]
async fn cancelled_get_event_loses_no_event() {
    let (sim, mut mocks) = Sim::new_open();
    let dpm = &mut mocks.dpm;

    let mut pending = Box::pin(dpm.get_event());
    assert_eq!(pending.as_mut().poll_once().await, None);
    sim.dpm.push_event(Event::Disable);
    drop(pending);

    sim.dpm.push_event(Event::Enable);
    assert_eq!(dpm.get_event().await, Event::Disable);
    assert_eq!(dpm.get_event().await, Event::Enable);
    assert_eq!(dpm.get_event().settle().await, None);
}

#[tokio::test(start_paused = true)]
async fn injected_error_fails_only_the_next_call() {
    let (sim, mut mocks) = Sim::new_open();
    let error = SimError("injected");

    sim.power.fail_next(error);
    assert_eq!(mocks.power.set_vbus(VbusPath::Sink).await, Err(error));
    assert_eq!(sim.power.state().vbus, VbusPath::Off);
    assert_eq!(mocks.power.set_vbus(VbusPath::Sink).await, Ok(()));
    assert_eq!(sim.power.state().vbus, VbusPath::Sink);
}

#[tokio::test(start_paused = true)]
async fn log_has_virtual_timestamps_in_call_order() {
    let (sim, mut mocks) = Sim::new_open();
    mocks.vbus.vbus_mv().await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    mocks.power.set_active(true).await.unwrap();
    mocks.cc.cc_state().await.unwrap();

    let times: Vec<_> = sim.log.entries().iter().map(|entry| entry.at.as_millis()).collect();
    assert_eq!(times, [0, 100, 100]);
    assert_eq!(sim.log.len(), 3);
}

#[tokio::test(start_paused = true)]
async fn run_for_reports_completion_and_advances_time() {
    let start = tokio::time::Instant::now();
    let sleep = |ms| tokio::time::sleep(Duration::from_millis(ms));

    assert_eq!(sleep(100).run_for(Duration::from_millis(100)).await, Some(()));
    assert_eq!(start.elapsed(), Duration::from_millis(100));
    assert_eq!(sleep(150).run_for(Duration::from_millis(100)).await, None);
    assert_eq!(start.elapsed(), Duration::from_millis(200));
}

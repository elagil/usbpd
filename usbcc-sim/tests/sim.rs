//! The behavior of the mocks that `usbcc` relies on, one test per property.

use std::pin::pin;
use std::time::Duration;

use usbcc::{DevicePolicyManager, Event};
use usbcc_sim::{Sim, SimError, run_for};
use usbcc_traits::{CcPhy, CcState, PowerPath, RpLevel, VbusSense};

const RP: CcState = CcState::Rp(RpLevel::Default);
const OPEN: CcState = CcState::Open;

/// Longer than anything that isn't waiting takes, so a future still pending after it waits.
const LONG: Duration = Duration::from_secs(1);

#[tokio::test(start_paused = true)]
async fn cc_change_is_reported_once_per_read() {
    let (sim, mut mocks) = Sim::new();
    let cc = &mut mocks.cc;

    // Nothing changed yet.
    assert_eq!(run_for(LONG, cc.wait_for_cc_change()).await, None);

    // An unread change returns immediately, and then counts as read.
    sim.cc.set_cc(RP, OPEN);
    assert_eq!(
        run_for(Duration::ZERO, cc.wait_for_cc_change()).await,
        Some(Ok((RP, OPEN)))
    );
    assert_eq!(run_for(LONG, cc.wait_for_cc_change()).await, None);

    // `cc_state` reads too.
    sim.cc.set_cc(OPEN, RP);
    assert_eq!(cc.cc_state().await, Ok((OPEN, RP)));
    assert_eq!(run_for(LONG, cc.wait_for_cc_change()).await, None);

    // A reverted change is still a change; setting the same state is not.
    sim.cc.set_cc(RP, RP);
    sim.cc.set_cc(OPEN, RP);
    assert_eq!(
        run_for(Duration::ZERO, cc.wait_for_cc_change()).await,
        Some(Ok((OPEN, RP)))
    );
    sim.cc.set_cc(OPEN, RP);
    assert_eq!(run_for(LONG, cc.wait_for_cc_change()).await, None);
}

#[tokio::test(start_paused = true)]
async fn pending_waits_wake_on_stimulus() {
    let (sim, mut mocks) = Sim::new();

    let mut cc = pin!(mocks.cc.wait_for_cc_change());
    assert_eq!(run_for(LONG, &mut cc).await, None);
    sim.cc.set_cc(RP, OPEN);
    assert_eq!(run_for(Duration::ZERO, &mut cc).await, Some(Ok((RP, OPEN))));

    sim.vbus.set_mv(5000);
    let mut vbus = pin!(mocks.vbus.wait_vbus_below(3500));
    assert_eq!(run_for(LONG, &mut vbus).await, None);
    sim.vbus.set_mv(0);
    assert_eq!(run_for(Duration::ZERO, &mut vbus).await, Some(Ok(())));
}

#[tokio::test(start_paused = true)]
async fn vbus_waits_are_level_based_with_strict_bounds() {
    let (sim, mut mocks) = Sim::new();
    let vbus = &mut mocks.vbus;

    sim.vbus.set_mv(800);
    assert_eq!(run_for(LONG, vbus.wait_vbus_outside(800, 5500)).await, None);
    sim.vbus.set_mv(799);
    assert_eq!(
        run_for(Duration::ZERO, vbus.wait_vbus_outside(800, 5500)).await,
        Some(Ok(()))
    );

    // An excursion reverted before the wait runs again isn't seen.
    sim.vbus.set_mv(5000);
    let mut wait = pin!(vbus.wait_vbus_above(5500));
    assert_eq!(run_for(LONG, &mut wait).await, None);
    sim.vbus.set_mv(6000);
    sim.vbus.set_mv(5000);
    assert_eq!(run_for(LONG, &mut wait).await, None);
}

#[tokio::test(start_paused = true)]
async fn cancelled_get_event_loses_no_event() {
    let (sim, mut mocks) = Sim::new();
    let dpm = &mut mocks.dpm;

    let mut pending = Box::pin(dpm.get_event());
    assert_eq!(run_for(Duration::ZERO, &mut pending).await, None);
    sim.dpm.push_event(Event::Disable);
    drop(pending);

    sim.dpm.push_event(Event::Enable);
    assert_eq!(dpm.get_event().await, Event::Disable);
    assert_eq!(dpm.get_event().await, Event::Enable);
    assert_eq!(run_for(LONG, dpm.get_event()).await, None);
}

#[tokio::test(start_paused = true)]
async fn injected_error_fails_only_the_next_call() {
    let (sim, mut mocks) = Sim::new();
    let error = SimError("injected");

    sim.power.fail_next(error);
    assert_eq!(mocks.power.set_vbus_sink(true).await, Err(error));
    assert!(!sim.power.state().vbus_sink);
    assert_eq!(mocks.power.set_vbus_sink(true).await, Ok(()));
    assert!(sim.power.state().vbus_sink);
}

#[tokio::test(start_paused = true)]
#[should_panic(expected = "called while `PowerPath::prepare_vbus_source` is in progress")]
async fn concurrent_power_path_calls_panic() {
    let (sim, mut mocks) = Sim::new();
    sim.power.set_source_ready(false);
    // Leaking a pending call is the only way past the borrow checker.
    std::mem::forget(mocks.power.prepare_vbus_source());
    let _ = mocks.power.set_active(true).await;
}

#[tokio::test(start_paused = true)]
async fn log_has_virtual_timestamps_in_call_order() {
    let (sim, mut mocks) = Sim::new();
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

    assert_eq!(run_for(Duration::from_millis(100), sleep(100)).await, Some(()));
    assert_eq!(start.elapsed(), Duration::from_millis(100));
    assert_eq!(run_for(Duration::from_millis(100), sleep(150)).await, None);
    assert_eq!(start.elapsed(), Duration::from_millis(200));
}

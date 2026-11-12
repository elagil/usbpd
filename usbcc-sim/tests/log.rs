//! Timestamps and ordering of the shared log.

use std::time::Duration;

use tokio::time::{advance, sleep};
use usbcc::{DevicePolicyManager, Info, RpLevel};
use usbcc_sim::{Call, CcCall, DpmCall, Entry, Log, PowerCall, Sim, VbusCall, run_for};
use usbcc_traits::{CcPhy, PowerPath, Termination, VbusSense};

fn ms(ms: u64) -> Duration {
    Duration::from_millis(ms)
}

#[tokio::test(start_paused = true)]
async fn entries_carry_virtual_time_in_call_order() {
    let (sim, mut mocks) = Sim::new();
    assert!(sim.log.is_empty());
    assert_eq!(sim.log.now(), Duration::ZERO);

    mocks
        .cc
        .set_termination(Termination::Rd, Termination::Rd)
        .await
        .unwrap();
    mocks.power.set_active(true).await.unwrap();
    sleep(ms(100)).await;
    mocks.vbus.vbus_mv().await.unwrap();
    advance(ms(50)).await;
    mocks.dpm.inform(Info::TypeCCurrent(RpLevel::Current3A0)).await;
    mocks.cc.cc_state().await.unwrap();

    assert_eq!(sim.log.now(), ms(150));
    let entries = sim.log.entries();
    assert_eq!(
        entries,
        [
            (
                0,
                CcCall::SetTermination {
                    cc1: Termination::Rd,
                    cc2: Termination::Rd
                }
                .into()
            ),
            (0, PowerCall::SetActive(true).into()),
            (100, VbusCall::VbusMv.into()),
            (150, DpmCall::Inform(Info::TypeCCurrent(RpLevel::Current3A0)).into()),
            (150, CcCall::CcState.into()),
        ]
        .map(|(at, call): (u64, Call)| Entry {
            at: ms(at),
            port: Sim::DEFAULT_PORT,
            call
        })
    );
}

#[tokio::test(start_paused = true)]
async fn calls_are_stamped_when_made_not_when_complete() {
    let (sim, mut mocks) = Sim::new();
    sim.power.set_source_ready(false);
    sim.vbus.set_mv(5000);

    let (prepared, ()) = tokio::join!(mocks.power.prepare_vbus_source(), async {
        sleep(ms(30)).await;
        // Interleaved with the pending call.
        mocks.vbus.wait_vbus_above(4750).await.unwrap();
        sleep(ms(20)).await;
        sim.power.set_source_ready(true);
    });
    prepared.unwrap();
    mocks.power.set_vbus_source(true).await.unwrap();

    let stamped: Vec<_> = sim
        .log
        .entries()
        .into_iter()
        .map(|entry| (entry.at, entry.call))
        .collect();
    assert_eq!(
        stamped,
        [
            (ms(0), PowerCall::PrepareVbusSource.into()),
            (ms(30), VbusCall::WaitVbusAbove { mv: 4750 }.into()),
            (ms(50), PowerCall::SetVbusSource(true).into()),
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn epoch_is_log_creation() {
    sleep(ms(500)).await;
    let log = Log::new();
    let (sim, mut mocks) = Sim::with_log(&log, "late");
    assert_eq!(log.epoch().elapsed(), Duration::ZERO);
    sleep(ms(5)).await;
    mocks.cc.cc_state().await.unwrap();
    assert_eq!(sim.log.entries()[0].at, ms(5));
}

#[tokio::test(start_paused = true)]
async fn take_and_clear() {
    let (sim, mut mocks) = Sim::new();
    mocks.cc.cc_state().await.unwrap();
    mocks.vbus.vbus_mv().await.unwrap();
    assert_eq!(sim.log.len(), 2);

    let taken = sim.log.take();
    assert_eq!(taken.len(), 2);
    assert!(sim.log.is_empty());

    sleep(ms(10)).await;
    mocks.cc.cc_state().await.unwrap();
    // Timestamps still count from the creation of the log.
    assert_eq!(sim.log.entries()[0].at, ms(10));
    sim.log.clear();
    assert!(sim.log.is_empty());
    assert!(sim.log.calls().is_empty());
}

#[tokio::test(start_paused = true)]
async fn ports_share_one_log() {
    let log = Log::new();
    let (_a, mut a) = Sim::with_log(&log, "a");
    let (b_sim, mut b) = Sim::with_log(&log, "b");

    a.power.set_active(true).await.unwrap();
    sleep(ms(1)).await;
    b.power.set_active(true).await.unwrap();
    a.cc.cc_state().await.unwrap();

    let ports: Vec<_> = b_sim.log.entries().iter().map(|entry| (entry.at, entry.port)).collect();
    assert_eq!(ports, [(ms(0), "a"), (ms(1), "b"), (ms(1), "a")]);
}

#[tokio::test(start_paused = true)]
async fn cancelled_calls_stay_recorded() {
    let (sim, mut mocks) = Sim::new();
    assert_eq!(run_for(ms(20), mocks.cc.wait_for_cc_change()).await, None);
    assert_eq!(sim.log.calls(), [Call::from(CcCall::WaitForCcChange)]);
}

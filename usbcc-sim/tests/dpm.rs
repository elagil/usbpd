//! Behavior of the DPM mock.

use std::time::Duration;

use tokio::time::{Instant, sleep};
use usbcc::{DevicePolicyManager, Event, Info, RpLevel};
use usbcc_sim::{Call, DpmCall, Log, MockDpm, run_for};

#[tokio::test(start_paused = true)]
async fn get_event_pends_while_queue_is_empty() {
    let log = Log::new();
    let (mut dpm, _handle) = MockDpm::new(&log, "port");
    assert_eq!(run_for(Duration::from_secs(10), dpm.get_event()).await, None);
    assert!(log.is_empty());
}

#[tokio::test(start_paused = true)]
async fn queued_events_return_in_order_immediately() {
    let log = Log::new();
    let (mut dpm, handle) = MockDpm::new(&log, "port");
    handle.push_event(Event::Disable);
    handle.push_event(Event::Enable);
    assert_eq!(handle.queued_events(), [Event::Disable, Event::Enable]);

    let start = Instant::now();
    assert_eq!(dpm.get_event().await, Event::Disable);
    assert_eq!(dpm.get_event().await, Event::Enable);
    assert_eq!(start.elapsed(), Duration::ZERO);
    assert!(handle.queued_events().is_empty());
    assert_eq!(
        log.calls(),
        [
            DpmCall::EventReturned(Event::Disable),
            DpmCall::EventReturned(Event::Enable)
        ]
        .map(Call::from)
    );
}

#[tokio::test(start_paused = true)]
async fn get_event_wakes_on_push() {
    let log = Log::new();
    let (mut dpm, handle) = MockDpm::new(&log, "port");
    let start = Instant::now();
    let (event, ()) = tokio::join!(dpm.get_event(), async {
        sleep(Duration::from_millis(70)).await;
        handle.push_event(Event::ErrorRecovery);
    });
    assert_eq!(event, Event::ErrorRecovery);
    assert_eq!(start.elapsed(), Duration::from_millis(70));
    assert_eq!(log.entries()[0].at, Duration::from_millis(70));
}

#[tokio::test(start_paused = true)]
async fn cancelling_get_event_loses_no_event() {
    let log = Log::new();
    let (mut dpm, handle) = MockDpm::new(&log, "port");

    // Cancelled while pending, then an event arrives.
    assert_eq!(run_for(Duration::from_millis(10), dpm.get_event()).await, None);
    handle.push_event(Event::Disable);
    assert_eq!(handle.queued_events(), [Event::Disable]);
    assert_eq!(dpm.get_event().await, Event::Disable);

    // Woken by the push, but cancelled before it is polled again.
    let mut pending = Box::pin(dpm.get_event());
    assert_eq!(run_for(Duration::ZERO, &mut pending).await, None);
    handle.push_event(Event::Enable);
    drop(pending);
    assert_eq!(handle.queued_events(), [Event::Enable]);

    // A future that is never polled takes nothing.
    drop(dpm.get_event());
    assert_eq!(handle.queued_events(), [Event::Enable]);

    // As in a `select` with another branch that is ready.
    tokio::select! {
        biased;
        () = std::future::ready(()) => {}
        _ = dpm.get_event() => unreachable!(),
    }
    assert_eq!(handle.queued_events(), [Event::Enable]);

    assert_eq!(dpm.get_event().await, Event::Enable);
    assert_eq!(
        log.calls(),
        [
            DpmCall::EventReturned(Event::Disable),
            DpmCall::EventReturned(Event::Enable)
        ]
        .map(Call::from)
    );
}

#[tokio::test(start_paused = true)]
async fn informs_are_recorded_with_timestamps() {
    let log = Log::new();
    let (mut dpm, handle) = MockDpm::new(&log, "port");
    sleep(Duration::from_millis(15)).await;
    dpm.inform(Info::TypeCCurrent(RpLevel::Current1A5)).await;
    sleep(Duration::from_millis(10)).await;
    dpm.inform(Info::TypeCCurrent(RpLevel::Default)).await;

    let expected = [
        (Duration::from_millis(15), Info::TypeCCurrent(RpLevel::Current1A5)),
        (Duration::from_millis(25), Info::TypeCCurrent(RpLevel::Default)),
    ];
    assert_eq!(handle.informs(), expected);
    assert_eq!(
        log.entries()
            .iter()
            .map(|entry| (entry.at, entry.call))
            .collect::<Vec<_>>(),
        expected.map(|(at, info)| (at, DpmCall::Inform(info).into()))
    );

    // Kept when the log is cleared.
    log.clear();
    assert_eq!(handle.informs(), expected);
}

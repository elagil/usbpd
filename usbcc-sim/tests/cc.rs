//! Behavior of the CC mock.

use std::pin::pin;
use std::time::Duration;

use tokio::time::{Instant, sleep};
use usbcc_sim::{Call, CcCall, Log, MockCcPhy, SimError, run_for};
use usbcc_traits::{CcPhy, CcPin, CcState, RpLevel, Termination};

const RP: CcState = CcState::Rp(RpLevel::Default);
const RP_3A0: CcState = CcState::Rp(RpLevel::Current3A0);
const OPEN: (CcState, CcState) = (CcState::Open, CcState::Open);

fn mock() -> (Log, MockCcPhy, usbcc_sim::CcHandle) {
    let log = Log::new();
    let (cc, handle) = MockCcPhy::new(&log, "port");
    (log, cc, handle)
}

#[tokio::test(start_paused = true)]
async fn starts_open() {
    let (_log, mut cc, handle) = mock();
    assert_eq!(handle.cc(), OPEN);
    assert_eq!(handle.terminations(), (Termination::Open, Termination::Open));
    assert_eq!(cc.cc_state().await, Ok(OPEN));
}

#[tokio::test(start_paused = true)]
async fn cc_state_reports_stimulus() {
    let (_log, mut cc, handle) = mock();
    handle.set_cc(RP, CcState::Open);
    assert_eq!(cc.cc_state().await, Ok((RP, CcState::Open)));
    handle.set_pin(CcPin::Cc2, CcState::Ra);
    assert_eq!(cc.cc_state().await, Ok((RP, CcState::Ra)));
    assert_eq!(handle.cc(), (RP, CcState::Ra));
}

#[tokio::test(start_paused = true)]
async fn initial_state_counts_as_read() {
    let (_log, mut cc, _handle) = mock();
    assert_eq!(run_for(Duration::from_secs(10), cc.wait_for_cc_change()).await, None);
}

#[tokio::test(start_paused = true)]
async fn wait_returns_immediately_after_unread_change() {
    let (_log, mut cc, handle) = mock();
    // Changed before the first read.
    handle.set_cc(RP, CcState::Open);
    let start = Instant::now();
    assert_eq!(cc.wait_for_cc_change().await, Ok((RP, CcState::Open)));
    assert_eq!(start.elapsed(), Duration::ZERO);

    // Changed after a read by `wait_for_cc_change`.
    handle.set_cc(RP_3A0, CcState::Open);
    assert_eq!(
        run_for(Duration::ZERO, cc.wait_for_cc_change()).await,
        Some(Ok((RP_3A0, CcState::Open)))
    );

    // Changed after a read by `cc_state`.
    cc.cc_state().await.unwrap();
    handle.set_cc(CcState::Open, CcState::Open);
    assert_eq!(run_for(Duration::ZERO, cc.wait_for_cc_change()).await, Some(Ok(OPEN)));
}

#[tokio::test(start_paused = true)]
async fn wait_marks_state_as_seen() {
    let (_log, mut cc, handle) = mock();
    handle.set_cc(RP, CcState::Open);
    cc.wait_for_cc_change().await.unwrap();
    assert_eq!(run_for(Duration::from_secs(1), cc.wait_for_cc_change()).await, None);
}

#[tokio::test(start_paused = true)]
async fn cc_state_marks_state_as_seen() {
    let (_log, mut cc, handle) = mock();
    handle.set_cc(RP, CcState::Open);
    cc.cc_state().await.unwrap();
    assert_eq!(run_for(Duration::from_secs(1), cc.wait_for_cc_change()).await, None);
}

#[tokio::test(start_paused = true)]
async fn wait_wakes_on_change() {
    let (_log, mut cc, handle) = mock();
    let start = Instant::now();
    let (result, ()) = tokio::join!(cc.wait_for_cc_change(), async {
        sleep(Duration::from_millis(100)).await;
        handle.set_cc(CcState::Open, RP);
    });
    assert_eq!(result, Ok((CcState::Open, RP)));
    assert_eq!(start.elapsed(), Duration::from_millis(100));
}

#[tokio::test(start_paused = true)]
async fn pending_wait_wakes_on_change() {
    let (_log, mut cc, handle) = mock();
    let mut wait = pin!(cc.wait_for_cc_change());
    assert_eq!(run_for(Duration::from_millis(100), &mut wait).await, None);
    handle.set_cc(RP, CcState::Open);
    assert_eq!(run_for(Duration::ZERO, &mut wait).await, Some(Ok((RP, CcState::Open))));
}

#[tokio::test(start_paused = true)]
async fn setting_the_same_state_is_no_change() {
    let (_log, mut cc, handle) = mock();
    handle.set_cc(CcState::Open, CcState::Open);
    handle.set_pin(CcPin::Cc1, CcState::Open);
    assert_eq!(run_for(Duration::from_secs(1), cc.wait_for_cc_change()).await, None);
}

#[tokio::test(start_paused = true)]
async fn reverted_change_is_reported() {
    let (_log, mut cc, handle) = mock();
    handle.set_cc(RP, CcState::Open);
    handle.set_cc(CcState::Open, CcState::Open);
    // Like an edge-triggered interrupt: the change happened, even though the state is back.
    assert_eq!(run_for(Duration::ZERO, cc.wait_for_cc_change()).await, Some(Ok(OPEN)));
}

#[tokio::test(start_paused = true)]
async fn spurious_change_on_request() {
    let (_log, mut cc, handle) = mock();
    handle.trigger_spurious_change();
    assert_eq!(run_for(Duration::ZERO, cc.wait_for_cc_change()).await, Some(Ok(OPEN)));
    // Only once.
    assert_eq!(run_for(Duration::from_secs(1), cc.wait_for_cc_change()).await, None);

    // Also wakes a pending wait.
    let mut wait = pin!(cc.wait_for_cc_change());
    assert_eq!(run_for(Duration::from_millis(10), &mut wait).await, None);
    handle.trigger_spurious_change();
    assert_eq!(run_for(Duration::ZERO, &mut wait).await, Some(Ok(OPEN)));
}

#[tokio::test(start_paused = true)]
async fn terminations_are_visible_and_watchable() {
    let (_log, mut cc, handle) = mock();
    let mut watcher = handle.watch_terminations();
    assert_eq!(run_for(Duration::from_secs(1), watcher.changed()).await, None);

    let rp = Termination::Rp(RpLevel::Current1A5);
    cc.set_termination(rp, Termination::Rd).await.unwrap();
    assert_eq!(handle.terminations(), (rp, Termination::Rd));
    assert_eq!(watcher.changed().await, (rp, Termination::Rd));

    // Several calls in between are reported once, with the latest terminations.
    cc.set_termination(Termination::Rd, Termination::Rd).await.unwrap();
    cc.set_termination(Termination::Open, Termination::Open).await.unwrap();
    assert_eq!(watcher.changed().await, (Termination::Open, Termination::Open));
    assert_eq!(run_for(Duration::from_secs(1), watcher.changed()).await, None);

    // Setting terminations doesn't wake `wait_for_cc_change`: only the stimulus does.
    assert_eq!(run_for(Duration::from_secs(1), cc.wait_for_cc_change()).await, None);
}

#[tokio::test(start_paused = true)]
async fn watcher_wakes_on_set_termination() {
    let (_log, mut cc, handle) = mock();
    let mut watcher = handle.watch_terminations();
    let start = Instant::now();
    let (terminations, ()) = tokio::join!(watcher.changed(), async {
        sleep(Duration::from_millis(30)).await;
        cc.set_termination(Termination::Rd, Termination::Open).await.unwrap();
    });
    assert_eq!(terminations, (Termination::Rd, Termination::Open));
    assert_eq!(start.elapsed(), Duration::from_millis(30));
}

#[tokio::test(start_paused = true)]
async fn injected_errors() {
    let (log, mut cc, handle) = mock();
    let error = SimError("cc");

    // `set_termination` fails without effect.
    handle.fail_next(error);
    assert_eq!(cc.set_termination(Termination::Rd, Termination::Rd).await, Err(error));
    assert_eq!(handle.terminations(), (Termination::Open, Termination::Open));

    // `cc_state` fails without marking the state as seen.
    handle.set_cc(RP, CcState::Open);
    handle.fail_next(error);
    assert_eq!(cc.cc_state().await, Err(error));
    assert_eq!(
        run_for(Duration::ZERO, cc.wait_for_cc_change()).await,
        Some(Ok((RP, CcState::Open)))
    );

    // `wait_for_cc_change` fails immediately, although nothing changed.
    handle.fail_next(error);
    assert_eq!(run_for(Duration::ZERO, cc.wait_for_cc_change()).await, Some(Err(error)));

    // Only the next call fails.
    assert_eq!(cc.cc_state().await, Ok((RP, CcState::Open)));

    // Failed calls are logged.
    assert_eq!(
        log.calls(),
        [
            CcCall::SetTermination {
                cc1: Termination::Rd,
                cc2: Termination::Rd
            },
            CcCall::CcState,
            CcCall::WaitForCcChange,
            CcCall::WaitForCcChange,
            CcCall::CcState,
        ]
        .map(Call::from)
    );
}

/// Exercises the mock through the trait, as a generic port does.
async fn through_trait(mut cc: impl CcPhy<Error = SimError>) -> Result<(CcState, CcState), SimError> {
    cc.set_termination(Termination::Rd, Termination::Rd).await?;
    cc.cc_state().await?;
    cc.wait_for_cc_change().await
}

#[tokio::test(start_paused = true)]
async fn works_through_mut_ref() {
    let (log, mut cc, handle) = mock();
    let (result, ()) = tokio::join!(through_trait(&mut cc), async {
        sleep(Duration::from_millis(5)).await;
        handle.set_cc(RP, CcState::Open);
    });
    assert_eq!(result, Ok((RP, CcState::Open)));
    assert_eq!(handle.terminations(), (Termination::Rd, Termination::Rd));
    assert_eq!(log.len(), 3);

    // The seen state lives in the mock, not in the reference.
    assert_eq!(run_for(Duration::from_secs(1), cc.wait_for_cc_change()).await, None);
}

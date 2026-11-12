//! Behavior of the power path mock.

use std::pin::pin;
use std::time::Duration;

use tokio::time::{Instant, sleep};
use usbcc_sim::{Call, Log, MockPowerPath, PowerCall, PowerPathHandle, PowerPathState, SimError, run_for};
use usbcc_traits::{CcPin, PowerPath};

fn mock() -> (Log, MockPowerPath, PowerPathHandle) {
    let log = Log::new();
    let (power, handle) = MockPowerPath::new(&log, "port");
    (log, power, handle)
}

#[tokio::test(start_paused = true)]
async fn switches_follow_calls() {
    let (log, mut power, handle) = mock();
    assert_eq!(handle.state(), PowerPathState::default());

    power.set_active(true).await.unwrap();
    power.set_vbus_sink(true).await.unwrap();
    power.set_vbus_source(true).await.unwrap();
    power.set_vbus_discharge(true).await.unwrap();
    power.set_vconn(Some(CcPin::Cc2)).await.unwrap();
    power.set_vconn_discharge(Some(CcPin::Cc1)).await.unwrap();
    assert_eq!(
        handle.state(),
        PowerPathState {
            active: true,
            vbus_sink: true,
            vbus_source: true,
            vbus_discharge: true,
            vconn: Some(CcPin::Cc2),
            vconn_discharge: Some(CcPin::Cc1),
        }
    );

    power.set_vbus_sink(false).await.unwrap();
    power.set_vconn(None).await.unwrap();
    let state = handle.state();
    assert!(!state.vbus_sink);
    assert_eq!(state.vconn, None);

    assert_eq!(
        log.calls(),
        [
            PowerCall::SetActive(true),
            PowerCall::SetVbusSink(true),
            PowerCall::SetVbusSource(true),
            PowerCall::SetVbusDischarge(true),
            PowerCall::SetVconn(Some(CcPin::Cc2)),
            PowerCall::SetVconnDischarge(Some(CcPin::Cc1)),
            PowerCall::SetVbusSink(false),
            PowerCall::SetVconn(None),
        ]
        .map(Call::from)
    );
}

#[tokio::test(start_paused = true)]
async fn ready_flags_complete_immediately_by_default() {
    let (_log, mut power, _handle) = mock();
    assert_eq!(run_for(Duration::ZERO, power.prepare_vbus_source()).await, Some(Ok(())));
    assert_eq!(
        run_for(Duration::ZERO, power.wait_vconn_discharged()).await,
        Some(Ok(()))
    );
}

#[tokio::test(start_paused = true)]
async fn prepare_vbus_source_waits_for_ready() {
    let (_log, mut power, handle) = mock();
    handle.set_source_ready(false);
    let start = Instant::now();
    let (result, ()) = tokio::join!(power.prepare_vbus_source(), async {
        sleep(Duration::from_millis(40)).await;
        assert_eq!(handle.in_progress(), Some("prepare_vbus_source"));
        handle.set_source_ready(true);
    });
    assert_eq!(result, Ok(()));
    assert_eq!(start.elapsed(), Duration::from_millis(40));
    assert_eq!(handle.in_progress(), None);
}

#[tokio::test(start_paused = true)]
async fn wait_vconn_discharged_waits_for_flag() {
    let (_log, mut power, handle) = mock();
    handle.set_vconn_discharged(false);
    let mut wait = pin!(power.wait_vconn_discharged());
    assert_eq!(run_for(Duration::from_secs(1), &mut wait).await, None);
    handle.set_vconn_discharged(true);
    assert_eq!(run_for(Duration::ZERO, &mut wait).await, Some(Ok(())));
}

#[tokio::test(start_paused = true)]
async fn dropping_a_pending_call_ends_it() {
    let (_log, mut power, handle) = mock();
    handle.set_source_ready(false);
    // Abandoned attach: the future is dropped while pending.
    assert_eq!(
        run_for(Duration::from_millis(100), power.prepare_vbus_source()).await,
        None
    );
    assert_eq!(handle.in_progress(), None);
    power.set_vbus_source(false).await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn unpolled_call_is_in_progress() {
    let (log, mut power, handle) = mock();
    let call = power.set_vbus_sink(true);
    assert_eq!(handle.in_progress(), Some("set_vbus_sink"));
    drop(call);
    assert_eq!(handle.in_progress(), None);
    // Dropped before the first poll: it was called, but had no effect.
    assert!(!handle.state().vbus_sink);
    assert_eq!(log.calls(), [Call::from(PowerCall::SetVbusSink(true))]);
}

#[tokio::test(start_paused = true)]
#[should_panic(expected = "`PowerPath::set_active` called while `PowerPath::prepare_vbus_source` is in progress")]
async fn concurrent_calls_panic() {
    let (_log, mut power, handle) = mock();
    handle.set_source_ready(false);
    let mut prepare = Box::pin(power.prepare_vbus_source());
    assert_eq!(run_for(Duration::from_millis(10), &mut prepare).await, None);
    // Leak the pending call: the borrow ends, but the call is still in progress.
    std::mem::forget(prepare);
    let _ = power.set_active(true).await;
}

#[tokio::test(start_paused = true)]
async fn injected_errors() {
    let (log, mut power, handle) = mock();
    let error = SimError("power");

    handle.fail_next(error);
    assert_eq!(power.set_vbus_source(true).await, Err(error));
    assert!(!handle.state().vbus_source);
    assert_eq!(handle.in_progress(), None);

    // A wait fails immediately, although it isn't ready.
    handle.set_source_ready(false);
    handle.fail_next(error);
    assert_eq!(
        run_for(Duration::ZERO, power.prepare_vbus_source()).await,
        Some(Err(error))
    );

    // Every method, including those with a default implementation in the trait, can fail.
    handle.fail_next(error);
    assert_eq!(power.set_active(true).await, Err(error));
    handle.fail_next(error);
    assert_eq!(power.set_vconn(Some(CcPin::Cc1)).await, Err(error));
    handle.fail_next(error);
    assert_eq!(power.set_vconn_discharge(Some(CcPin::Cc1)).await, Err(error));
    handle.fail_next(error);
    assert_eq!(power.wait_vconn_discharged().await, Err(error));
    assert_eq!(handle.state(), PowerPathState::default());

    // Only the next call fails.
    assert_eq!(power.set_vbus_sink(true).await, Ok(()));
    assert_eq!(log.len(), 7);
}

async fn through_trait(mut power: impl PowerPath<Error = SimError>) -> Result<(), SimError> {
    power.set_active(true).await?;
    power.prepare_vbus_source().await?;
    power.set_vbus_source(true).await?;
    power.set_vconn(Some(CcPin::Cc1)).await?;
    power.set_vconn(None).await?;
    power.set_vconn_discharge(Some(CcPin::Cc1)).await?;
    power.wait_vconn_discharged().await
}

#[tokio::test(start_paused = true)]
async fn works_through_mut_ref() {
    let (log, mut power, handle) = mock();
    through_trait(&mut power).await.unwrap();
    assert_eq!(
        handle.state(),
        PowerPathState {
            active: true,
            vbus_source: true,
            vconn_discharge: Some(CcPin::Cc1),
            ..Default::default()
        }
    );
    assert_eq!(log.len(), 7);
}

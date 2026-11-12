//! Behavior of the VBUS mock.

use std::pin::pin;
use std::time::Duration;

use tokio::time::{Instant, sleep};
use usbcc_sim::{Call, Log, MockVbusSense, SimError, VbusCall, VbusHandle, run_for};
use usbcc_traits::VbusSense;

fn mock() -> (Log, MockVbusSense, VbusHandle) {
    let log = Log::new();
    let (vbus, handle) = MockVbusSense::new(&log, "port");
    (log, vbus, handle)
}

const SECOND: Duration = Duration::from_secs(1);

#[tokio::test(start_paused = true)]
async fn reports_voltage() {
    let (_log, mut vbus, handle) = mock();
    assert_eq!(vbus.vbus_mv().await, Ok(0));
    handle.set_mv(5000);
    assert_eq!(vbus.vbus_mv().await, Ok(5000));
    assert_eq!(handle.mv(), 5000);
}

#[tokio::test(start_paused = true)]
async fn outside_returns_immediately_if_already_outside() {
    let (_log, mut vbus, handle) = mock();
    handle.set_mv(100);
    let start = Instant::now();
    vbus.wait_vbus_outside(800, 5500).await.unwrap();
    handle.set_mv(6000);
    vbus.wait_vbus_outside(800, 5500).await.unwrap();
    assert_eq!(start.elapsed(), Duration::ZERO);
}

#[tokio::test(start_paused = true)]
async fn bounds_are_strict() {
    let (_log, mut vbus, handle) = mock();
    for mv in [800, 3000, 5500] {
        handle.set_mv(mv);
        assert_eq!(run_for(SECOND, vbus.wait_vbus_outside(800, 5500)).await, None);
    }
    handle.set_mv(799);
    assert_eq!(
        run_for(Duration::ZERO, vbus.wait_vbus_outside(800, 5500)).await,
        Some(Ok(()))
    );
    handle.set_mv(5501);
    assert_eq!(
        run_for(Duration::ZERO, vbus.wait_vbus_outside(800, 5500)).await,
        Some(Ok(()))
    );
}

#[tokio::test(start_paused = true)]
async fn outside_waits_until_outside() {
    let (_log, mut vbus, handle) = mock();
    handle.set_mv(5000);
    let start = Instant::now();
    let (result, ()) = tokio::join!(vbus.wait_vbus_outside(3500, 5500), async {
        sleep(Duration::from_millis(10)).await;
        // Still inside: no return.
        handle.set_mv(4000);
        sleep(Duration::from_millis(10)).await;
        handle.set_mv(3000);
    });
    assert_eq!(result, Ok(()));
    assert_eq!(start.elapsed(), Duration::from_millis(20));
}

#[tokio::test(start_paused = true)]
async fn waits_are_level_based() {
    let (_log, mut vbus, handle) = mock();
    handle.set_mv(5000);
    let mut wait = pin!(vbus.wait_vbus_below(3500));
    assert_eq!(run_for(SECOND, &mut wait).await, None);
    // An excursion reverted before the wait is polled again isn't seen.
    handle.set_mv(0);
    handle.set_mv(5000);
    assert_eq!(run_for(SECOND, &mut wait).await, None);
    handle.set_mv(0);
    assert_eq!(run_for(Duration::ZERO, &mut wait).await, Some(Ok(())));
}

#[tokio::test(start_paused = true)]
async fn below_and_above() {
    let (log, mut vbus, handle) = mock();
    handle.set_mv(3500);
    assert_eq!(run_for(SECOND, vbus.wait_vbus_below(3500)).await, None);
    assert_eq!(run_for(SECOND, vbus.wait_vbus_above(3500)).await, None);
    assert_eq!(run_for(Duration::ZERO, vbus.wait_vbus_below(3501)).await, Some(Ok(())));
    assert_eq!(run_for(Duration::ZERO, vbus.wait_vbus_above(3499)).await, Some(Ok(())));

    // Extreme thresholds: never below 0, never above u32::MAX.
    handle.set_mv(0);
    assert_eq!(run_for(SECOND, vbus.wait_vbus_below(0)).await, None);
    handle.set_mv(u32::MAX);
    assert_eq!(run_for(SECOND, vbus.wait_vbus_above(u32::MAX)).await, None);

    // Recorded as the methods that were called.
    assert_eq!(
        log.calls()[..4],
        [
            VbusCall::WaitVbusBelow { mv: 3500 },
            VbusCall::WaitVbusAbove { mv: 3500 },
            VbusCall::WaitVbusBelow { mv: 3501 },
            VbusCall::WaitVbusAbove { mv: 3499 },
        ]
        .map(Call::from)
    );
}

#[tokio::test(start_paused = true)]
async fn below_wakes_on_change() {
    let (_log, mut vbus, handle) = mock();
    handle.set_mv(5000);
    let start = Instant::now();
    let (result, ()) = tokio::join!(vbus.wait_vbus_below(800), async {
        sleep(Duration::from_millis(275)).await;
        handle.set_mv(700);
    });
    assert_eq!(result, Ok(()));
    assert_eq!(start.elapsed(), Duration::from_millis(275));
}

#[tokio::test(start_paused = true)]
async fn injected_errors() {
    let (log, mut vbus, handle) = mock();
    let error = SimError("vbus");
    handle.set_mv(5000);

    handle.fail_next(error);
    assert_eq!(vbus.vbus_mv().await, Err(error));
    assert_eq!(vbus.vbus_mv().await, Ok(5000));

    // A wait fails immediately, although VBUS is inside.
    handle.fail_next(error);
    assert_eq!(
        run_for(Duration::ZERO, vbus.wait_vbus_outside(0, 6000)).await,
        Some(Err(error))
    );
    handle.fail_next(error);
    assert_eq!(
        run_for(Duration::ZERO, vbus.wait_vbus_below(800)).await,
        Some(Err(error))
    );
    handle.fail_next(error);
    assert_eq!(
        run_for(Duration::ZERO, vbus.wait_vbus_above(6000)).await,
        Some(Err(error))
    );
    assert_eq!(run_for(SECOND, vbus.wait_vbus_above(6000)).await, None);

    assert_eq!(log.len(), 6);
}

async fn through_trait(mut vbus: impl VbusSense<Error = SimError>) -> Result<u32, SimError> {
    vbus.wait_vbus_above(4000).await?;
    vbus.wait_vbus_outside(4000, 6000).await?;
    vbus.wait_vbus_below(800).await?;
    vbus.vbus_mv().await
}

#[tokio::test(start_paused = true)]
async fn works_through_mut_ref() {
    let (log, mut vbus, handle) = mock();
    let (result, ()) = tokio::join!(through_trait(&mut vbus), async {
        sleep(Duration::from_millis(5)).await;
        handle.set_mv(5000);
        sleep(Duration::from_millis(5)).await;
        handle.set_mv(600);
    });
    assert_eq!(result, Ok(600));
    assert_eq!(
        log.calls(),
        [
            VbusCall::WaitVbusAbove { mv: 4000 },
            VbusCall::WaitVbusOutside {
                low_mv: 4000,
                high_mv: 6000
            },
            VbusCall::WaitVbusBelow { mv: 800 },
            VbusCall::VbusMv,
        ]
        .map(Call::from)
    );
}

//! The run-for-a-duration helper and the timer.

use std::future::pending;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tokio::time::{Instant, sleep};
use usbcc::Timer;
use usbcc_sim::{SimTimer, run_for};

/// Sets a flag when dropped.
struct DropFlag(Arc<AtomicBool>);

impl Drop for DropFlag {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[tokio::test(start_paused = true)]
async fn completed_future_returns_output_early() {
    let start = Instant::now();
    let output = run_for(Duration::from_millis(100), async {
        sleep(Duration::from_millis(30)).await;
        42
    })
    .await;
    assert_eq!(output, Some(42));
    assert_eq!(start.elapsed(), Duration::from_millis(30));
}

#[tokio::test(start_paused = true)]
async fn unfinished_future_runs_for_the_duration_and_is_dropped() {
    let dropped = Arc::new(AtomicBool::new(false));
    let flag = DropFlag(dropped.clone());
    let polled = Arc::new(AtomicBool::new(false));
    let polled_in_future = polled.clone();

    let start = Instant::now();
    let output = run_for(Duration::from_millis(250), async move {
        let _flag = flag;
        polled_in_future.store(true, Ordering::SeqCst);
        pending::<()>().await;
    })
    .await;
    assert_eq!(output, None);
    assert_eq!(start.elapsed(), Duration::from_millis(250));
    assert!(polled.load(Ordering::SeqCst));
    assert!(dropped.load(Ordering::SeqCst));
}

#[tokio::test(start_paused = true)]
async fn completion_at_the_deadline_counts() {
    let output = run_for(Duration::from_millis(100), sleep(Duration::from_millis(100))).await;
    assert_eq!(output, Some(()));
}

#[tokio::test(start_paused = true)]
async fn zero_duration_polls_once() {
    let start = Instant::now();
    assert_eq!(run_for(Duration::ZERO, async { 1 }).await, Some(1));
    assert_eq!(run_for(Duration::ZERO, sleep(Duration::from_millis(1))).await, None);
    assert_eq!(start.elapsed(), Duration::ZERO);
}

#[tokio::test(start_paused = true)]
async fn pinned_future_survives_across_runs() {
    let mut sleep = Box::pin(sleep(Duration::from_millis(150)));
    assert_eq!(run_for(Duration::from_millis(100), &mut sleep).await, None);
    assert_eq!(run_for(Duration::from_millis(100), &mut sleep).await, Some(()));
}

#[tokio::test(start_paused = true)]
async fn timer_sleeps_in_virtual_time() {
    let start = Instant::now();
    SimTimer::after_millis(1234).await;
    assert_eq!(start.elapsed(), Duration::from_millis(1234));
}

//! Mock of the device policy manager.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;
use usbcc::{DevicePolicyManager, Event, Info};

use crate::log::{DpmCall, Log};

#[derive(Debug, Default)]
struct DpmInner {
    events: VecDeque<Event>,
    informs: Vec<(Duration, Info)>,
}

/// The [`DevicePolicyManager`] side of the DPM mock, handed to the port.
///
/// [`DevicePolicyManager::inform`] completes immediately. [`DevicePolicyManager::get_event`]
/// returns the oldest event queued through the [`DpmHandle`], and stays pending while there is
/// none. It is cancel-safe: an event is only taken off the queue by the poll that returns it.
#[derive(Debug)]
pub struct MockDpm {
    shared: Arc<watch::Sender<DpmInner>>,
    log: Log,
    port: &'static str,
}

/// The test side of the DPM mock. Cloning yields another handle to the same mock.
#[derive(Debug, Clone)]
pub struct DpmHandle {
    shared: Arc<watch::Sender<DpmInner>>,
}

impl MockDpm {
    /// Creates a DPM mock that records its calls in `log`, labelled with `port`.
    pub fn new(log: &Log, port: &'static str) -> (Self, DpmHandle) {
        let shared = Arc::new(watch::Sender::new(DpmInner::default()));
        let mock = Self {
            shared: shared.clone(),
            log: log.clone(),
            port,
        };
        (mock, DpmHandle { shared })
    }
}

impl DevicePolicyManager for MockDpm {
    async fn inform(&mut self, info: Info) {
        self.log.record(self.port, DpmCall::Inform(info));
        let at = self.log.now();
        self.shared.send_modify(|inner| inner.informs.push((at, info)));
    }

    async fn get_event(&mut self) -> Event {
        self.shared
            .subscribe()
            .wait_for(|inner| !inner.events.is_empty())
            .await
            .expect("the mock holds the sender");
        // Taken in the same poll that found it, so a cancelled call never loses an event.
        let mut event = None;
        self.shared.send_modify(|inner| event = inner.events.pop_front());
        let event = event.expect("checked above");
        self.log.record(self.port, DpmCall::EventReturned(event));
        event
    }
}

impl DpmHandle {
    /// Queues `event` for [`DevicePolicyManager::get_event`], and wakes a pending call.
    pub fn push_event(&self, event: Event) {
        self.shared.send_modify(|inner| inner.events.push_back(event));
    }

    /// The events queued and not yet returned, oldest first.
    pub fn queued_events(&self) -> Vec<Event> {
        self.shared.borrow().events.iter().copied().collect()
    }

    /// Every [`DevicePolicyManager::inform`] call of this mock so far, with its timestamp on the
    /// scale of [`Entry::at`](crate::Entry::at), oldest first.
    ///
    /// The calls are in the [`Log`] as well; this record is kept even if the log is cleared.
    pub fn informs(&self) -> Vec<(Duration, Info)> {
        self.shared.borrow().informs.clone()
    }
}

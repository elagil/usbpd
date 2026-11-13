//! Mock of the device policy manager.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::watch;
use usbcc::{DevicePolicyManager, Event, Info};

use crate::log::{DpmCall, Log};

/// The [`DevicePolicyManager`] side of the DPM mock, handed to the port.
///
/// [`DevicePolicyManager::inform`] completes immediately. [`DevicePolicyManager::get_event`]
/// returns the oldest event queued through the [`DpmHandle`], and stays pending while there is
/// none. It is cancel-safe: an event is only taken off the queue by the poll that returns it.
#[derive(Debug)]
pub struct MockDpm {
    events: watch::Sender<VecDeque<Event>>,
    informs: Arc<Mutex<Vec<(Duration, Info)>>>,
    log: Log,
}

/// The test side of the DPM mock. Cloning yields another handle to the same mock.
#[derive(Debug, Clone)]
pub struct DpmHandle {
    events: watch::Sender<VecDeque<Event>>,
    informs: Arc<Mutex<Vec<(Duration, Info)>>>,
}

impl MockDpm {
    /// Creates a DPM mock that records its calls in `log`.
    pub fn new(log: &Log) -> (Self, DpmHandle) {
        let handle = DpmHandle {
            events: watch::Sender::default(),
            informs: Arc::default(),
        };
        let mock = Self {
            events: handle.events.clone(),
            informs: handle.informs.clone(),
            log: log.clone(),
        };
        (mock, handle)
    }
}

impl DevicePolicyManager for MockDpm {
    async fn inform(&mut self, info: Info) {
        self.log.record(DpmCall::Inform(info));
        let at = self.log.now();
        self.informs.lock().unwrap().push((at, info));
    }

    async fn get_event(&mut self) -> Event {
        self.events
            .subscribe()
            .wait_for(|events| !events.is_empty())
            .await
            .expect("the mock holds the sender");
        // Taken in the same poll that found it, so a cancelled call never loses an event.
        let mut event = None;
        self.events.send_modify(|events| event = events.pop_front());
        let event = event.expect("checked above");
        self.log.record(DpmCall::EventReturned(event));
        event
    }
}

impl DpmHandle {
    /// Queues `event` for [`DevicePolicyManager::get_event`], and wakes a pending call.
    pub fn push_event(&self, event: Event) {
        self.events.send_modify(|events| events.push_back(event));
    }

    /// The events queued and not yet returned, oldest first.
    pub fn queued_events(&self) -> Vec<Event> {
        self.events.borrow().iter().copied().collect()
    }

    /// Every [`DevicePolicyManager::inform`] call of this mock so far, with its timestamp on the
    /// scale of [`Entry::at`](crate::Entry::at), oldest first.
    ///
    /// The calls are in the [`Log`] as well; this record is kept even if the log is cleared.
    pub fn informs(&self) -> Vec<(Duration, Info)> {
        self.informs.lock().unwrap().clone()
    }
}

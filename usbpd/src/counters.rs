//! Definition of counters, used for retry attempts, and message IDs.
use defmt::Format;

/// Counter error variants.
#[non_exhaustive]
#[derive(Debug, Format)]
pub enum Error {
    /// The counter wrapped around its maximum allowed value and was reset.
    Exceeded,
}

/// A counter structure, used for detecting overruns (e.g. retries).
#[derive(Debug, Clone, Copy)]
pub struct Counter {
    value: u8,
    max_value: u8,
}

/// The type of counter that can be created.
#[derive(Debug, Clone, Copy)]
#[allow(missing_docs)]
pub enum CounterType {
    Busy,
    Caps,
    DiscoverIdentity,
    HardReset,
    MessageId,
    Retry,
}

impl Counter {
    /// Create a new counter of a provided type.
    pub fn new(counter_type: CounterType) -> Self {
        // See spec, [Table 6.70]
        let max_value = match counter_type {
            CounterType::Busy => 5,
            CounterType::Caps => 50,
            CounterType::DiscoverIdentity => 20,
            CounterType::HardReset => 2,
            CounterType::MessageId => 7,
            CounterType::Retry => 2,
        };

        Self { value: 0, max_value }
    }

    /// Create a new counter of a provided type from an initial value.
    pub fn new_from_value(counter_type: CounterType, value: u8) -> Self {
        let mut counter = Self::new(counter_type);
        counter.set(value);
        counter
    }

    /// Set a new counter value, clamped to the maximum counter value.
    pub fn set(&mut self, value: u8) {
        self.value = value % (self.max_value + 1);
    }

    /// The counter value.
    pub fn value(&self) -> u8 {
        self.value
    }

    /// Increment a counter.
    ///
    /// If it wraps, this returns an error.
    pub fn increment(&mut self) -> Result<(), Error> {
        self.set(self.value + 1);

        if self.value == 0 {
            Err(Error::Exceeded)
        } else {
            Ok(())
        }
    }

    /// Reset a counter value to zero.
    pub fn reset(&mut self) {
        self.value = 0;
    }
}

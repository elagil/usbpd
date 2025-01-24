//! Definition of counters, used for retry attempts, and message IDs.
use defmt::Format;

#[non_exhaustive]
#[derive(Debug, Format)]
pub enum Error {
    Overrun,
}

#[derive(Debug, Clone, Copy)]
pub struct Counter {
    value: u8,
    max_value: u8,
}

#[derive(Debug, Clone, Copy)]
pub enum CounterType {
    Busy,
    Caps,
    DiscoverIdentity,
    HardReset,
    MessageId,
    Retry,
}

impl Counter {
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

    pub fn new_from_value(counter_type: CounterType, value: u8) -> Self {
        let mut counter = Self::new(counter_type);
        counter.set(value);
        counter
    }

    pub fn set(&mut self, value: u8) {
        self.value = value % (self.max_value + 1);
    }

    pub fn value(&self) -> u8 {
        self.value
    }

    pub fn increment(&mut self) -> Result<(), Error> {
        self.set(self.value + 1);

        if self.value == 0 {
            Err(Error::Overrun)
        } else {
            Ok(())
        }
    }

    pub fn reset(&mut self) {
        self.value = 0;
    }
}

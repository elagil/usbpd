//! Definition of counters, used for retry attempts, and message IDs.
use defmt::Format;

#[non_exhaustive]
#[derive(Debug, Format)]
pub struct CounterError {
    pub kind: CounterErrorKind,
}

#[non_exhaustive]
#[derive(Debug, Format)]
pub enum CounterErrorKind {
    ExceedsMaximumValue,
    Overrun,
}

#[derive(Debug, Clone, Copy)]
pub struct Counter {
    value: u8,
    max_value: u8,
    counter_type: CounterType,
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
        // R3.2, V1.1: Table 6.70
        let max_value = match counter_type {
            CounterType::Busy => 5,
            CounterType::Caps => 50,
            CounterType::DiscoverIdentity => 20,
            CounterType::HardReset => 2,
            CounterType::MessageId => 7,
            CounterType::Retry => 2,
        };

        Self {
            value: 0,
            max_value,
            counter_type,
        }
    }

    pub fn set(&mut self, value: u8) -> Result<(), CounterError> {
        if value >= self.max_value {
            return Err(CounterError {
                kind: CounterErrorKind::ExceedsMaximumValue,
            });
        }

        self.value = value;
        Ok(())
    }

    pub fn value(&self) -> u8 {
        self.value
    }

    pub fn increment(&mut self) -> Result<(), CounterError> {
        self.value = (self.value + 1) % (self.max_value + 1);

        if self.value == 0 {
            Err(CounterError {
                kind: CounterErrorKind::Overrun,
            })
        } else {
            Ok(())
        }
    }

    pub fn reset(&mut self) {
        self.value = 0;
    }
}

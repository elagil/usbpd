use embassy_futures::select::{Either, select};
use embassy_stm32::gpio::Output;
use embassy_time::{Duration, Timer};

use super::dual_role;
use super::dual_role::Connection;

const BLINK_PERIOD: Duration = Duration::from_millis(500);
const NEVER: Duration = Duration::MAX;

#[embassy_executor::task]
pub async fn run(mut led: Output<'static>) {
    let mut connection = dual_role::CONNECTION_STATUS.receiver().unwrap();
    let mut led_update_dur = BLINK_PERIOD;

    loop {
        match select(connection.changed(), Timer::after(led_update_dur)).await {
            Either::First(Connection::Disconnected) => {
                led_update_dur = BLINK_PERIOD;
                led.toggle();
            }
            Either::First(Connection::Sink) => {
                led_update_dur = NEVER;
                led.set_low();
            }
            Either::First(Connection::Source) => {
                led_update_dur = NEVER;
                led.set_high();
            }
            Either::Second(_) => {
                led.toggle();
            }
        }
    }
}

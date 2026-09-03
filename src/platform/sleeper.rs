use std::time::{Duration, Instant};

use super::{Sleeper, WaitResult};
use crate::model::PlayerControl;

/// Real clock: waits on the control condvar so stop requests wake it, spins the last stretch for precision.
#[derive(Default)]
pub struct RealSleeper {
    spin: spin_sleep::SpinSleeper,
}

const SPIN_THRESHOLD: Duration = Duration::from_micros(1500);

impl Sleeper for RealSleeper {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn sleep_until(&self, deadline: Instant, ctl: &PlayerControl) -> WaitResult {
        loop {
            if ctl.is_stopped() {
                return WaitResult::Stopped;
            }
            let now = Instant::now();
            if now >= deadline {
                return WaitResult::Elapsed;
            }
            let remaining = deadline - now;
            if remaining <= SPIN_THRESHOLD {
                self.spin.sleep(remaining);
                return if ctl.is_stopped() { WaitResult::Stopped } else { WaitResult::Elapsed };
            }
            let guard = ctl.wake.0.lock().unwrap_or_else(|e| e.into_inner());
            let _ = ctl.wake.1.wait_timeout(guard, remaining - SPIN_THRESHOLD);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elapses_close_to_deadline() {
        let s = RealSleeper::default();
        let ctl = PlayerControl::new();
        let start = Instant::now();
        let deadline = start + Duration::from_millis(20);
        assert_eq!(s.sleep_until(deadline, &ctl), WaitResult::Elapsed);
        let elapsed = start.elapsed();
        assert!(elapsed >= Duration::from_millis(20), "{elapsed:?}");
        assert!(elapsed < Duration::from_millis(60), "{elapsed:?}");
    }

    #[test]
    fn stop_wakes_a_long_wait() {
        let s = std::sync::Arc::new(RealSleeper::default());
        let ctl = PlayerControl::new();
        let ctl2 = ctl.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            ctl2.request_stop();
        });
        let start = Instant::now();
        let result = s.sleep_until(start + Duration::from_secs(10), &ctl);
        assert_eq!(result, WaitResult::Stopped);
        assert!(start.elapsed() < Duration::from_secs(2));
    }
}

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::model::PlayerControl;
use crate::platform::{Sleeper, WaitResult};

/// Playback clock that turns relative delays into absolute deadlines, so a long
/// sequence of short waits does not accumulate drift.
pub struct Scheduler {
    sleeper: Arc<dyn Sleeper>,
    base: Instant,
    offset_ms: f64,
    speed_factor: f64,
}

impl Scheduler {
    /// `speed_factor` multiplies every duration; 0.5 replays twice as fast.
    pub fn new(sleeper: Arc<dyn Sleeper>, speed_factor: f64) -> Self {
        let base = sleeper.now();
        let speed_factor = if speed_factor.is_finite() { speed_factor.max(0.0) } else { 1.0 };
        Self { sleeper, base, offset_ms: 0.0, speed_factor }
    }

    pub fn speed_factor(&self) -> f64 {
        self.speed_factor
    }

    /// Waits out `ms` scaled by the speed factor, measured from the scheduler base.
    pub fn wait(&mut self, ms: f64, ctl: &PlayerControl) -> WaitResult {
        self.offset_ms += self.scale(ms);
        let deadline = self.base + Duration::from_secs_f64(self.offset_ms / 1000.0);
        self.sleeper.sleep_until(deadline, ctl)
    }

    /// Restarts the clock at the current time; call after an action of unpredictable length.
    pub fn resync(&mut self) {
        self.base = self.sleeper.now();
        self.offset_ms = 0.0;
    }

    pub fn now(&self) -> Instant {
        self.sleeper.now()
    }

    /// Scaled length of `ms` in milliseconds.
    pub fn scale(&self, ms: f64) -> f64 {
        if ms.is_finite() { (ms * self.speed_factor).max(0.0) } else { 0.0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::mock::MockSleeper;

    fn scheduler(speed: f64) -> (Arc<MockSleeper>, Scheduler) {
        let sleeper = Arc::new(MockSleeper::default());
        let sched = Scheduler::new(sleeper.clone(), speed);
        (sleeper, sched)
    }

    #[test]
    fn deadlines_accumulate_without_drift() {
        let (sleeper, mut sched) = scheduler(1.0);
        let ctl = PlayerControl::new();
        for _ in 0..3 {
            assert_eq!(sched.wait(1.0 / 3.0, &ctl), WaitResult::Elapsed);
        }
        assert_eq!(sleeper.total_slept(), Duration::from_micros(1000));

        let (sleeper, mut sched) = scheduler(1.0);
        for _ in 0..300 {
            sched.wait(0.1, &ctl);
        }
        assert_eq!(sleeper.total_slept(), Duration::from_millis(30));
    }

    #[test]
    fn speed_factor_scales_every_wait() {
        let (sleeper, mut sched) = scheduler(0.5);
        let ctl = PlayerControl::new();
        sched.wait(100.0, &ctl);
        sched.wait(300.0, &ctl);
        assert_eq!(sleeper.total_slept(), Duration::from_millis(200));
    }

    #[test]
    fn resync_drops_the_accumulated_offset() {
        let (sleeper, mut sched) = scheduler(1.0);
        let ctl = PlayerControl::new();
        sched.wait(50.0, &ctl);
        sched.resync();
        sched.wait(10.0, &ctl);
        assert_eq!(sleeper.total_slept(), Duration::from_millis(60));
    }

    #[test]
    fn stop_short_circuits_a_wait() {
        let (sleeper, mut sched) = scheduler(1.0);
        let ctl = PlayerControl::new();
        ctl.request_stop();
        assert_eq!(sched.wait(1_000.0, &ctl), WaitResult::Stopped);
        assert_eq!(sleeper.total_slept(), Duration::ZERO);
    }

    #[test]
    fn negative_and_infinite_waits_are_clamped() {
        let (sleeper, mut sched) = scheduler(1.0);
        let ctl = PlayerControl::new();
        sched.wait(-5.0, &ctl);
        sched.wait(f64::INFINITY, &ctl);
        sched.wait(f64::NAN, &ctl);
        assert_eq!(sleeper.total_slept(), Duration::ZERO);
    }
}

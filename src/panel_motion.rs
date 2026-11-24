use std::time::{Duration, Instant};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    SPI_GETCLIENTAREAANIMATION, SystemParametersInfoW,
};

pub(crate) fn enabled() -> bool {
    let mut allowed = 0;
    let ok = unsafe {
        SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, (&mut allowed as *mut i32).cast(), 0)
    };
    ok != 0 && allowed != 0
}

// Critically damped and bounded. Retargeting preserves the current velocity.
const SPEED: f64 = 27.0;
const LIMIT: Duration = Duration::from_millis(340);

pub(crate) struct Motion {
    from: f64,
    velocity: f64,
    target: f64,
    started: Instant,
}

impl Motion {
    pub(crate) fn new(open: bool) -> Self {
        let target = f64::from(open);
        Self {
            from: target,
            velocity: 0.0,
            target,
            started: Instant::now(),
        }
    }

    fn at(&self, now: Instant) -> (f64, f64) {
        let elapsed = now.saturating_duration_since(self.started);
        if elapsed >= LIMIT {
            return (self.target, 0.0);
        }
        let t = elapsed.as_secs_f64();
        let delta = self.from - self.target;
        let slope = self.velocity + SPEED * delta;
        let decay = (-SPEED * t).exp();
        let raw = self.target + (delta + slope * t) * decay;
        let position = raw.clamp(0.0, 1.0);
        let velocity = (self.velocity - SPEED * slope * t) * decay;
        if raw != position {
            (position, 0.0)
        } else {
            (position, velocity)
        }
    }

    pub(crate) fn position(&self) -> f64 {
        self.at(Instant::now()).0
    }

    pub(crate) fn running(&self) -> bool {
        self.started.elapsed() < LIMIT && (self.from != self.target || self.velocity != 0.0)
    }

    pub(crate) fn to(&mut self, open: bool, animate: bool) {
        let now = Instant::now();
        let (from, velocity) = self.at(now);
        self.target = f64::from(open);
        self.started = now;
        self.from = if animate { from } else { self.target };
        self.velocity = if animate { velocity } else { 0.0 };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transition_has_intermediate_values_and_exact_endpoints() {
        let mut motion = Motion::new(true);
        motion.to(false, true);
        let now = motion.started;
        assert!(motion.at(now).0 > 0.99);
        let middle = motion.at(now + Duration::from_millis(80)).0;
        assert!((0.1..0.9).contains(&middle));
        assert_eq!(motion.at(now + LIMIT).0, 0.0);
        motion.to(true, false);
        assert_eq!(motion.position(), 1.0);
        assert!(!motion.running());
    }

    #[test]
    fn reversal_preserves_position_and_velocity() {
        let mut motion = Motion::new(true);
        motion.to(false, true);
        motion.started -= Duration::from_millis(52);
        let before = motion.at(Instant::now());
        motion.to(true, true);
        assert!((motion.from - before.0).abs() < 0.02);
        assert!(motion.velocity < 0.0);
        assert_eq!(motion.at(motion.started + LIMIT).0, 1.0);
    }
}

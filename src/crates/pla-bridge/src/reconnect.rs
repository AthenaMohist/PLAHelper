//! Exponential backoff reconnect policy: initial 1s, factor 2, max 60s, jitter.

use std::time::Duration;

use rand::Rng;

/// Backoff state machine. Reset on successful handshake.
#[derive(Debug, Clone)]
pub struct Backoff {
    initial: Duration,
    factor: f64,
    max: Duration,
    attempts: u32,
}

impl Default for Backoff {
    fn default() -> Self {
        Self::new(Duration::from_secs(1), 2.0, Duration::from_secs(60))
    }
}

impl Backoff {
    pub fn new(initial: Duration, factor: f64, max: Duration) -> Self {
        Self {
            initial,
            factor,
            max,
            attempts: 0,
        }
    }

    /// Returns the next wait interval (with jitter of +/-25%) and advances
    /// the attempt counter. Jitter keeps reconnect storms from aligning.
    pub fn next_delay(&mut self) -> Duration {
        let base = self.base();
        self.attempts = self.attempts.saturating_add(1);
        Self::jitter(base)
    }

    /// Current base delay without jitter, without advancing state.
    pub fn base(&self) -> Duration {
        let exp = f64::from(self.attempts).min(62.0);
        let secs = self.initial.as_secs_f64() * self.factor.powf(exp);
        let secs = secs.min(self.max.as_secs_f64());
        Duration::from_secs_f64(secs.max(0.001))
    }

    /// Full jitter within +/-25% of the base delay.
    fn jitter(base: Duration) -> Duration {
        let base_ms = base.as_millis() as f64;
        let spread = base_ms * 0.25;
        let delta = rand::thread_rng().gen_range(-spread..=spread);
        let jittered = (base_ms + delta).max(1.0);
        Duration::from_millis(jittered as u64)
    }

    /// Reset after a successful handshake.
    pub fn reset(&mut self) {
        self.attempts = 0;
    }

    pub fn attempts(&self) -> u32 {
        self.attempts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grows_exponentially_until_max() {
        let mut b = Backoff::new(Duration::from_secs(1), 2.0, Duration::from_secs(60));
        assert!((b.base().as_secs_f64() - 1.0).abs() < 1e-9);
        let _ = b.next_delay();
        assert!((b.base().as_secs_f64() - 2.0).abs() < 1e-9);
        for _ in 0..20 {
            let _ = b.next_delay();
        }
        assert!(b.base() <= Duration::from_secs(60));
    }

    #[test]
    fn reset_returns_to_initial() {
        let mut b = Backoff::default();
        for _ in 0..5 {
            let _ = b.next_delay();
        }
        b.reset();
        assert_eq!(b.attempts(), 0);
        assert!((b.base().as_secs_f64() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn jitter_stays_near_base() {
        let base = Duration::from_secs(2);
        for _ in 0..100 {
            let j = Backoff::jitter(base);
            assert!(j >= Duration::from_millis(1500));
            assert!(j <= Duration::from_millis(2500));
        }
    }
}

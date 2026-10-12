use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Consecutive failures before temporarily locking a login key.
pub const MAX_FAILURES: u32 = 5;
/// Lockout duration after exceeding `MAX_FAILURES`.
pub const LOCKOUT: Duration = Duration::from_secs(60);
/// Limit of tracked keys; above that we remove the unlocked ones.
const MAX_ENTRIES: usize = 2048;

#[derive(Debug, Default)]
struct Entry {
    failures: u32,
    locked_until: Option<Instant>,
}

/// Simple login attempt throttle per key (user).
#[derive(Debug, Default)]
pub struct LoginThrottle {
    entries: Mutex<HashMap<String, Entry>>,
}

impl LoginThrottle {
    pub fn new() -> Self {
        Self::default()
    }

    /// Remaining lockout seconds, if the key is locked.
    pub fn retry_after(&self, key: &str) -> Option<u64> {
        let now = Instant::now();
        let mut entries = self.entries.lock().ok()?;
        let entry = entries.get_mut(key)?;
        match entry.locked_until {
            Some(until) if until > now => Some((until - now).as_secs().max(1)),
            Some(_) => {
                entries.remove(key);
                None
            }
            None => None,
        }
    }

    pub fn record_failure(&self, key: &str) {
        let now = Instant::now();
        let Ok(mut entries) = self.entries.lock() else {
            return;
        };
        let entry = entries.entry(key.to_string()).or_default();
        entry.failures += 1;
        if entry.failures >= MAX_FAILURES {
            entry.locked_until = Some(now + LOCKOUT);
            entry.failures = 0;
        }
        if entries.len() > MAX_ENTRIES {
            entries.retain(|_, entry| entry.locked_until.is_some_and(|until| until > now));
        }
    }

    pub fn record_success(&self, key: &str) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.remove(key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locks_after_consecutive_failures() {
        let throttle = LoginThrottle::new();
        assert_eq!(throttle.retry_after("root"), None);
        for _ in 0..MAX_FAILURES {
            throttle.record_failure("root");
        }
        assert!(throttle.retry_after("root").is_some());
        assert_eq!(throttle.retry_after("other"), None);
    }

    #[test]
    fn success_clears_failures() {
        let throttle = LoginThrottle::new();
        throttle.record_failure("root");
        throttle.record_failure("root");
        throttle.record_success("root");
        for _ in 0..(MAX_FAILURES - 1) {
            throttle.record_failure("root");
        }
        assert_eq!(throttle.retry_after("root"), None, "success should reset");
    }
}

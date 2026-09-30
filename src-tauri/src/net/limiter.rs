//! Per-service rate limiting: requests to one service are spaced at least
//! its minimum interval apart, however many threads send them.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::Service;

/// Hands out send times, one per request, per service.
#[derive(Default)]
pub(crate) struct RateLimiter {
    /// The earliest time the next request to each service may go out.
    next: Mutex<HashMap<Service, Instant>>,
}

impl RateLimiter {
    /// Books the next free slot for `service`, at least `interval` after
    /// the previous one, and returns when it is. Doesn't wait.
    pub(crate) fn reserve(&self, service: Service, interval: Duration) -> Instant {
        let mut next = self.next.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        let slot = next.get(&service).map_or(now, |&n| n.max(now));
        next.insert(service, slot + interval);
        slot
    }

    /// Waits for the next free slot for `service`.
    pub(crate) fn wait(&self, service: Service, interval: Duration) {
        let slot = self.reserve(service, interval);
        // `sleep` never returns early, so the spacing holds.
        let wait = slot.saturating_duration_since(Instant::now());
        if !wait.is_zero() {
            std::thread::sleep(wait);
        }
    }

    /// Marks a request to `service` as going out now: the next may not go
    /// out until `interval` from now, even if this one's turn came earlier
    /// and it was held up after waiting.
    pub(crate) fn sending(&self, service: Service, interval: Duration) {
        let mut next = self.next.lock().unwrap_or_else(|e| e.into_inner());
        let earliest = Instant::now() + interval;
        let slot = next.entry(service).or_insert(earliest);
        *slot = (*slot).max(earliest);
    }

    /// When the next request to `service` may go out, if one has been
    /// booked.
    #[cfg(test)]
    pub(crate) fn next_slot(&self, service: Service) -> Option<Instant> {
        self.next
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&service)
            .copied()
    }
}

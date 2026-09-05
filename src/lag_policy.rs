//! Slow-subscriber policy shared by the WebSocket and SSE fan-out paths.
//!
//! Both protocols read the same `broadcast` channel, so both see the same
//! `Lagged(n)` signal when a subscriber can't keep up, and both need the same
//! answer to "when do we stop pretending this client is following the stream".

use std::time::{Duration, Instant};

/// Length of the rolling window missed messages are counted over.
pub const WINDOW: Duration = Duration::from_secs(60);

/// Messages a subscriber may miss inside one [`WINDOW`] before it is cut.
///
/// One full broadcast buffer's worth (the default `buffer_size` is 1000)
/// missed inside a minute means the client is sampling the stream, not
/// following it. Counting *consecutive* `Lagged` events instead, and resetting
/// on every successful send, would never cut a client that drops messages
/// continuously but receives one in between.
pub const MAX_DROPPED_PER_WINDOW: u64 = 1000;

/// Tumbling-window count of the messages one subscriber missed.
///
/// Tumbling rather than sliding: the budget resets wholesale once the window
/// elapses. That costs a little precision at the boundary and saves keeping a
/// timestamp per drop, which for a policy whose threshold is "three digits of
/// lost messages" is not a trade worth making.
pub struct LagWindow {
    window_start: Instant,
    dropped: u64,
}

impl LagWindow {
    pub fn new(now: Instant) -> Self {
        Self {
            window_start: now,
            dropped: 0,
        }
    }

    /// Record `n` missed messages. Returns `true` once this subscriber has
    /// spent its budget for the current window and must be disconnected.
    pub fn record(&mut self, now: Instant, n: u64) -> bool {
        if now.duration_since(self.window_start) >= WINDOW {
            self.window_start = now;
            self.dropped = 0;
        }
        self.dropped = self.dropped.saturating_add(n);
        self.dropped >= MAX_DROPPED_PER_WINDOW
    }

    pub fn dropped_in_window(&self) -> u64 {
        self.dropped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Guards the way this policy can be disabled without looking disabled:
    /// a budget set high enough that no real client ever reaches it. The exact
    /// value is a tuning decision and is not pinned here — the behavioural
    /// tests below read it symbolically.
    #[test]
    fn the_budget_stays_within_reach_of_a_real_client() {
        const _: () = assert!(MAX_DROPPED_PER_WINDOW >= 100);
        const _: () = assert!(MAX_DROPPED_PER_WINDOW <= 100_000);
        assert!(WINDOW <= Duration::from_secs(600));
    }

    #[test]
    fn drops_inside_one_window_accumulate_to_the_threshold() {
        let t0 = Instant::now();
        let mut w = LagWindow::new(t0);

        for i in 1..MAX_DROPPED_PER_WINDOW {
            assert!(
                !w.record(t0 + Duration::from_millis(i), 1),
                "cut at {i} drops, budget is {MAX_DROPPED_PER_WINDOW}"
            );
        }
        assert!(w.record(t0 + Duration::from_secs(1), 1));
        assert_eq!(w.dropped_in_window(), MAX_DROPPED_PER_WINDOW);
    }

    /// The case the old consecutive-lag counter could not see: a client that
    /// keeps missing messages the whole time but receives enough in between
    /// that it never lags twice in a row.
    #[test]
    fn steady_trickle_of_drops_still_trips_the_budget() {
        let t0 = Instant::now();
        let mut w = LagWindow::new(t0);
        let mut cut_at = None;

        // 20 drops every second — well inside the window, and under the old
        // policy every one of them was followed by a successful send.
        for sec in 0..59 {
            if w.record(t0 + Duration::from_secs(sec), 20) {
                cut_at = Some(sec);
                break;
            }
        }
        assert_eq!(
            cut_at,
            Some(MAX_DROPPED_PER_WINDOW / 20 - 1),
            "expected the cut on the tick where the drops reach the budget"
        );
    }

    #[test]
    fn drops_spread_across_windows_never_trip_it() {
        let t0 = Instant::now();
        let mut w = LagWindow::new(t0);

        // Just under the budget, once per window, for an hour.
        for window in 0..60 {
            let now = t0 + WINDOW * window;
            assert!(!w.record(now, MAX_DROPPED_PER_WINDOW - 1));
            assert_eq!(w.dropped_in_window(), MAX_DROPPED_PER_WINDOW - 1);
        }
    }

    #[test]
    fn a_single_oversized_lag_trips_it_immediately() {
        let t0 = Instant::now();
        let mut w = LagWindow::new(t0);
        assert!(w.record(t0, MAX_DROPPED_PER_WINDOW * 4));
    }
}

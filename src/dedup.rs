use dashmap::DashMap;
use dashmap::mapref::entry::Entry;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info};

/// Default capacity reset to 200K in 1.5.0 after the memory audit. The 1M
/// cap inherited from 1.4 cost ~38 MiB of resident memory for a window that
/// only needed ~200K entries in practice (ingest rate ~1K unique/sec × 15-min
/// TTL = 900K theoretical max but real cross-log dedup converges much lower
/// because TTL eviction churns continuously). Operators who really want a
/// wider window can override via `CERTSTREAM_DEDUP_CAPACITY` or YAML
/// `dedup.capacity`; the trade-off is purely memory ↔ deeper cross-log
/// dedup, never correctness.
const DEFAULT_CAPACITY: usize = 200_000;
/// 15 minutes — comfortably covers the typical multi-log SCT propagation window
/// (a few minutes) plus headroom for slower static-ct shards. Configurable via
/// `dedup.ttl_secs` in YAML or `CERTSTREAM_DEDUP_TTL_SECS`.
const DEFAULT_TTL_SECS: u64 = 900;
// 15s (down from 60s in v1.5.x) — at typical ingest rates the 60s cycle
// could let the map grow ~60k entries above steady state between sweeps,
// producing a sawtooth RSS curve. 15s flattens the curve and gives the
// allocator a chance to release pages sooner.
const DEFAULT_CLEANUP_INTERVAL_SECS: u64 = 15;

/// Issue #1: Use raw [u8; 32] SHA-256 bytes as the key — fixed-size, stack-allocated,
/// trivially hashable. Eliminates one heap allocation per certificate on every lookup.
///
/// The key is already a uniformly-distributed SHA-256 digest, so the map's
/// hasher only needs to fold it into a u64 — ahash does that in a few cycles,
/// vs SipHash's full keyed permutation on every lookup.
pub struct DedupFilter {
    seen: DashMap<[u8; 32], Instant, ahash::RandomState>,
    /// Upper bound on entries. Enforced by `cleanup`, which narrows the
    /// effective TTL window when the map overshoots, so the bound costs
    /// nothing on the insert path.
    capacity: usize,
    ttl: Duration,
}

impl DedupFilter {
    pub fn new() -> Self {
        Self::with_config(DEFAULT_CAPACITY, Duration::from_secs(DEFAULT_TTL_SECS))
    }

    pub fn with_config(capacity: usize, ttl: Duration) -> Self {
        Self {
            seen: DashMap::with_capacity_and_hasher(
                capacity.max(4) / 4,
                ahash::RandomState::default(),
            ),
            capacity: capacity.max(1),
            ttl,
        }
    }

    /// Returns true if this SHA-256 fingerprint has NOT been seen before (i.e., is new).
    /// Takes the raw 32-byte digest — zero heap allocation on both hit and miss.
    ///
    /// Atomicity: uses `DashMap::entry` so the test-and-insert pair is locked
    /// per-shard. Two concurrent calls with the same key can never both
    /// observe "not present" and both return true.
    ///
    /// **Cost note:** both TTL expiry and the capacity bound are applied by
    /// `cleanup`, never here. An earlier version called `evict_expired()`
    /// inline whenever `len() >= capacity`, which thrashed CPU when ingest
    /// rate × TTL exceeded the cap (every insert ran a full O(n) shard scan).
    /// The hot path only does the entry lookup.
    pub fn is_new(&self, sha256_raw: &[u8; 32]) -> bool {
        let now = Instant::now();

        match self.seen.entry(*sha256_raw) {
            Entry::Occupied(mut e) => {
                let age = now.duration_since(*e.get());
                if age > self.ttl {
                    *e.get_mut() = now;
                    true
                } else {
                    metrics::counter!("certstream_duplicates_filtered").increment(1);
                    false
                }
            }
            Entry::Vacant(v) => {
                v.insert(now);
                true
            }
        }
    }

    pub fn cleanup(&self) {
        let before = self.seen.len();
        let now = Instant::now();

        // `capacity` used to bound nothing: TTL was the only thing pruning the
        // map, so the steady-state size was ingest rate × TTL. Measured on the
        // live workload that is ~420 certs/s × 900 s = ~354K entries against a
        // configured 200K, so anyone sizing memory from `capacity` was out by
        // that factor.
        //
        // Rather than evicting oldest-first (which needs the ages sorted, an
        // allocation proportional to the map) the window itself is narrowed in
        // proportion to the overshoot: keeping 200K of 354K entries means
        // keeping the newest ~56% of the TTL window. Arrival is close enough to
        // uniform across a 15-minute window that this lands within a few
        // percent of exact oldest-first eviction, and it rides along on the
        // sweep that already walks every entry.
        //
        // Narrowing the window can only let more duplicates through, never
        // fewer, so an over-capacity filter degrades in dedup depth and never
        // in correctness.
        let effective_ttl = if before > self.capacity {
            let ratio = self.capacity as f64 / before as f64;
            metrics::counter!("certstream_dedup_capacity_trims").increment(1);
            self.ttl.mul_f64(ratio)
        } else {
            self.ttl
        };

        self.seen
            .retain(|_, v| now.duration_since(*v) < effective_ttl);
        let removed = before.saturating_sub(self.seen.len());
        if removed > 0 {
            debug!(
                removed = removed,
                remaining = self.seen.len(),
                effective_ttl_secs = effective_ttl.as_secs_f64(),
                "dedup cleanup"
            );
        }
        metrics::gauge!("certstream_dedup_cache_size").set(self.seen.len() as f64);
        metrics::gauge!("certstream_dedup_effective_ttl_seconds").set(effective_ttl.as_secs_f64());
    }

    /// Snapshot of current entry count. Used by the heartbeat log and by
    /// tests; the `certstream_dedup_cache_size` Prometheus gauge carries
    /// the same value for scrape-based monitoring.
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.seen.len()
    }

    pub fn start_cleanup_task(self: Arc<Self>, cancel: CancellationToken) {
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(DEFAULT_CLEANUP_INTERVAL_SECS));
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => {
                        info!("dedup cleanup task stopping");
                        break;
                    }
                    _ = tick.tick() => {
                        self.cleanup();
                    }
                }
            }
        });
    }
}

impl Default for DedupFilter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    fn key(n: u8) -> [u8; 32] {
        let mut k = [0u8; 32];
        k[0] = n;
        k
    }

    #[test]
    fn test_is_new_first_seen() {
        let filter = DedupFilter::new();
        assert!(filter.is_new(&key(1)));
        assert!(filter.is_new(&key(2)));
    }

    #[test]
    fn test_is_new_duplicate() {
        let filter = DedupFilter::new();
        assert!(filter.is_new(&key(1)));
        assert!(!filter.is_new(&key(1))); // second time → duplicate
        assert!(!filter.is_new(&key(1))); // third time → still duplicate
    }

    #[test]
    fn test_is_new_different_keys() {
        let filter = DedupFilter::new();
        assert!(filter.is_new(&key(1)));
        assert!(filter.is_new(&key(2)));
        assert!(filter.is_new(&key(3)));
        assert!(!filter.is_new(&key(1)));
        assert!(!filter.is_new(&key(2)));
    }

    #[test]
    fn test_is_new_ttl_expiry() {
        let filter = DedupFilter {
            seen: DashMap::with_capacity_and_hasher(100, ahash::RandomState::default()),
            capacity: DEFAULT_CAPACITY,
            ttl: Duration::from_millis(50),
        };

        let k = key(42);
        assert!(filter.is_new(&k));
        assert!(!filter.is_new(&k));

        thread::sleep(Duration::from_millis(60));

        // Should be treated as new again after TTL expiry
        assert!(filter.is_new(&k));
    }

    #[test]
    fn test_cleanup_over_capacity_narrows_window() {
        // Over capacity, entries younger than the full TTL are still evicted:
        // the window shrinks in proportion to the overshoot. Before this the
        // map grew to ingest-rate × TTL and `capacity` bounded nothing.
        let filter = DedupFilter {
            seen: DashMap::with_capacity_and_hasher(64, ahash::RandomState::default()),
            capacity: 5,
            ttl: Duration::from_millis(400),
        };

        for i in 0u8..20 {
            assert!(filter.is_new(&key(i)));
        }
        thread::sleep(Duration::from_millis(120));
        for i in 20u8..40 {
            assert!(filter.is_new(&key(i)));
        }

        // 40 entries against a capacity of 5 gives an effective window of
        // 400ms × 5/40 = 50ms, so the first batch (120ms old) goes even though
        // the configured TTL has not elapsed for any of them.
        filter.cleanup();
        assert!(filter.len() < 40, "cleanup should have evicted the older batch");
        assert!(filter.is_new(&key(0)), "oldest entry should be gone");
        assert!(!filter.is_new(&key(39)), "newest entry should still be held");
    }

    #[test]
    fn test_cleanup_under_capacity_keeps_full_ttl() {
        let filter = DedupFilter {
            seen: DashMap::with_capacity_and_hasher(64, ahash::RandomState::default()),
            capacity: 1000,
            ttl: Duration::from_secs(300),
        };

        for i in 0u8..10 {
            assert!(filter.is_new(&key(i)));
        }
        filter.cleanup();
        assert_eq!(filter.len(), 10, "nothing expired, nothing over capacity");
    }

    #[test]
    fn test_is_new_capacity_overflow_no_wipe() {
        // Regression: at capacity, the old code wiped the entire map via
        // `clear()` and silently re-broadcast every in-flight cert. The new
        // path only evicts genuinely expired entries, so fresh keys survive.
        let filter = DedupFilter {
            seen: DashMap::with_capacity_and_hasher(4, ahash::RandomState::default()),
            capacity: 5,
            ttl: Duration::from_secs(300),
        };

        for i in 0u8..5 {
            assert!(filter.is_new(&key(i)));
        }
        assert_eq!(filter.len(), 5);

        // Insert past capacity — soft eviction runs but nothing is expired,
        // so the new key is added on top and previously-seen keys are still
        // treated as duplicates (no catastrophic clear).
        assert!(filter.is_new(&key(255)));
        assert!(!filter.is_new(&key(0)));
        assert!(!filter.is_new(&key(4)));
        assert!(filter.len() >= 5);
    }

    #[test]
    fn test_is_new_atomic_under_contention() {
        use std::sync::Arc;
        use std::thread;
        use std::sync::atomic::{AtomicUsize, Ordering};

        // 32 threads, each calls is_new on the same key 1000 times.
        // Exactly ONE call across all threads should ever return true.
        let filter = Arc::new(DedupFilter::new());
        let true_count = Arc::new(AtomicUsize::new(0));
        let k = key(7);

        let handles: Vec<_> = (0..32)
            .map(|_| {
                let f = Arc::clone(&filter);
                let c = Arc::clone(&true_count);
                thread::spawn(move || {
                    for _ in 0..1000 {
                        if f.is_new(&k) {
                            c.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(true_count.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_cleanup_removes_expired() {
        let filter = DedupFilter {
            seen: DashMap::with_capacity_and_hasher(100, ahash::RandomState::default()),
            capacity: DEFAULT_CAPACITY,
            ttl: Duration::from_millis(50),
        };

        filter.is_new(&key(1));
        filter.is_new(&key(2));
        assert_eq!(filter.len(), 2);

        thread::sleep(Duration::from_millis(60));

        // Add a fresh entry
        filter.is_new(&key(3));

        filter.cleanup();

        // key1 and key2 should be removed, key3 should remain
        assert_eq!(filter.len(), 1);
        assert!(filter.is_new(&key(1))); // key1 was cleaned up, so it's new again
    }

    #[test]
    fn test_cleanup_keeps_fresh_entries() {
        let filter = DedupFilter::new();
        filter.is_new(&key(1));
        filter.is_new(&key(2));
        filter.is_new(&key(3));

        filter.cleanup();

        // All entries are fresh, none removed
        assert_eq!(filter.len(), 3);
        assert!(!filter.is_new(&key(1))); // still a duplicate
    }

    #[test]
    fn test_len() {
        let filter = DedupFilter::new();
        assert_eq!(filter.len(), 0);

        filter.is_new(&key(1));
        assert_eq!(filter.len(), 1);

        filter.is_new(&key(2));
        assert_eq!(filter.len(), 2);

        filter.is_new(&key(1)); // duplicate, no new entry
        assert_eq!(filter.len(), 2);
    }

    #[test]
    fn test_zero_key() {
        let filter = DedupFilter::new();
        let z = [0u8; 32];
        assert!(filter.is_new(&z));
        assert!(!filter.is_new(&z));
    }

    #[tokio::test]
    async fn test_cleanup_task_stops_on_cancellation() {
        let filter = Arc::new(DedupFilter::new());
        let cancel = CancellationToken::new();

        filter.clone().start_cleanup_task(cancel.clone());

        tokio::time::sleep(Duration::from_millis(50)).await;

        cancel.cancel();

        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

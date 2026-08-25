//! Allocator-level heap statistics.
//!
//! RSS on its own cannot separate "the process is holding this data" from
//! "jemalloc has not handed these pages back to the OS yet". `allocated`
//! answers the first question, `resident` the second, and the gap between
//! them is what tells an operator whether to shrink a cache or to retune the
//! allocator. Both are exported so that question is answerable from
//! `/metrics` instead of from a heap profiler attached after the fact.

/// Live heap figures from the allocator, in bytes.
#[derive(Debug, Clone, Copy, Default)]
pub struct HeapUsage {
    /// Bytes the application currently holds.
    pub allocated: u64,
    /// Bytes in physically resident pages mapped by the allocator. Tracks
    /// process RSS closely, minus stacks and non-allocator mappings.
    pub resident: u64,
}

#[cfg(not(target_env = "msvc"))]
pub fn record() -> Option<HeapUsage> {
    use tikv_jemalloc_ctl::{epoch, stats};

    // jemalloc caches its counters and only refreshes them when the epoch is
    // advanced. Skipping this makes every read return the values from process
    // start, which looks like a perfectly flat heap.
    if let Err(e) = epoch::advance() {
        tracing::debug!(error = %e, "jemalloc epoch advance failed");
        return None;
    }

    let mut usage = HeapUsage::default();

    if let Ok(v) = stats::allocated::read() {
        usage.allocated = v as u64;
        metrics::gauge!("certstream_jemalloc_allocated_bytes").set(v as f64);
    }
    if let Ok(v) = stats::resident::read() {
        usage.resident = v as u64;
        metrics::gauge!("certstream_jemalloc_resident_bytes").set(v as f64);
    }
    // active - allocated is per-object rounding waste; mapped - resident and
    // retained together describe address space jemalloc holds but does not
    // pay for in physical memory.
    if let Ok(v) = stats::active::read() {
        metrics::gauge!("certstream_jemalloc_active_bytes").set(v as f64);
    }
    if let Ok(v) = stats::mapped::read() {
        metrics::gauge!("certstream_jemalloc_mapped_bytes").set(v as f64);
    }
    if let Ok(v) = stats::retained::read() {
        metrics::gauge!("certstream_jemalloc_retained_bytes").set(v as f64);
    }
    if let Ok(v) = stats::metadata::read() {
        metrics::gauge!("certstream_jemalloc_metadata_bytes").set(v as f64);
    }

    Some(usage)
}

#[cfg(target_env = "msvc")]
pub fn record() -> Option<HeapUsage> {
    None
}

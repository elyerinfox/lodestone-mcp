//! Retrieval delegation — the "go fetch this URL for me" service this node
//! optionally offers to constellation peers, plus the per-peer and global
//! rate limits that protect it.
//!
//! ## What it is
//!
//! When `[network].delegation_enabled = true`, a peer that has the URL in
//! its Bloom but **can't reach the upstream** (rate-limited, geo-blocked,
//! captive-portalled, …) can POST `/constellation/retrieve {url, …}` to
//! ask another node to perform the fetch on its behalf. The serving node
//! fetches from upstream, caches the body in its own
//! [`IndexedRetrievalCache`](crate::retrieval::IndexedRetrievalCache) under
//! the supplied `source` classifier, and returns the bytes. The requester
//! cache-stores the result, the next consumer in the asking constellation
//! consults the requester's cache via the existing
//! [`consult_blob_hash`](super::Constellation::consult_blob_hash_sourced)
//! path, and the rate-limited upstream is hit **once** for the whole mesh
//! instead of once per node.
//!
//! ## Guardrails
//!
//! Three sliding-hour-window counters protect the serving node from being
//! used as someone else's exit:
//!
//! - **`jobs_per_peer_per_hour`** — caps how many delegated fetches any
//!   single peer can request per hour. A misbehaving peer fills its own
//!   quota first.
//! - **`bytes_per_job`** — caps the body size of any single delegated
//!   fetch (a peer can't ask us to download a 5 GB file).
//! - **`total_bytes_per_hour`** — caps the aggregate bytes served via
//!   delegation per hour, summed across all peers. Protects the local
//!   egress / ingress budget against fan-out.
//!
//! All three are configurable via `[network].delegation_*`. Over-budget
//! requests return HTTP 429 with a `Retry-After` header.
//!
//! ## Privacy
//!
//! The requested URL **does** cross the wire here (it has to — the serving
//! node fetches it), so delegation is **strictly opt-in** for the serving
//! node: never publish outbound traffic for someone else without choosing
//! to. The requesting node has no privacy obligation — it asked for the URL,
//! it knows the URL. The constellation token (`[network].token`) gates who
//! can ask in the first place.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Why a delegation request was rejected. Maps directly to the HTTP 429 body
/// the endpoint returns, so the requester can decide whether to back off,
/// reduce, or try a different peer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RejectReason {
    /// This specific peer has hit `delegation_max_jobs_per_peer_per_hour`.
    /// They should wait and retry, or use a different exit. Carries the
    /// number of seconds until at least one job ages out of their window.
    PeerJobsExceeded { retry_after_secs: u64 },
    /// The request itself asked for more bytes than
    /// `delegation_max_bytes_per_job` allows. Not retryable — fundamentally
    /// too big.
    PerJobBytesExceeded { limit: u64, requested: u64 },
    /// The global `delegation_total_bytes_per_hour` is saturated. Try a
    /// different peer or wait. Carries seconds until the global budget
    /// recovers some headroom.
    GlobalBytesExceeded { retry_after_secs: u64 },
    /// Delegation is disabled on this node. Final — caller shouldn't retry.
    Disabled,
}

/// Sliding-hour-window state for a single peer's job count and a global
/// byte counter. A single mutex covers both because acquisition is a single
/// transactional check (per-peer + global) and contention is low (one
/// delegation request = one mutex grab).
pub struct DelegationLimiter {
    enabled: bool,
    max_jobs_per_peer_per_hour: u32,
    max_bytes_per_job: u64,
    total_bytes_per_hour: u64,
    state: Mutex<State>,
}

const WINDOW: Duration = Duration::from_secs(3600);

struct State {
    /// Per-peer job timestamps over the last hour. Each successful
    /// `try_acquire` pushes `Instant::now()`; expired entries are pruned at
    /// the next check.
    per_peer_jobs: HashMap<String, Vec<Instant>>,
    /// Global byte counters as `(when, bytes)` over the last hour. Expired
    /// entries pruned on each check.
    global_bytes: Vec<(Instant, u64)>,
}

impl DelegationLimiter {
    pub fn new(
        enabled: bool,
        max_jobs_per_peer_per_hour: u32,
        max_bytes_per_job: u64,
        total_bytes_per_hour: u64,
    ) -> Self {
        Self {
            enabled,
            max_jobs_per_peer_per_hour,
            max_bytes_per_job,
            total_bytes_per_hour,
            state: Mutex::new(State {
                per_peer_jobs: HashMap::new(),
                global_bytes: Vec::new(),
            }),
        }
    }

    /// Reserve one delegation slot for `peer_id`, expecting up to
    /// `expected_bytes` to be served. Returns `Ok(slot)` on success — the
    /// caller calls [`Slot::commit`] once it knows the actual byte count to
    /// finalise the accounting, or drops the slot to refund the global
    /// budget on failure. Returns `Err(RejectReason)` on rejection.
    pub fn try_acquire(
        &self,
        peer_id: &str,
        expected_bytes: u64,
    ) -> Result<Slot<'_>, RejectReason> {
        if !self.enabled {
            return Err(RejectReason::Disabled);
        }
        if expected_bytes > self.max_bytes_per_job {
            return Err(RejectReason::PerJobBytesExceeded {
                limit: self.max_bytes_per_job,
                requested: expected_bytes,
            });
        }
        let now = Instant::now();
        let mut state = self.state.lock().unwrap();

        // Per-peer sliding window over jobs. Prune + count in a scoped
        // borrow; we'll reach back to push only after the global-bytes
        // check passes.
        {
            let entry = state.per_peer_jobs.entry(peer_id.to_string()).or_default();
            entry.retain(|t| now.duration_since(*t) <= WINDOW);
            if entry.len() as u32 >= self.max_jobs_per_peer_per_hour {
                let oldest = entry.first().copied().unwrap_or(now);
                let elapsed = now.duration_since(oldest);
                let retry_after_secs = WINDOW.saturating_sub(elapsed).as_secs().max(1);
                return Err(RejectReason::PeerJobsExceeded { retry_after_secs });
            }
        }

        // Global sliding window over bytes.
        state
            .global_bytes
            .retain(|(t, _)| now.duration_since(*t) <= WINDOW);
        let used: u64 = state.global_bytes.iter().map(|(_, b)| *b).sum();
        if used + expected_bytes > self.total_bytes_per_hour {
            // Find the oldest reservation that would, on expiry, give us
            // enough headroom — that's our Retry-After hint.
            let need = (used + expected_bytes).saturating_sub(self.total_bytes_per_hour);
            let mut freed = 0u64;
            let mut retry_at = now;
            for (t, b) in &state.global_bytes {
                freed += *b;
                if freed >= need {
                    retry_at = *t + WINDOW;
                    break;
                }
            }
            let retry_after_secs = retry_at.saturating_duration_since(now).as_secs().max(1);
            return Err(RejectReason::GlobalBytesExceeded { retry_after_secs });
        }

        // Reserve: push the job timestamp + a tentative byte reservation.
        // `Slot::commit` will replace the tentative byte count with the
        // actual one once known; `Drop` on a non-committed slot rolls
        // both back so a failed fetch doesn't permanently consume budget.
        state
            .per_peer_jobs
            .get_mut(peer_id)
            .expect("entry was inserted above")
            .push(now);
        state.global_bytes.push((now, expected_bytes));
        let slot_idx = state.global_bytes.len() - 1;

        Ok(Slot {
            limiter: self,
            peer_id: peer_id.to_string(),
            timestamp: now,
            slot_idx,
            tentative_bytes: expected_bytes,
            committed: false,
        })
    }

    /// Current byte spend in the active window. Used by tests + the
    /// `/constellation/retrieve` 429 body to report headroom.
    #[allow(dead_code)] // exercised by the delegation unit tests
    pub fn bytes_used(&self) -> u64 {
        let now = Instant::now();
        let mut state = self.state.lock().unwrap();
        state
            .global_bytes
            .retain(|(t, _)| now.duration_since(*t) <= WINDOW);
        state.global_bytes.iter().map(|(_, b)| *b).sum()
    }

    /// Number of jobs the given peer has spent in the active window.
    #[allow(dead_code)] // exercised by the delegation unit tests
    pub fn peer_jobs_used(&self, peer_id: &str) -> usize {
        let now = Instant::now();
        let mut state = self.state.lock().unwrap();
        if let Some(entry) = state.per_peer_jobs.get_mut(peer_id) {
            entry.retain(|t| now.duration_since(*t) <= WINDOW);
            entry.len()
        } else {
            0
        }
    }
}

/// One outstanding delegation reservation. Held while the fetch is in
/// flight; the caller calls `commit(actual_bytes)` once the fetch finishes
/// to record the true byte count. Dropping without committing rolls the
/// reservation back so a failed fetch doesn't burn budget.
#[derive(Debug)]
pub struct Slot<'a> {
    limiter: &'a DelegationLimiter,
    peer_id: String,
    timestamp: Instant,
    slot_idx: usize,
    tentative_bytes: u64,
    committed: bool,
}

impl std::fmt::Debug for DelegationLimiter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DelegationLimiter")
            .field("enabled", &self.enabled)
            .field(
                "max_jobs_per_peer_per_hour",
                &self.max_jobs_per_peer_per_hour,
            )
            .field("max_bytes_per_job", &self.max_bytes_per_job)
            .field("total_bytes_per_hour", &self.total_bytes_per_hour)
            .finish()
    }
}

impl Slot<'_> {
    /// Finalise the reservation with the actual byte count. After this the
    /// slot is consumed: `Drop` does nothing.
    pub fn commit(mut self, actual_bytes: u64) {
        let mut state = self.limiter.state.lock().unwrap();
        // The slot index may have shifted if the vec was pruned between
        // acquire and commit — find by (timestamp, tentative_bytes) tuple
        // and update in place.
        if let Some(slot) = state
            .global_bytes
            .get_mut(self.slot_idx)
            .filter(|s| s.0 == self.timestamp && s.1 == self.tentative_bytes)
        {
            slot.1 = actual_bytes;
        } else if let Some(slot) = state
            .global_bytes
            .iter_mut()
            .find(|s| s.0 == self.timestamp && s.1 == self.tentative_bytes)
        {
            slot.1 = actual_bytes;
        }
        self.committed = true;
    }
}

impl Drop for Slot<'_> {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        // Roll back: remove our tentative reservation + our job timestamp.
        let mut state = self.limiter.state.lock().unwrap();
        if let Some(pos) = state
            .global_bytes
            .iter()
            .position(|s| s.0 == self.timestamp && s.1 == self.tentative_bytes)
        {
            state.global_bytes.remove(pos);
        }
        if let Some(entry) = state.per_peer_jobs.get_mut(&self.peer_id) {
            if let Some(pos) = entry.iter().position(|t| *t == self.timestamp) {
                entry.remove(pos);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limiter(enabled: bool, jobs: u32, bytes_job: u64, bytes_hr: u64) -> DelegationLimiter {
        DelegationLimiter::new(enabled, jobs, bytes_job, bytes_hr)
    }

    #[test]
    fn disabled_rejects_with_disabled_reason() {
        let lim = limiter(false, 100, 1_000_000, 10_000_000);
        let err = lim.try_acquire("peer", 1024).unwrap_err();
        assert_eq!(err, RejectReason::Disabled);
    }

    #[test]
    fn per_job_byte_cap_rejects_oversized_request() {
        let lim = limiter(true, 100, 1024, 1_000_000);
        let err = lim.try_acquire("peer", 2048).unwrap_err();
        assert!(matches!(
            err,
            RejectReason::PerJobBytesExceeded {
                limit: 1024,
                requested: 2048
            }
        ));
        // Counter not consumed: the rejection happened before reservation.
        assert_eq!(lim.peer_jobs_used("peer"), 0);
        assert_eq!(lim.bytes_used(), 0);
    }

    #[test]
    fn per_peer_jobs_cap_rejects_after_quota_exhausted() {
        let lim = limiter(true, 2, 1024, 10_000_000);
        let s1 = lim.try_acquire("peer", 100).unwrap();
        s1.commit(100);
        let s2 = lim.try_acquire("peer", 100).unwrap();
        s2.commit(100);
        let err = lim.try_acquire("peer", 100).unwrap_err();
        assert!(matches!(err, RejectReason::PeerJobsExceeded { .. }));
        // A *different* peer is unaffected.
        assert!(lim.try_acquire("other", 100).is_ok());
    }

    #[test]
    fn global_bytes_cap_rejects_when_aggregate_exceeded() {
        let lim = limiter(true, 100, 1_000_000, 500);
        let s1 = lim.try_acquire("peer-a", 300).unwrap();
        s1.commit(300);
        let s2 = lim.try_acquire("peer-b", 200).unwrap();
        s2.commit(200);
        let err = lim.try_acquire("peer-c", 100).unwrap_err();
        assert!(matches!(err, RejectReason::GlobalBytesExceeded { .. }));
    }

    #[test]
    fn dropped_slot_rolls_back_reservation() {
        // 10_000-byte per-job cap so 5000 fits; 100_000 hourly cap; 2-job
        // per-peer cap. Reserve 5000, then drop the slot to simulate a
        // failed fetch — both the job-count and byte-budget reservations
        // should revert.
        let lim = limiter(true, 2, 10_000, 100_000);
        let s = lim.try_acquire("peer", 5000).unwrap();
        assert_eq!(lim.peer_jobs_used("peer"), 1);
        assert_eq!(lim.bytes_used(), 5000);
        drop(s); // simulate a failed fetch
        assert_eq!(lim.peer_jobs_used("peer"), 0);
        assert_eq!(lim.bytes_used(), 0);
    }

    #[test]
    fn commit_updates_actual_bytes() {
        let lim = limiter(true, 10, 1_000_000, 10_000_000);
        let s = lim.try_acquire("peer", 5000).unwrap();
        s.commit(8000); // upstream returned more than we estimated
        assert_eq!(lim.bytes_used(), 8000);
    }

    #[test]
    fn rejection_carries_retry_after_hint() {
        let lim = limiter(true, 1, 1024, 100);
        let s = lim.try_acquire("peer", 100).unwrap();
        s.commit(100);
        // Per-peer cap fires first (1-job cap).
        match lim.try_acquire("peer", 1).unwrap_err() {
            RejectReason::PeerJobsExceeded { retry_after_secs } => {
                assert!(retry_after_secs >= 1);
                assert!(retry_after_secs <= 3600);
            }
            other => panic!("expected PeerJobsExceeded, got {other:?}"),
        }
        // A different peer also can't squeeze in: global byte cap fires.
        match lim.try_acquire("other", 50).unwrap_err() {
            RejectReason::GlobalBytesExceeded { retry_after_secs } => {
                assert!(retry_after_secs >= 1);
            }
            other => panic!("expected GlobalBytesExceeded, got {other:?}"),
        }
    }
}

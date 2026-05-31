//! Multi-identifier retrieval cache used by both the local
//! [`Lodestone::retrieval_lookup`](crate::Lodestone::retrieval_lookup) path
//! and the constellation's digest / `blob_lookup` path.
//!
//! Each [`Entry`] is reachable by **any** of its declared identifiers —
//! primary cache key, URL aliases, source-specific identifiers (arXiv id,
//! Wayback `(url, timestamp)`, GitHub release tag, …), and the body's
//! content hash. The constellation digest emits every identifier hash, so a
//! peer that asks by *any* of them gets a Bloom hit — closing the alignment
//! gap that made the mesh useless for long-tail rate-limited content.
//!
//! ## Layout
//!
//! Two maps under one mutex:
//! - `entries: HashMap<EntryId, Entry>` — the primary store. `Entry` owns
//!   the body, its [`crate::constellation::Identifiers`], its expiry, and
//!   the *hashed* keys it advertises (precomputed so the digest can
//!   enumerate without re-hashing every iteration).
//! - `by_hash: HashMap<String, EntryId>` — secondary lookup. Every
//!   identifier hash maps to the entry's id. On eviction the entry's hash
//!   list is walked to drop all secondary mappings in one sweep.
//!
//! Entry ids are a monotonic `u64` counter (not the hash of the primary
//! key), so a `put` that overwrites a stale primary key safely replaces
//! the entry without orphaning any secondary mapping (the old entry's
//! mappings are removed by id before the new ones go in).
//!
//! ## TTL
//!
//! Default TTL comes from `[cache].ttl_secs`. A per-entry
//! [`Source::ttl_secs_override`] takes precedence — Wayback / arXiv /
//! GitHub-release entries live a week, Overpass a day, search engines an
//! hour, and `Other` falls back to the global default. Eviction is lazy
//! (checked at `lookup` / `keys`), with an opportunistic sweep on `put`
//! when the cache hits its size cap.
//!
//! ## Single-process
//!
//! The original `TtlCache` had an optional Redis backend for shared-cache
//! deployments. The indexed cache is **in-memory only** for v1 — multi-key
//! Redis with atomic secondary-index updates needs proper Redis SETs +
//! `MULTI`/`EXEC` transactions and is deferred. Single-node deployments are
//! the default; multi-node deployments today share via the constellation,
//! not via a shared Redis.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::constellation::{hash_bytes, hash_key, Identifiers};

/// Opaque entry handle — a monotonically-increasing counter so overwrite of
/// a primary key safely replaces the entry without orphaning aliases.
type EntryId = u64;

/// One body + everything needed to enumerate its identifiers for the
/// constellation digest and to evict it cleanly later.
struct Entry {
    body: String,
    /// Pre-hashed identifier keys (primary, URL aliases, source-id aliases,
    /// and the content hash). Stored so `keys()` and eviction don't re-hash
    /// on every call.
    identifier_hashes: Vec<String>,
    /// When this entry stops being served. Lazy: checked at lookup time.
    expires: Instant,
}

/// The retrieval-cache state. See module docs for layout / TTL / single-
/// process notes.
pub struct IndexedRetrievalCache {
    inner: Mutex<Inner>,
    ttl_default: Duration,
    max_entries: usize,
    /// Cap on the *summed body size* of all live entries. 0 = no byte cap
    /// (rely solely on `max_entries`). When the cap would be exceeded by
    /// a `put`, oldest-by-expiry entries are evicted until the new entry
    /// fits. Operators set this to protect cache memory against
    /// large-body delegation traffic — see
    /// `[network].delegation_max_cache_bytes`.
    max_bytes: u64,
    next_id: AtomicU64,
}

struct Inner {
    entries: HashMap<EntryId, Entry>,
    by_hash: HashMap<String, EntryId>,
    /// Sum of `entries[*].body.len()` — incremented in `put`, decremented
    /// at every eviction so the byte cap accounting stays consistent
    /// with the actual entry table without an O(N) scan.
    bytes_used: u64,
}

impl IndexedRetrievalCache {
    /// Build a fresh in-memory indexed retrieval cache. `ttl_secs` is the
    /// default lifetime (per-entry overrides from
    /// [`Source::ttl_secs_override`] take precedence). `max_entries` bounds
    /// the entry count; `max_bytes` (0 = unlimited) bounds the summed body
    /// size. Oldest-by-expiry entries are evicted first whenever either
    /// cap fires.
    pub fn new(ttl_secs: u64, max_entries: usize, max_bytes: u64) -> Self {
        Self {
            inner: Mutex::new(Inner {
                entries: HashMap::new(),
                by_hash: HashMap::new(),
                bytes_used: 0,
            }),
            ttl_default: Duration::from_secs(ttl_secs),
            max_entries: max_entries.max(1),
            max_bytes,
            next_id: AtomicU64::new(1),
        }
    }

    /// Store `body` under every identifier in `ids`. If any of the
    /// identifiers already maps to an entry, that prior entry is evicted
    /// first (so an overwrite of the primary key cleanly replaces the
    /// aliases too). The content hash of `body` is appended to the
    /// identifier set automatically — that's what makes single-peer trust
    /// safe for content-addressable sources later.
    ///
    /// Empty / whitespace-only bodies are silently dropped so failed
    /// fetches can be retried.
    pub fn put(&self, ids: &Identifiers, body: &str) {
        if body.trim().is_empty() {
            return;
        }
        let now = Instant::now();
        let ttl = ids
            .source
            .ttl_secs_override()
            .map(Duration::from_secs)
            .unwrap_or(self.ttl_default);
        let expires = now + ttl;

        // Pre-hash every identifier (primary + URL aliases + source-id
        // aliases) plus the content hash. This is what the digest emits and
        // what `lookup` walks; precomputing avoids re-hashing on every read.
        let mut identifier_hashes: Vec<String> =
            ids.iter_capped().map(|r| hash_key(&r.key())).collect();
        let content_hash = hash_bytes(body.as_bytes());
        identifier_hashes.push(format!("cnt:{content_hash}"));

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let entry = Entry {
            body: body.to_string(),
            identifier_hashes: identifier_hashes.clone(),
            expires,
        };

        let body_len = body.len() as u64;
        let mut inner = self.inner.lock().unwrap();

        // Evict any prior entry that any of these identifiers point at. A
        // primary-key overwrite is the common case; URL-alias collisions
        // are rare but possible (two skills cached the same URL under
        // different primary keys — second writer wins).
        let mut already_removed: Vec<EntryId> = Vec::new();
        for h in &identifier_hashes {
            if let Some(&prev_id) = inner.by_hash.get(h) {
                if already_removed.contains(&prev_id) {
                    continue;
                }
                already_removed.push(prev_id);
                if let Some(prev_entry) = inner.entries.remove(&prev_id) {
                    inner.bytes_used = inner
                        .bytes_used
                        .saturating_sub(prev_entry.body.len() as u64);
                    for prev_hash in &prev_entry.identifier_hashes {
                        if inner.by_hash.get(prev_hash) == Some(&prev_id) {
                            inner.by_hash.remove(prev_hash);
                        }
                    }
                }
            }
        }

        // Respect the byte-budget cap. If the body alone is larger than the
        // whole cap, refuse the write *before* evicting anything — nuking
        // the cache for an entry that wouldn't fit anyway is the worst of
        // both worlds.
        if self.max_bytes > 0 && body_len > self.max_bytes {
            return;
        }
        // Respect the entry-count cap. Drop the expired-soonest entry first.
        if inner.entries.len() >= self.max_entries {
            self.evict_one_locked(&mut inner);
        }
        // Then enforce the byte cap by evicting oldest-by-expiry until the
        // new entry fits. `max_bytes == 0` means unlimited.
        if self.max_bytes > 0 {
            while inner.bytes_used + body_len > self.max_bytes && !inner.entries.is_empty() {
                self.evict_one_locked(&mut inner);
            }
        }

        // Install the new entry + all secondary mappings.
        for h in &identifier_hashes {
            inner.by_hash.insert(h.clone(), id);
        }
        inner.bytes_used = inner.bytes_used.saturating_add(body_len);
        inner.entries.insert(id, entry);
    }

    /// Look up by *any* identifier hash. Returns the body if a live entry
    /// is reachable from that hash; expired entries are evicted lazily on
    /// the way out.
    pub fn lookup_by_hash(&self, identifier_hash: &str) -> Option<String> {
        let mut inner = self.inner.lock().unwrap();
        let id = *inner.by_hash.get(identifier_hash)?;
        let expired = inner
            .entries
            .get(&id)
            .map(|e| e.expires <= Instant::now())
            .unwrap_or(true);
        if expired {
            if let Some(e) = inner.entries.remove(&id) {
                inner.bytes_used = inner.bytes_used.saturating_sub(e.body.len() as u64);
                for h in &e.identifier_hashes {
                    if inner.by_hash.get(h) == Some(&id) {
                        inner.by_hash.remove(h);
                    }
                }
            }
            return None;
        }
        inner.entries.get(&id).map(|e| e.body.clone())
    }

    /// Look up by an `Identifiers` set — walks each identifier in turn
    /// until one hits. Used by `Lodestone::retrieval_lookup` so a consumer
    /// that asks by URL finds an entry a peer stored by source-id.
    pub fn lookup(&self, ids: &Identifiers) -> Option<String> {
        for r in ids.iter_capped() {
            if let Some(body) = self.lookup_by_hash(&hash_key(&r.key())) {
                return Some(body);
            }
        }
        None
    }

    /// Identifier hashes for everything currently held (after expiring
    /// stale entries). The constellation digest enumerates this to build
    /// the advertised Bloom.
    pub fn keys(&self) -> Vec<String> {
        let mut inner = self.inner.lock().unwrap();
        let now = Instant::now();
        let stale: Vec<EntryId> = inner
            .entries
            .iter()
            .filter(|(_, e)| e.expires <= now)
            .map(|(id, _)| *id)
            .collect();
        for id in stale {
            if let Some(e) = inner.entries.remove(&id) {
                inner.bytes_used = inner.bytes_used.saturating_sub(e.body.len() as u64);
                for h in &e.identifier_hashes {
                    if inner.by_hash.get(h) == Some(&id) {
                        inner.by_hash.remove(h);
                    }
                }
            }
        }
        inner
            .entries
            .values()
            .flat_map(|e| e.identifier_hashes.iter().cloned())
            .collect()
    }

    /// Drop the entry that expires soonest. Called from `put` when the
    /// entry-count cap or the byte-budget cap fires. Holds the mutex;
    /// caller passes its existing guard. Maintains `bytes_used` on each
    /// removal so the byte-budget check stays accurate.
    fn evict_one_locked(&self, inner: &mut Inner) {
        let now = Instant::now();
        // First try the truly-expired set.
        let expired: Vec<EntryId> = inner
            .entries
            .iter()
            .filter(|(_, e)| e.expires <= now)
            .map(|(id, _)| *id)
            .collect();
        for id in &expired {
            if let Some(e) = inner.entries.remove(id) {
                inner.bytes_used = inner.bytes_used.saturating_sub(e.body.len() as u64);
                for h in &e.identifier_hashes {
                    if inner.by_hash.get(h) == Some(id) {
                        inner.by_hash.remove(h);
                    }
                }
            }
        }
        if !expired.is_empty() {
            return;
        }
        // Nothing expired — drop the one closest to expiry.
        let victim = inner
            .entries
            .iter()
            .min_by_key(|(_, e)| e.expires)
            .map(|(id, _)| *id);
        if let Some(id) = victim {
            if let Some(e) = inner.entries.remove(&id) {
                inner.bytes_used = inner.bytes_used.saturating_sub(e.body.len() as u64);
                for h in &e.identifier_hashes {
                    if inner.by_hash.get(h) == Some(&id) {
                        inner.by_hash.remove(h);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constellation::{identifiers::IdentifierRef, Source};

    fn h(s: &str) -> String {
        hash_key(&IdentifierRef::Primary(s).key())
    }

    fn url_h(u: &str) -> String {
        hash_key(&IdentifierRef::Url(u).key())
    }

    fn sid_h(source: Source, label: &str, value: &str) -> String {
        hash_key(
            &IdentifierRef::SourceId {
                source,
                label,
                value,
            }
            .key(),
        )
    }

    #[test]
    fn put_then_lookup_by_primary() {
        let c = IndexedRetrievalCache::new(60, 16, 0);
        c.put(&Identifiers::new("primary"), "the body");
        assert_eq!(c.lookup_by_hash(&h("primary")).as_deref(), Some("the body"));
    }

    #[test]
    fn put_then_lookup_by_url_alias() {
        let c = IndexedRetrievalCache::new(60, 16, 0);
        let ids = Identifiers::new("primary")
            .with_source(Source::Wayback)
            .with_url("https://example.com/")
            .with_url("https://web.archive.org/web/20240101000000id_/https://example.com/");
        c.put(&ids, "body");
        assert_eq!(
            c.lookup_by_hash(&url_h("https://example.com/")).as_deref(),
            Some("body")
        );
        assert_eq!(
            c.lookup_by_hash(&url_h(
                "https://web.archive.org/web/20240101000000id_/https://example.com/"
            ))
            .as_deref(),
            Some("body")
        );
    }

    #[test]
    fn put_then_lookup_by_source_id() {
        let c = IndexedRetrievalCache::new(60, 16, 0);
        let ids = Identifiers::new("primary")
            .with_source(Source::Wayback)
            .with_source_id("wayback_ts", "20240101000000");
        c.put(&ids, "body");
        assert_eq!(
            c.lookup_by_hash(&sid_h(Source::Wayback, "wayback_ts", "20240101000000"))
                .as_deref(),
            Some("body")
        );
    }

    #[test]
    fn put_then_lookup_by_content_hash() {
        let c = IndexedRetrievalCache::new(60, 16, 0);
        c.put(&Identifiers::new("primary"), "exactly-this-body");
        let ch = hash_bytes(b"exactly-this-body");
        assert_eq!(
            c.lookup_by_hash(&format!("cnt:{ch}")).as_deref(),
            Some("exactly-this-body")
        );
    }

    #[test]
    fn lookup_with_identifiers_finds_via_any_member() {
        let c = IndexedRetrievalCache::new(60, 16, 0);
        // First peer stores under URL.
        let store_ids = Identifiers::new("primary-A")
            .with_source(Source::Wayback)
            .with_url("https://example.com/");
        c.put(&store_ids, "body");

        // Second peer asks by source-id only — must NOT find (different alias set).
        let ask_by_sid = Identifiers::new("different-primary")
            .with_source(Source::Wayback)
            .with_source_id("wayback_ts", "20240101000000");
        assert!(c.lookup(&ask_by_sid).is_none());

        // But asking by URL hits.
        let ask_by_url = Identifiers::new("different-primary")
            .with_source(Source::Wayback)
            .with_url("https://example.com/");
        assert_eq!(c.lookup(&ask_by_url).as_deref(), Some("body"));
    }

    #[test]
    fn empty_body_is_dropped() {
        let c = IndexedRetrievalCache::new(60, 16, 0);
        c.put(&Identifiers::new("k"), "");
        c.put(&Identifiers::new("k2"), "   \n\t  ");
        assert!(c.lookup_by_hash(&h("k")).is_none());
        assert!(c.lookup_by_hash(&h("k2")).is_none());
    }

    #[test]
    fn overwrite_primary_evicts_old_aliases() {
        let c = IndexedRetrievalCache::new(60, 16, 0);
        let v1 = Identifiers::new("primary")
            .with_source(Source::Wayback)
            .with_url("https://old-alias.example.com/");
        c.put(&v1, "v1-body");
        // Same primary key, different URL alias set.
        let v2 = Identifiers::new("primary")
            .with_source(Source::Wayback)
            .with_url("https://new-alias.example.com/");
        c.put(&v2, "v2-body");
        // New alias resolves to new body.
        assert_eq!(
            c.lookup_by_hash(&url_h("https://new-alias.example.com/"))
                .as_deref(),
            Some("v2-body")
        );
        // Old alias is gone — the prior entry was fully removed.
        assert!(c
            .lookup_by_hash(&url_h("https://old-alias.example.com/"))
            .is_none());
    }

    #[test]
    fn expired_entries_disappear_from_keys_and_lookup() {
        let c = IndexedRetrievalCache::new(0, 16, 0); // ttl 0 → already expired by read time
        c.put(&Identifiers::new("k"), "v");
        assert!(c.lookup_by_hash(&h("k")).is_none());
        assert!(c.keys().is_empty());
    }

    #[test]
    fn evicts_to_stay_within_max() {
        let c = IndexedRetrievalCache::new(60, 2, 0);
        c.put(&Identifiers::new("a"), "1");
        c.put(&Identifiers::new("b"), "2");
        c.put(&Identifiers::new("c"), "3"); // forces eviction
        assert_eq!(c.lookup_by_hash(&h("c")).as_deref(), Some("3"));
        // At most one of the originals survives.
        assert!(c.lookup_by_hash(&h("a")).is_none() || c.lookup_by_hash(&h("b")).is_none());
    }

    #[test]
    fn keys_returns_every_identifier_hash() {
        let c = IndexedRetrievalCache::new(60, 16, 0);
        let ids = Identifiers::new("primary")
            .with_source(Source::Wayback)
            .with_url("https://example.com/")
            .with_source_id("wayback_ts", "20240101000000");
        c.put(&ids, "body");
        let keys = c.keys();
        // primary + 1 url + 1 source-id + content hash = 4
        assert_eq!(keys.len(), 4);
        assert!(keys.contains(&h("primary")));
        assert!(keys.contains(&url_h("https://example.com/")));
        assert!(keys.contains(&sid_h(Source::Wayback, "wayback_ts", "20240101000000")));
    }

    #[test]
    fn per_source_ttl_override_is_applied() {
        // Wayback gets the 7-day floor regardless of a 1-second default.
        let c = IndexedRetrievalCache::new(1, 16, 0);
        c.put(
            &Identifiers::new("wayback-entry").with_source(Source::Wayback),
            "still here",
        );
        c.put(&Identifiers::new("other-entry"), "ephemeral");
        std::thread::sleep(Duration::from_millis(1100));
        assert!(c.lookup_by_hash(&h("wayback-entry")).is_some());
        assert!(c.lookup_by_hash(&h("other-entry")).is_none());
    }

    #[test]
    fn primary_only_is_just_primary_plus_content_hash() {
        let c = IndexedRetrievalCache::new(60, 16, 0);
        c.put(&Identifiers::new("k"), "v");
        // primary + content hash = 2
        assert_eq!(c.keys().len(), 2);
    }

    #[test]
    fn byte_budget_evicts_oldest_first_when_cap_exceeded() {
        // 100-byte byte budget; entry-count cap is high so byte budget is
        // the only thing firing.
        let c = IndexedRetrievalCache::new(60, 1000, 100);
        c.put(&Identifiers::new("first"), &"a".repeat(40));
        c.put(&Identifiers::new("second"), &"b".repeat(40));
        // Both fit so far (80 bytes total). Adding a 40-byte third pushes
        // total to 120 — first should evict.
        c.put(&Identifiers::new("third"), &"c".repeat(40));
        assert!(c.lookup_by_hash(&h("first")).is_none());
        assert!(c.lookup_by_hash(&h("second")).is_some());
        assert!(c.lookup_by_hash(&h("third")).is_some());
    }

    #[test]
    fn byte_budget_refuses_oversized_single_body() {
        // 100-byte cap, but the body is 500 bytes. Even evicting everything
        // wouldn't make room — so we refuse the write rather than nuke the
        // cache for an entry that wouldn't fit anyway.
        let c = IndexedRetrievalCache::new(60, 1000, 100);
        c.put(&Identifiers::new("small"), "fits");
        c.put(&Identifiers::new("oversize"), &"x".repeat(500));
        assert!(c.lookup_by_hash(&h("small")).is_some());
        assert!(c.lookup_by_hash(&h("oversize")).is_none());
    }

    #[test]
    fn byte_budget_zero_means_unlimited() {
        // 0 = no byte cap; only max_entries fires. Bodies are made distinct
        // so the content-hash identifier doesn't dedupe them into one entry.
        let c = IndexedRetrievalCache::new(60, 16, 0);
        for i in 0..10 {
            c.put(
                &Identifiers::new(format!("k{i}")),
                &format!("entry-{i}-{}", "x".repeat(1_000_000)),
            );
        }
        for i in 0..10 {
            assert!(c.lookup_by_hash(&h(&format!("k{i}"))).is_some());
        }
    }
}

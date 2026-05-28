//! A tiny in-memory TTL cache for search results.
//!
//! Repeated identical searches otherwise re-hit rate-limited engines and burn
//! the StackExchange/GitHub quota; caching the normalized result list for a short
//! TTL makes bursts and retries cheap. It is process-local (cleared on restart)
//! and stores only small, serializable values (JSON result lists) — never
//! secrets. This is the in-memory backend; a shared Redis backend can implement
//! the same get/put contract later.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A size-bounded, time-to-live string cache. Values are opaque to the cache
/// (callers serialize/deserialize their own types).
pub struct TtlCache {
    ttl: Duration,
    max: usize,
    map: Mutex<HashMap<String, Entry>>,
}

struct Entry {
    value: String,
    expires: Instant,
}

impl TtlCache {
    /// `ttl_secs` is the lifetime of each entry; `max_entries` bounds memory.
    pub fn new(ttl_secs: u64, max_entries: usize) -> Self {
        Self {
            ttl: Duration::from_secs(ttl_secs),
            max: max_entries.max(1),
            map: Mutex::new(HashMap::new()),
        }
    }

    /// Return a live (unexpired) value for `key`, evicting it if it has expired.
    pub fn get(&self, key: &str) -> Option<String> {
        let mut map = self.map.lock().unwrap();
        match map.get(key) {
            Some(e) if e.expires > Instant::now() => Some(e.value.clone()),
            Some(_) => {
                map.remove(key);
                None
            }
            None => None,
        }
    }

    /// Insert/overwrite `key` with a fresh TTL. When at capacity, expired entries
    /// are dropped first, then an arbitrary entry if still full.
    pub fn put(&self, key: String, value: String) {
        let mut map = self.map.lock().unwrap();
        let now = Instant::now();
        if map.len() >= self.max && !map.contains_key(&key) {
            map.retain(|_, e| e.expires > now);
            if map.len() >= self.max {
                if let Some(victim) = map.keys().next().cloned() {
                    map.remove(&victim);
                }
            }
        }
        map.insert(
            key,
            Entry {
                value,
                expires: now + self.ttl,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_and_returns_live_values() {
        let c = TtlCache::new(60, 16);
        c.put("k".into(), "v".into());
        assert_eq!(c.get("k").as_deref(), Some("v"));
        assert_eq!(c.get("missing"), None);
    }

    #[test]
    fn expired_entries_are_not_returned() {
        let c = TtlCache::new(0, 16); // ttl 0 → already expired by read time
        c.put("k".into(), "v".into());
        assert_eq!(c.get("k"), None);
    }

    #[test]
    fn evicts_to_stay_within_max() {
        let c = TtlCache::new(60, 2);
        c.put("a".into(), "1".into());
        c.put("b".into(), "2".into());
        c.put("c".into(), "3".into()); // forces an eviction
        assert_eq!(c.get("c").as_deref(), Some("3"));
        // at most one of the originals survives under a cap of 2
        assert!(c.get("a").is_none() || c.get("b").is_none());
    }
}

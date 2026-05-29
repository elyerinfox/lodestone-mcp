//! A small TTL cache for search/retrieval results, with two interchangeable
//! backends behind one `get`/`put`/`keys` contract:
//!
//! * **Memory** (default) — a process-local, size-bounded map. Cleared on restart.
//! * **Redis** (`[cache].backend = "redis"`) — a shared store several lodestone
//!   instances can point at, so a result cached by one is served by all. Keys are
//!   namespaced by a per-store `prefix`; values expire via Redis `SET … EX`.
//!
//! Repeated identical searches otherwise re-hit rate-limited engines and burn the
//! StackExchange/GitHub quota. Only small, serializable values (JSON result lists,
//! page text) are cached — never secrets.
//!
//! The public API is synchronous so call sites stay simple. The Redis backend
//! bridges its async client via `block_in_place` + the runtime handle; that briefly
//! occupies a worker thread for the (typically sub-millisecond, localhost/LAN)
//! round-trip, which is an acceptable trade for not threading `async` through every
//! cache touch. The memory backend does no blocking.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A size-bounded, time-to-live string cache. Values are opaque to the cache
/// (callers serialize/deserialize their own types).
pub struct TtlCache {
    backend: Backend,
}

enum Backend {
    Memory(Memory),
    // Boxed: a `ConnectionManager` is much larger than the memory variant.
    Redis(Box<RedisCache>),
}

impl TtlCache {
    /// `ttl_secs` is the lifetime of each entry; `max_entries` bounds memory.
    /// Builds the in-memory backend.
    pub fn new(ttl_secs: u64, max_entries: usize) -> Self {
        Self {
            backend: Backend::Memory(Memory::new(ttl_secs, max_entries)),
        }
    }

    /// Connect to a Redis store as the backend. `prefix` namespaces this cache's
    /// keys so multiple caches (search, retrieval) can share one Redis DB without
    /// colliding. Must be called from within a Tokio runtime.
    pub async fn connect_redis(
        url: &str,
        ttl_secs: u64,
        prefix: &str,
    ) -> Result<Self, redis::RedisError> {
        let client = redis::Client::open(url)?;
        let mgr = client.get_connection_manager().await?;
        Ok(Self {
            backend: Backend::Redis(Box::new(RedisCache {
                mgr,
                handle: tokio::runtime::Handle::current(),
                ttl_secs: ttl_secs.max(1),
                prefix: prefix.to_string(),
            })),
        })
    }

    /// Return a live (unexpired) value for `key`.
    pub fn get(&self, key: &str) -> Option<String> {
        match &self.backend {
            Backend::Memory(m) => m.get(key),
            Backend::Redis(r) => r.get(key),
        }
    }

    /// Insert/overwrite `key` with a fresh TTL.
    pub fn put(&self, key: String, value: String) {
        match &self.backend {
            Backend::Memory(m) => m.put(key, value),
            Backend::Redis(r) => r.put(&key, &value),
        }
    }

    /// Snapshot of the currently-live (unexpired) keys. Used by the constellation to
    /// build a digest of what this node can serve.
    pub fn keys(&self) -> Vec<String> {
        match &self.backend {
            Backend::Memory(m) => m.keys(),
            Backend::Redis(r) => r.keys(),
        }
    }
}

// ---------------------------------------------------------------------------
// In-memory backend
// ---------------------------------------------------------------------------

struct Memory {
    ttl: Duration,
    max: usize,
    map: Mutex<HashMap<String, Entry>>,
}

struct Entry {
    value: String,
    expires: Instant,
}

impl Memory {
    fn new(ttl_secs: u64, max_entries: usize) -> Self {
        Self {
            ttl: Duration::from_secs(ttl_secs),
            max: max_entries.max(1),
            map: Mutex::new(HashMap::new()),
        }
    }

    fn get(&self, key: &str) -> Option<String> {
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

    fn keys(&self) -> Vec<String> {
        let now = Instant::now();
        let map = self.map.lock().unwrap();
        map.iter()
            .filter(|(_, e)| e.expires > now)
            .map(|(k, _)| k.clone())
            .collect()
    }

    fn put(&self, key: String, value: String) {
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

// ---------------------------------------------------------------------------
// Redis backend (shared store)
// ---------------------------------------------------------------------------

struct RedisCache {
    mgr: redis::aio::ConnectionManager,
    handle: tokio::runtime::Handle,
    ttl_secs: u64,
    prefix: String,
}

impl RedisCache {
    fn full_key(&self, key: &str) -> String {
        format!("{}{}", self.prefix, key)
    }

    /// Drive an async Redis op to completion from this synchronous API. Safe on the
    /// multi-thread runtime: `block_in_place` lets other tasks proceed on other
    /// workers while this one waits on the (short) round-trip.
    fn block_on<F: std::future::Future>(&self, fut: F) -> F::Output {
        tokio::task::block_in_place(|| self.handle.block_on(fut))
    }

    fn get(&self, key: &str) -> Option<String> {
        use redis::AsyncCommands;
        let full = self.full_key(key);
        let mut conn = self.mgr.clone();
        let res: redis::RedisResult<Option<String>> =
            self.block_on(async { conn.get(&full).await });
        match res {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, "redis cache get failed");
                None
            }
        }
    }

    fn put(&self, key: &str, value: &str) {
        use redis::AsyncCommands;
        let full = self.full_key(key);
        let ttl = self.ttl_secs;
        let mut conn = self.mgr.clone();
        let res: redis::RedisResult<()> =
            self.block_on(async { conn.set_ex(&full, value, ttl).await });
        if let Err(e) = res {
            tracing::warn!(error = %e, "redis cache put failed");
        }
    }

    fn keys(&self) -> Vec<String> {
        use redis::AsyncCommands;
        let pattern = format!("{}*", self.prefix);
        let mut conn = self.mgr.clone();
        let res: redis::RedisResult<Vec<String>> =
            self.block_on(async { conn.keys(&pattern).await });
        match res {
            Ok(keys) => keys
                .into_iter()
                .map(|k| k.strip_prefix(&self.prefix).map(String::from).unwrap_or(k))
                .collect(),
            Err(e) => {
                tracing::warn!(error = %e, "redis cache keys failed");
                Vec::new()
            }
        }
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

//! An on-disk, key-addressed file store for fetched bytes (repo files, PDFs,
//! rendered pages) so they can be reused across calls without re-downloading.
//!
//! Each entry is two files under the store directory: `<hash>.data` (the bytes) and
//! `<hash>.key` (the original key, e.g. the URL, so listings are human-readable),
//! where `<hash>` is [`crate::constellation::hash_key`] of the key. Retention is enforced on
//! write: entries older than the TTL are dropped, then the oldest are evicted until
//! the total is under the byte budget. Off by default — enabled via `[store]`.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result};

/// One stored entry's metadata (for listings).
pub struct StoreEntry {
    pub key: String,
    pub size: u64,
    pub modified: SystemTime,
}

/// A bounded, TTL'd on-disk byte store.
pub struct FileStore {
    dir: PathBuf,
    max_bytes: u64,
    ttl: Duration,
}

impl FileStore {
    /// Open (creating it if needed) a store at `dir`. `ttl_secs == 0` disables
    /// expiry; `max_bytes == 0` disables the size cap.
    pub async fn open(dir: &str, max_bytes: u64, ttl_secs: u64) -> Result<Self> {
        let dir = if dir.trim().is_empty() {
            PathBuf::from(".lodestone-store")
        } else {
            PathBuf::from(dir.trim())
        };
        tokio::fs::create_dir_all(&dir)
            .await
            .with_context(|| format!("creating file store at '{}'", dir.display()))?;
        Ok(Self {
            dir,
            max_bytes,
            ttl: Duration::from_secs(ttl_secs),
        })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The on-disk path an entry for `key` is (or would be) stored at.
    pub fn path_for(&self, key: &str) -> PathBuf {
        self.data_path(key)
    }

    fn data_path(&self, key: &str) -> PathBuf {
        self.dir
            .join(format!("{}.data", crate::constellation::hash_key(key)))
    }

    fn key_path(&self, key: &str) -> PathBuf {
        self.dir
            .join(format!("{}.key", crate::constellation::hash_key(key)))
    }

    fn expired(&self, modified: SystemTime) -> bool {
        if self.ttl.is_zero() {
            return false;
        }
        modified
            .elapsed()
            .map(|age| age > self.ttl)
            .unwrap_or(false)
    }

    /// Fetch a fresh entry's bytes, or `None` if missing/expired.
    pub async fn get(&self, key: &str) -> Option<Vec<u8>> {
        let path = self.data_path(key);
        let meta = tokio::fs::metadata(&path).await.ok()?;
        if let Ok(modified) = meta.modified() {
            if self.expired(modified) {
                let _ = self.remove(key).await;
                return None;
            }
        }
        tokio::fs::read(&path).await.ok()
    }

    /// Read a fresh entry by its already-hashed key (the filename stem). Used by the
    /// constellation, which addresses entries by hash over the wire (never the raw key).
    pub async fn get_by_hash(&self, hash: &str) -> Option<Vec<u8>> {
        // Guard against path tricks: a hash is hex only.
        if hash.is_empty() || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let path = self.dir.join(format!("{hash}.data"));
        let meta = tokio::fs::metadata(&path).await.ok()?;
        if let Ok(modified) = meta.modified() {
            if self.expired(modified) {
                return None;
            }
        }
        tokio::fs::read(&path).await.ok()
    }

    /// Hashes (filename stems) of the fresh entries — what this node can serve to
    /// peers. Fed into the constellation digest's Bloom filter.
    pub async fn hashes(&self) -> Vec<String> {
        let mut out = Vec::new();
        let Ok(mut rd) = tokio::fs::read_dir(&self.dir).await else {
            return out;
        };
        while let Ok(Some(e)) = rd.next_entry().await {
            let path = e.path();
            if path.extension().is_none_or(|x| x != "data") {
                continue;
            }
            let Ok(meta) = e.metadata().await else {
                continue;
            };
            let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            if self.expired(modified) {
                continue;
            }
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                out.push(stem.to_string());
            }
        }
        out
    }

    /// Store `bytes` under `key`, returning the data-file path. Enforces retention.
    pub async fn put(&self, key: &str, bytes: &[u8]) -> Result<PathBuf> {
        let data = self.data_path(key);
        tokio::fs::write(&data, bytes)
            .await
            .with_context(|| format!("writing store entry '{}'", data.display()))?;
        tokio::fs::write(self.key_path(key), key.as_bytes())
            .await
            .ok();
        self.prune().await;
        Ok(data)
    }

    /// Remove one entry (both files). Returns true if the data file existed.
    pub async fn remove(&self, key: &str) -> bool {
        let existed = tokio::fs::remove_file(self.data_path(key)).await.is_ok();
        let _ = tokio::fs::remove_file(self.key_path(key)).await;
        existed
    }

    /// Remove every entry; returns how many data files were deleted.
    pub async fn purge(&self) -> usize {
        let mut removed = 0;
        if let Ok(mut rd) = tokio::fs::read_dir(&self.dir).await {
            while let Ok(Some(e)) = rd.next_entry().await {
                let p = e.path();
                let is_data = p.extension().is_some_and(|x| x == "data");
                if tokio::fs::remove_file(&p).await.is_ok() && is_data {
                    removed += 1;
                }
            }
        }
        removed
    }

    /// All current entries (newest first), reading the `.key` sidecars for readable keys.
    pub async fn list(&self) -> Vec<StoreEntry> {
        let mut out = Vec::new();
        let Ok(mut rd) = tokio::fs::read_dir(&self.dir).await else {
            return out;
        };
        while let Ok(Some(e)) = rd.next_entry().await {
            let path = e.path();
            if path.extension().is_none_or(|x| x != "data") {
                continue;
            }
            let meta = match e.metadata().await {
                Ok(m) => m,
                Err(_) => continue,
            };
            let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            // The sibling `.key` file holds the original key (URL).
            let key_file = path.with_extension("key");
            let key = tokio::fs::read_to_string(&key_file)
                .await
                .unwrap_or_else(|_| {
                    path.file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into()
                });
            out.push(StoreEntry {
                key,
                size: meta.len(),
                modified,
            });
        }
        out.sort_by_key(|e| std::cmp::Reverse(e.modified));
        out
    }

    /// Drop expired entries, then evict the oldest until under the byte budget.
    async fn prune(&self) {
        let mut entries: Vec<(PathBuf, u64, SystemTime)> = Vec::new();
        let Ok(mut rd) = tokio::fs::read_dir(&self.dir).await else {
            return;
        };
        while let Ok(Some(e)) = rd.next_entry().await {
            let path = e.path();
            if path.extension().is_none_or(|x| x != "data") {
                continue;
            }
            let Ok(meta) = e.metadata().await else {
                continue;
            };
            let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            if self.expired(modified) {
                let _ = tokio::fs::remove_file(&path).await;
                let _ = tokio::fs::remove_file(path.with_extension("key")).await;
                continue;
            }
            entries.push((path, meta.len(), modified));
        }
        if self.max_bytes == 0 {
            return;
        }
        let mut total: u64 = entries.iter().map(|(_, s, _)| *s).sum();
        if total <= self.max_bytes {
            return;
        }
        entries.sort_by_key(|e| e.2); // oldest first
        for (path, size, _) in entries {
            if total <= self.max_bytes {
                break;
            }
            if tokio::fs::remove_file(&path).await.is_ok() {
                let _ = tokio::fs::remove_file(path.with_extension("key")).await;
                total = total.saturating_sub(size);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn roundtrip_by_key_and_hash() {
        let dir = std::env::temp_dir().join(format!("lode-store-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&dir).await;
        let store = FileStore::open(dir.to_str().unwrap(), 0, 0).await.unwrap();

        let url = "https://arxiv.org/pdf/1706.03762";
        store.put(url, b"PDFDATA").await.unwrap();

        // Local read by raw key.
        assert_eq!(store.get(url).await.as_deref(), Some(&b"PDFDATA"[..]));
        // Constellation reads by hash (the filename stem) — must resolve to the same bytes.
        let h = crate::constellation::hash_key(url);
        assert_eq!(
            store.get_by_hash(&h).await.as_deref(),
            Some(&b"PDFDATA"[..])
        );
        // The digest advertises that hash.
        assert!(store.hashes().await.contains(&h));
        // Path-traversal / non-hex keys are rejected.
        assert!(store.get_by_hash("../etc/passwd").await.is_none());
        assert!(store.get_by_hash("not a hash").await.is_none());

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
}

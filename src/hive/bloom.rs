//! A small Bloom filter and a stable content hash, both built on FNV-1a so their
//! output is identical across processes and versions (unlike std's
//! `DefaultHasher`, whose SipHash keys are randomized per process). The hivemind
//! uses these to advertise *which* query keys a node has cached — as a compact
//! filter of hashes — without revealing the queries themselves.

use serde::{Deserialize, Serialize};

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
/// A second seed (golden-ratio constant) for an independent FNV pass.
const SEED2: u64 = 0x9e37_79b9_7f4a_7c15;

/// FNV-1a 64-bit over `data`, seeded by `offset` so we can derive independent
/// hashes from the same bytes.
fn fnv1a(data: &[u8], offset: u64) -> u64 {
    let mut h = offset;
    for &b in data {
        h ^= b as u64;
        h = h.wrapping_mul(FNV_PRIME);
    }
    h
}

/// Stable 128-bit content hash as 32 lowercase hex chars (two independent FNV-1a
/// passes). Deterministic across machines, so peers compute the same key for the
/// same query — this is what crosses the wire instead of the raw query text.
pub(crate) fn hash_key(s: &str) -> String {
    let h1 = fnv1a(s.as_bytes(), FNV_OFFSET);
    let h2 = fnv1a(s.as_bytes(), FNV_OFFSET ^ SEED2);
    format!("{h1:016x}{h2:016x}")
}

/// A Bloom filter with a JSON-friendly wire form. `m` is the bit count, `k` the
/// number of probes; bits are packed into `Vec<u64>` words.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct BloomFilter {
    pub m: u64,
    pub k: u32,
    pub bits: Vec<u64>,
}

impl BloomFilter {
    /// Size the filter for `expected` items at ~1% false-positive rate, clamped
    /// to sane bounds (64 bits .. 4 Mbit, 1..16 probes).
    pub fn for_capacity(expected: usize) -> Self {
        let n = expected.max(1) as f64;
        let p = 0.01_f64;
        let m = (-(n * p.ln()) / std::f64::consts::LN_2.powi(2)).ceil() as u64;
        let m = m.clamp(64, 1 << 22);
        let k = ((m as f64 / n) * std::f64::consts::LN_2).round() as u32;
        let k = k.clamp(1, 16);
        let words = m.div_ceil(64) as usize;
        Self {
            m,
            k,
            bits: vec![0u64; words],
        }
    }

    /// Build a filter populated from `keys`.
    pub fn from_keys(keys: &[String]) -> Self {
        let mut bf = Self::for_capacity(keys.len());
        for key in keys {
            bf.insert(key);
        }
        bf
    }

    fn indices(&self, key: &str) -> impl Iterator<Item = u64> {
        let h1 = fnv1a(key.as_bytes(), FNV_OFFSET);
        // Force odd so the step is coprime with powers of two (better spread).
        let h2 = fnv1a(key.as_bytes(), FNV_OFFSET ^ SEED2) | 1;
        let (m, k) = (self.m, self.k);
        (0..k as u64).map(move |i| h1.wrapping_add(i.wrapping_mul(h2)) % m)
    }

    pub fn insert(&mut self, key: &str) {
        for idx in self.indices(key) {
            let (w, b) = ((idx / 64) as usize, idx % 64);
            if let Some(word) = self.bits.get_mut(w) {
                *word |= 1u64 << b;
            }
        }
    }

    pub fn maybe_contains(&self, key: &str) -> bool {
        if self.m == 0 || self.bits.is_empty() {
            return false;
        }
        for idx in self.indices(key) {
            let (w, b) = ((idx / 64) as usize, idx % 64);
            match self.bits.get(w) {
                Some(word) if word & (1u64 << b) != 0 => {}
                _ => return false,
            }
        }
        true
    }

    /// Structural sanity check for a filter received from a peer (defends against
    /// malformed/oversized payloads).
    pub fn is_valid(&self) -> bool {
        self.m >= 1
            && self.k >= 1
            && self.k <= 64
            && self.m <= (1 << 24)
            && self.bits.len() == self.m.div_ceil(64) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_key_is_deterministic_and_distinct() {
        assert_eq!(
            hash_key("search|web|fallback|q"),
            hash_key("search|web|fallback|q")
        );
        assert_ne!(hash_key("a"), hash_key("b"));
        assert_eq!(hash_key("anything").len(), 32);
    }

    #[test]
    fn no_false_negatives() {
        let keys: Vec<String> = (0..200).map(|i| format!("key-{i}")).collect();
        let bf = BloomFilter::from_keys(&keys);
        for k in &keys {
            assert!(bf.maybe_contains(k), "missing inserted key {k}");
        }
    }

    #[test]
    fn empty_filter_contains_nothing() {
        let bf = BloomFilter::from_keys(&[]);
        assert!(!bf.maybe_contains("anything"));
    }

    #[test]
    fn survives_json_round_trip() {
        let keys: Vec<String> = (0..50).map(|i| format!("k{i}")).collect();
        let bf = BloomFilter::from_keys(&keys);
        let json = serde_json::to_string(&bf).unwrap();
        let back: BloomFilter = serde_json::from_str(&json).unwrap();
        assert!(back.is_valid());
        for k in &keys {
            assert!(back.maybe_contains(k));
        }
    }
}

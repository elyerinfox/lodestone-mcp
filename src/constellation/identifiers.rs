//! Per-cache-entry **identifiers** and the **source** classifier that drives
//! per-source caching / consensus policy.
//!
//! A retrieval cache entry today is reachable by exactly one canonical key (the
//! hash of a per-skill string like `wayback|max|ts|<url>`). The mesh can only
//! serve a consumer that asks by the *same* canonical key, which is why
//! long-tail rate-limited content (a specific arXiv paper, a specific Wayback
//! snapshot) breaks the consensus floor: usually zero or one peer has it
//! under that exact key.
//!
//! [`Identifiers`] closes that alignment gap. An entry can declare every public
//! name it's known by — the canonical key *plus* one or more URLs (raw,
//! redirected, snapshot-rewritten, …) *plus* a small map of source-specific
//! identifiers (arXiv id, DOI, GitHub release tag, Wayback `(url, timestamp)`,
//! …). The constellation digest emits **all** of those identifier hashes so a
//! peer that asks by *any* of them gets a Bloom hit.
//!
//! [`Source`] classifies the kind of content the entry holds; that classifier
//! drives the per-source TTL override and the `min_agreement` floor used by
//! `consult_blob_hash`. Content-addressable sources (Wayback, arXiv by
//! `{id}+v`, GitHub releases by tag) can safely be single-peer-trusted because
//! the consumer can verify the bytes against the source-specific identifier;
//! volatile / non-content-addressable sources (search engines, Overpass) keep
//! the existing multi-peer corroboration requirement.
//!
//! See [`docs/constellation.md`](../../docs/constellation.md) for the design
//! motivation; the per-source policy table is the canonical reference.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Maximum number of identifiers stored / emitted per entry. Caps the digest
/// Bloom growth at a small constant per cached entry: a 1000-entry digest
/// stays well under 10 KB even at the cap.
pub const MAX_IDENTIFIERS_PER_ENTRY: usize = 8;

/// The kind of upstream that produced a cache entry. Drives per-source TTL and
/// the consensus floor for trusting peer-served bytes.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// Internet Archive Wayback Machine snapshots. Content is identified by
    /// `(url, timestamp)` and immutable forever; a single peer is enough.
    Wayback,
    /// arXiv papers, addressed by `{id}+v` (e.g. `2103.00020v1`). Immutable per
    /// version; metadata can drift between versions.
    Arxiv,
    /// GitHub releases addressed by repo + tag. Immutable per tag.
    Github,
    /// OpenStreetMap Overpass results. World data, changes slowly.
    Overpass,
    /// HTML-scraping search engine responses (DuckDuckGo, Mojeek, …). Volatile
    /// — providers update constantly.
    SearchEngine,
    /// Fallback — uses the global cache TTL and the existing global
    /// `min_agreement`. Existing call sites that don't classify their content
    /// land here so nothing changes for them.
    #[default]
    Other,
}

impl Source {
    /// Minimum number of peers that must corroborate a content hash before
    /// their bytes are trusted. For content-addressable sources (Wayback,
    /// arXiv, GitHub releases) a single peer is safe — the *consumer* can
    /// independently verify the bytes against the source-specific identifier
    /// the entry already carries (the snapshot timestamp, the arXiv id, the
    /// release tag), so a malicious peer can't substitute different content
    /// without changing the identifier (which would change the hash the
    /// consumer asked for in the first place).
    ///
    /// For volatile sources (Overpass, search engines) the existing
    /// multi-peer corroboration floor stays — the bytes aren't recoverable
    /// from the identifier alone, so we need agreement.
    ///
    /// Returns the per-source floor; the caller picks the **max** of this
    /// and the global `min_agreement` config so a user that sets
    /// `min_agreement = 3` cluster-wide isn't silently relaxed to 1 for
    /// Wayback.
    pub fn min_agreement_floor(self) -> usize {
        match self {
            Self::Wayback | Self::Arxiv | Self::Github => 1,
            Self::Overpass => 2,
            Self::SearchEngine => 2,
            Self::Other => 0,
        }
    }

    /// Per-source TTL override in seconds. `None` falls back to the global
    /// `[cache].ttl_secs`. Wayback / arXiv / GitHub-release entries are
    /// effectively immutable so a week-long TTL is conservative; Overpass
    /// world data changes slowly enough that a day fits; search engine
    /// responses stay on a one-hour leash because providers update fast.
    pub fn ttl_secs_override(self) -> Option<u64> {
        match self {
            Self::Wayback | Self::Arxiv | Self::Github => Some(7 * 24 * 3600),
            Self::Overpass => Some(24 * 3600),
            Self::SearchEngine => Some(3600),
            Self::Other => None,
        }
    }

    /// A short stable lowercase label used as a namespace prefix when
    /// hashing `source_ids` entries (so `("wayback_ts", "20240315120000")`
    /// can't collide with `("arxiv", "20240315120000")` as keys). The label
    /// is internal — never serialised into a tool response.
    pub(crate) fn ns(self) -> &'static str {
        match self {
            Self::Wayback => "wayback",
            Self::Arxiv => "arxiv",
            Self::Github => "github",
            Self::Overpass => "overpass",
            Self::SearchEngine => "search",
            Self::Other => "other",
        }
    }
}

/// Every public name a cache entry is known by. The constellation digest
/// emits the hash of each of these so any of them suffices for a peer to
/// find the entry.
///
/// Build with [`Identifiers::new`] (single-key migration path) or the
/// per-source helpers (`Identifiers::wayback`, etc.) so each adopter site
/// reads as "this is a Wayback snapshot, identified by (url, ts)" rather
/// than as a free-form bag of strings.
#[derive(Debug, Clone)]
pub struct Identifiers {
    /// The skill's pre-existing canonical cache key (still the entry's
    /// primary handle — the URL / source-id hashes are *aliases*).
    pub primary_key: String,
    /// The source classifier — drives TTL + min_agreement policy.
    pub source: Source,
    /// Every URL that resolves to this body — raw URL, snapshot URL,
    /// redirected URL, alternate-host URL. Order doesn't matter; the cache
    /// hashes each independently.
    pub urls: Vec<String>,
    /// Source-specific identifiers: `("arxiv", "1706.03762v5")`,
    /// `("doi", "10.48550/arXiv.1706.03762")`, `("wayback_ts", "20240315…")`.
    /// Each `(label, value)` is namespaced with `source.ns()` before hashing
    /// to keep different sources' id-spaces from colliding.
    pub source_ids: BTreeMap<String, String>,
}

impl Identifiers {
    /// Bare-key constructor — what the existing `retrieval_put(key, …)` call
    /// sites collapse into when they migrate, so existing behavior is
    /// preserved without per-site changes.
    pub fn new(primary_key: impl Into<String>) -> Self {
        Self {
            primary_key: primary_key.into(),
            source: Source::Other,
            urls: Vec::new(),
            source_ids: BTreeMap::new(),
        }
    }

    /// Set the source classifier.
    pub fn with_source(mut self, source: Source) -> Self {
        self.source = source;
        self
    }

    /// Add a URL alias. Empty / whitespace-only URLs are ignored. Duplicates
    /// (after canonicalising via [`crate::provider::normalize_url`] if
    /// callers want exact-dedup) are kept as separate entries — the
    /// constellation digest will dedup them downstream by hash.
    pub fn with_url(mut self, url: impl Into<String>) -> Self {
        let s = url.into();
        if !s.trim().is_empty() {
            self.urls.push(s);
        }
        self
    }

    /// Add a `(label, value)` source-specific identifier. Empty values are
    /// ignored. Calling twice with the same label overwrites.
    pub fn with_source_id(mut self, label: impl Into<String>, value: impl Into<String>) -> Self {
        let l = label.into();
        let v = value.into();
        if !l.trim().is_empty() && !v.trim().is_empty() {
            self.source_ids.insert(l, v);
        }
        self
    }

    /// True if this entry holds only a primary key — i.e. it's a plain
    /// single-key entry equivalent to the legacy `retrieval_put(key, ...)`
    /// path. The cache uses this to skip secondary-index work for the
    /// migration shim; tests use it to assert the shape after a builder
    /// chain.
    #[allow(dead_code)] // exercised by the identifiers + retrieval tests
    pub fn is_primary_only(&self) -> bool {
        self.urls.is_empty() && self.source_ids.is_empty()
    }

    /// Iterate every identifier as an `(IdentifierKind, &str)` pair so the
    /// caller can hash each and emit a digest entry. The order is:
    /// primary key, then URLs in insertion order, then source-ids in
    /// `BTreeMap` order. Capped at [`MAX_IDENTIFIERS_PER_ENTRY`]; excess
    /// identifiers are dropped at iteration time, not at construction, so
    /// callers can build a richer struct and let the cap fire deterministically.
    pub fn iter_capped(&self) -> impl Iterator<Item = IdentifierRef<'_>> {
        let primary = std::iter::once(IdentifierRef::Primary(&self.primary_key));
        let urls = self.urls.iter().map(|u| IdentifierRef::Url(u));
        let source_ids = self
            .source_ids
            .iter()
            .map(|(label, value)| IdentifierRef::SourceId {
                source: self.source,
                label,
                value,
            });
        primary
            .chain(urls)
            .chain(source_ids)
            .take(MAX_IDENTIFIERS_PER_ENTRY)
    }
}

/// One identifier as seen during iteration. `Primary` is the canonical key;
/// `Url` is one URL alias; `SourceId` is one `(source, label, value)` triple
/// that gets namespaced via [`Source::ns`] before hashing so different
/// sources can't collide on identical `(label, value)` pairs.
#[derive(Debug)]
pub enum IdentifierRef<'a> {
    Primary(&'a str),
    Url(&'a str),
    SourceId {
        source: Source,
        label: &'a str,
        value: &'a str,
    },
}

impl<'a> IdentifierRef<'a> {
    /// The pre-hash string for this identifier. Caller passes this to
    /// `bloom::hash_key` to obtain the digest entry. The strings encode the
    /// identifier *kind* so a `Url("https://foo")` and a
    /// `SourceId{ value: "https://foo", … }` deliberately hash to different
    /// slots — they mean different things and shouldn't accidentally collide.
    pub fn key(&self) -> String {
        match self {
            Self::Primary(k) => (*k).to_string(),
            Self::Url(u) => format!("url:{u}"),
            Self::SourceId {
                source,
                label,
                value,
            } => format!("sid:{}:{label}:{value}", source.ns()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn min_agreement_floor_table() {
        assert_eq!(Source::Wayback.min_agreement_floor(), 1);
        assert_eq!(Source::Arxiv.min_agreement_floor(), 1);
        assert_eq!(Source::Github.min_agreement_floor(), 1);
        assert_eq!(Source::Overpass.min_agreement_floor(), 2);
        assert_eq!(Source::SearchEngine.min_agreement_floor(), 2);
        assert_eq!(Source::Other.min_agreement_floor(), 0);
    }

    #[test]
    fn ttl_override_table() {
        assert_eq!(Source::Wayback.ttl_secs_override(), Some(7 * 24 * 3600));
        assert_eq!(Source::Overpass.ttl_secs_override(), Some(24 * 3600));
        assert_eq!(Source::SearchEngine.ttl_secs_override(), Some(3600));
        assert_eq!(Source::Other.ttl_secs_override(), None);
    }

    #[test]
    fn builder_preserves_order_and_dedup_behaviour() {
        let ids = Identifiers::new("primary")
            .with_source(Source::Wayback)
            .with_url("https://example.com/")
            .with_url("https://web.archive.org/web/20240315120000id_/https://example.com/")
            .with_source_id("wayback_ts", "20240315120000");
        assert_eq!(ids.urls.len(), 2);
        assert_eq!(ids.source_ids.len(), 1);
        // primary + 2 urls + 1 source-id = 4 identifiers
        assert_eq!(ids.iter_capped().count(), 4);
        assert!(!ids.is_primary_only());
    }

    #[test]
    fn empty_url_and_source_id_are_ignored() {
        let ids = Identifiers::new("primary")
            .with_url("")
            .with_url("   ")
            .with_source_id("", "x")
            .with_source_id("y", "");
        assert!(ids.urls.is_empty());
        assert!(ids.source_ids.is_empty());
        assert!(ids.is_primary_only());
    }

    #[test]
    fn iter_capped_respects_max() {
        let mut ids = Identifiers::new("primary").with_source(Source::Wayback);
        for i in 0..32 {
            ids = ids.with_url(format!("https://host{i}/"));
        }
        let count = ids.iter_capped().count();
        assert_eq!(count, MAX_IDENTIFIERS_PER_ENTRY);
    }

    #[test]
    fn iter_capped_yields_primary_first() {
        let ids = Identifiers::new("primary-key")
            .with_source(Source::Arxiv)
            .with_url("https://arxiv.org/abs/1706.03762v5")
            .with_source_id("arxiv", "1706.03762v5");
        let mut it = ids.iter_capped();
        match it.next().unwrap() {
            IdentifierRef::Primary(k) => assert_eq!(k, "primary-key"),
            other => panic!("expected Primary, got {other:?}"),
        }
    }

    #[test]
    fn source_id_key_namespaces_by_source() {
        let r1 = IdentifierRef::SourceId {
            source: Source::Wayback,
            label: "ts",
            value: "20240315",
        };
        let r2 = IdentifierRef::SourceId {
            source: Source::Arxiv,
            label: "ts",
            value: "20240315",
        };
        assert_ne!(
            r1.key(),
            r2.key(),
            "same (label,value) under different sources must hash differently"
        );
    }

    #[test]
    fn primary_only_round_trip() {
        let ids = Identifiers::new("just-a-key");
        assert!(ids.is_primary_only());
        assert_eq!(ids.iter_capped().count(), 1);
    }

    #[test]
    fn wayback_pattern_builds_a_two_url_set_when_snapshot_added() {
        // The pattern used by the Wayback skill adopter: raw URL goes on
        // first, snapshot URL gets attached after the snapshot resolves,
        // timestamp is the source-id.
        let ids = Identifiers::new("primary")
            .with_source(Source::Wayback)
            .with_url("https://example.com/")
            .with_url("https://web.archive.org/web/20240315120000id_/https://example.com/")
            .with_source_id("wayback_ts", "20240315120000");
        assert_eq!(ids.source, Source::Wayback);
        assert_eq!(ids.urls.len(), 2);
        assert_eq!(
            ids.source_ids.get("wayback_ts").map(String::as_str),
            Some("20240315120000")
        );
    }
}

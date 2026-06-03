# Proposed skills — LLM-struggle use cases worth landing

This is a scoping document, not a roadmap commitment. It catalogs eight
common tooling tasks that **other LLMs measurably fail at** (deterministic
inputs they hallucinate or compute wrong) and proposes dedicated lodestone
skills to cover each one. Every proposal here is shaped to fit the
[contributor guide](../CONTRIBUTING.md#adding-a-skill--adding-a-tool) and
the [14 golden rules](golden-rules.md) — keyless-by-default, gated,
guarded where destructive, retrieval-policy-typed where remote, cited
where factual.

The shape of each entry:

- **Pain point** — concrete LLM failure mode with an example query.
- **Proposed family** + tool names (golden rule 9 — one method per tool).
- **Keyless?** + retrieval policy (golden rule 1, 13).
- **Destructive?** + guard policy (golden rule 8).
- **Citation source** if the tool returns factual claims (golden rule 12).
- **Reference crate** for the implementation.
- **Risk / scope** — what would push it back to a follow-up.

The goal is to pick a subset to land in 0.1.11 — not all eight at once.

---

## 1. `network` family — CIDR / subnet math

**Pain point.** LLMs are bad at binary subnet math. Ask any model
"what's the broadcast address of 10.42.7.83/19?" or "give me 4 equal
/26 subnets of 192.168.1.0/24" and you get plausible-but-wrong answers
roughly half the time. The math is deterministic; LLMs guess.

**Proposed tools.**

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `net_cidr_info` | `cidr` | Network address, broadcast, mask, wildcard, host range, host count, prefix length. |
| `net_cidr_subnets` | `cidr`, `new_prefix` | Equal-sized subnet split (e.g. /24 → four /26s). |
| `net_ip_in_cidr` | `ip`, `cidr` | Membership test + position. |
| `net_ip_classify` | `ip` | Public / private (RFC1918) / loopback / link-local / multicast / CGNAT / documentation-range / reserved. |
| `net_cidr_summarize` | `cidrs` (list) | Coalesce a list of CIDRs into the minimal set covering the same range. |

**Keyless.** Pure compute, no network. `retrieval_policy: None`.

**Destructive?** No, no guard.

**Citation source.** RFC 791 / 4632 / 1918 / 6890 cited in the module
doc; one-line citation in the relevant tool descriptions. The classifier
is governed by the IANA Special-Purpose Address Registry — pin the
revision date.

**Reference crate.** [`ipnet`](https://crates.io/crates/ipnet) (IPv4 +
IPv6, supersets, summarization, host iteration). Pure-Rust, no extra
deps.

**Risk / scope.** Tiny. IPv6 examples must hit `ipnet`'s
`Ipv6Net::aggregate` so the summarizer doesn't quietly under-coalesce.

---

## 2. `dns` family — keyless resolver + DNSSEC chain

**Pain point.** LLMs make up DNS records. Ask "what does
`example.com`'s SPF record say?" or "is `example.com` DNSSEC-signed?"
and you get fabricated TXT bodies and confident wrong "yes/no"
answers. Even when the LLM knows the right resolver to call, it
doesn't have one in-context.

**Proposed tools.**

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `dns_lookup` | `name`, `record_type?`, `resolver?` | A / AAAA / MX / TXT / CNAME / NS / SOA / SRV / CAA. Default resolver is the system one; `resolver=1.1.1.1` to override. |
| `dns_reverse` | `ip` | PTR record for a v4 or v6 address. |
| `dns_chain` | `name` | Full DNSSEC validation chain (DS / DNSKEY / RRSIG verified path). Reports `secure` / `bogus` / `insecure`. |
| `dns_propagation` | `name`, `record_type` | Query the same name across ~10 well-known public resolvers (Cloudflare 1.1.1.1, Google 8.8.8.8, Quad9, OpenDNS, …) and surface disagreements. |

**Keyless.** Plain DNS protocol over UDP/TCP. `retrieval_policy::Shared
{ source: Source::Other }` — DNS records are public and benefit from
constellation sharing (TTL governs staleness).

**Destructive?** No.

**Citation source.** RFC 1034 / 1035 / 4034 / 4035 / 6840 cited in the
module doc.

**Reference crate.** [`hickory-resolver`](https://crates.io/crates/hickory-resolver)
(formerly trust-dns; pure-Rust, DNSSEC validation supported). The propagation
tool fans out across resolvers in parallel via the existing TaskRuntime.

**Risk / scope.** DNSSEC chain validation can fall back to insecure for
unsigned zones — make sure that case is named in the output, not
reported as a failure.

---

## 3. `tls` family — certificate chain + cipher probe

**Pain point.** "Is `github.com`'s cert chain valid?" / "what's the
issuer of the cert serving port 443 on 1.2.3.4?" / "is this server
still negotiating TLS 1.2?" — LLMs guess. The information is one
network round-trip away.

**Proposed tools.**

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `tls_inspect` | `host`, `port?` (default 443), `sni?` | Negotiate TLS, walk the leaf + chain, report subject / issuer / SANs / NotBefore / NotAfter / SHA-256 fingerprints / signature algorithm / key algorithm + size / SCT count. |
| `tls_ciphersuites` | `host`, `port?` | Per-protocol-version cipher suite negotiation probe (which suites the server offers for TLS 1.2 / 1.3). |
| `tls_pem_decode` | `pem_text` | Read a PEM-encoded cert from text input (paste from `openssl x509`) and dump the same fields, no network. |

**Keyless.** No registry, no API. `retrieval_policy::None` for the
host-probe tools (results are time-sensitive); `LocalOnly` for
`tls_pem_decode` (the input came from the user).

**Destructive?** No. The probe is a read-only handshake.

**Citation source.** RFC 5280 (X.509), RFC 8446 (TLS 1.3), RFC 5246
(TLS 1.2). Cited in the module doc; tool descriptions name the field
sources.

**Reference crate.** [`rustls`](https://crates.io/crates/rustls) +
[`webpki`](https://crates.io/crates/rustls-webpki) +
[`x509-parser`](https://crates.io/crates/x509-parser) — all pure-Rust.
The cipher-probe tool needs to walk the suite list manually rather
than using `rustls`'s "pick what works" handshake.

**Risk / scope.** SNI-only servers (most modern hosts) require setting
the SNI name; the `host` arg doubles as the SNI by default. STARTTLS
ports (25, 110, 143, 587) are a follow-up — initial scope is direct
TLS only.

---

## 4. `whois` family — RDAP + classic WHOIS

**Pain point.** "Who owns `example.com`?" / "when does `example.com`
expire?" / "is this IP block ARIN- or RIPE-allocated?" — LLMs invent
registrars and dates. Real answers come from RDAP (modern) or classic
WHOIS (legacy).

**Proposed tools.**

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `whois_domain` | `domain` | RDAP lookup against the appropriate registry; falls back to classic WHOIS port 43 for legacy TLDs that don't have RDAP. Returns registrar, dates, name servers, status flags, redacted contacts. |
| `whois_ip` | `ip` | RDAP lookup against the responsible RIR (ARIN / RIPE / APNIC / LACNIC / AFRINIC). Returns owning org, network range, abuse contact. |
| `whois_asn` | `asn` | RDAP for an AS number — owning org, country, range of allocated prefixes. |

**Keyless.** RDAP is keyless and standardized (RFC 7480-7484). Classic
WHOIS port 43 is also keyless. `retrieval_policy::Shared { source:
Source::Other }` — public-record data, content-addressable by
domain/IP/ASN.

**Destructive?** No.

**Citation source.** RFC 7480-7484 (RDAP), RFC 3912 (WHOIS port 43).
IANA RDAP bootstrap registry pinned to its retrieval date.

**Reference crate.** No solid pure-Rust RDAP client exists yet; the
implementation is reqwest + the IANA bootstrap registry. ~300 lines.

**Risk / scope.** Rate limits at registry level are real. Cache
aggressively. Avoid tail-spinning on a registrar that 429s — surface
the rate-limit response cleanly, don't retry.

---

## 5. `cve` family — NIST NVD lookup

**Pain point.** "What's CVE-2024-12345?" / "list 2023 CVEs in nginx
above CVSS 7.0" — LLMs hallucinate CVE descriptions and CVSS scores
roughly all the time, including for CVEs that don't exist. Real
data is at NIST NVD which has a keyless JSON API.

**Proposed tools.**

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `cve_get` | `cve_id` | Fetch one CVE by id. Returns description, CVSS v2 / v3.1 vectors + scores, CWE, affected CPE list, references. |
| `cve_search` | `keyword?`, `cpe?`, `published_after?`, `cvss_v3_min?`, `limit?` | Search NVD by keyword, CPE id, date range, and severity floor. Returns a paginated list of summaries. |
| `cpe_search` | `keyword`, `limit?` | Find the canonical CPE 2.3 id for a vendor/product (e.g. `nginx 1.27` → `cpe:2.3:a:nginx:nginx:1.27:*:*:*:*:*:*:*`). The CPE id is what `cve_search`'s `cpe` filter takes. |

**Keyless.** NVD's public API is keyless (an optional key gives you
higher rate limits — defer until proven needed; document the optional
key surface). `retrieval_policy::Shared { source: Source::Other }` —
CVE records are append-only and content-addressable.

**Destructive?** No.

**Citation source.** NIST NVD API spec link in the module doc; CVSS
v3.1 spec for score interpretation. CVSS vectors must be reported
exactly as NIST publishes them, not re-summarized.

**Reference crate.** None needed beyond reqwest + serde_json. ~250 lines.

**Risk / scope.** NVD's rate limit without a key is 5 requests / 30
seconds. Bake the existing per-skill TTL cache (one record per CVE id,
content-addressable, indefinite — NVD records do get updated though,
so 24h TTL is the right floor) and surface the rate limit headers.

---

## 6. `cron` family — describe + next firings

**Pain point.** "What does `*/15 9-17 * * 1-5` mean?" / "when does
`0 2 * * 0` next fire?" — LLMs explain cron syntax wrong roughly half
the time on intermediate cases (DOM/DOW interaction, step values past
range, 6 vs 7 fields).

**Proposed tools.**

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `cron_describe` | `expression`, `timezone?` (default UTC) | Plain-English description of when the expression fires. |
| `cron_next` | `expression`, `count?` (default 5), `from?`, `timezone?` | Next N firings as ISO timestamps. |
| `cron_validate` | `expression` | Returns OK or a precise parse error pointing at the bad field. |

**Keyless.** Pure compute. `retrieval_policy::None`.

**Destructive?** No.

**Citation source.** Vixie cron format spec / `man 5 crontab` cited in
module doc. The describer's DOM/DOW interaction note must spell out
the "OR" semantics explicitly because LLMs always get this wrong.

**Reference crate.** [`cron`](https://crates.io/crates/cron) for
parsing + iteration. `cron_describe` needs a small custom render pass
on top.

**Risk / scope.** 7-field cron (with seconds) vs 5-field (standard).
Document which we support; default to 5 unless 7 detected.

---

## 7. `uuid` family — generation + parsing + introspection

**Pain point.** "Generate a UUIDv7 timestamped 2024-06-01 12:34:56
UTC" / "what's the embedded timestamp in
`017fc3a4-58dc-7c00-8000-000000000001`?" — LLMs fabricate UUID
encodings. Especially v7 (Unix-ms timestamp + entropy), where the
field layout is recent and frequently hallucinated.

**Proposed tools.**

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `uuid_generate` | `version` (`v4` / `v7`), `at?` (v7 only — defaults to now), `count?` | Generate one or N UUIDs. |
| `uuid_parse` | `uuid` | Identify the version + variant; for v1/v6/v7 extract the embedded timestamp + node id. |
| `uuid_to_short` | `uuid`, `encoding` (`base32` / `base58` / `base64url`) | Compact, URL-safe encoding of an existing UUID. |

**Keyless.** Pure compute. `retrieval_policy::None`.

**Destructive?** No.

**Citation source.** RFC 9562 (UUIDv1-v8) cited in module doc;
encoding tools cite RFC 4648.

**Reference crate.** [`uuid`](https://crates.io/crates/uuid) crate
(features `v4`, `v7`, `serde`). Short-encoding can use
[`bs58`](https://crates.io/crates/bs58) /
[`base32`](https://crates.io/crates/base32) /
[`base64`](https://crates.io/crates/base64).

**Risk / scope.** Tiny. The `at` arg for v7 needs millisecond
precision; document the truncation.

---

## 8. `token` family — LLM tokenizer counts

**Pain point.** "How many GPT-4 tokens is this 8k-char prompt?" /
"compare GPT-3.5 vs Claude tokenization of this paragraph" — LLMs
cannot count their own tokens accurately. Tokenizer choice affects
cost estimation, context-window planning, and chunk-boundary
selection.

**Proposed tools.**

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `token_count` | `text`, `model?` (default `gpt-4o`) | Token count + first-50 tokens decoded, for a chosen tokenizer (OpenAI BPE families via `tiktoken`; Anthropic / Llama families via their published vocabularies where available). |
| `token_compare` | `text` | Run the text through every supported tokenizer and report counts side by side. |

**Keyless.** All BPE vocabularies are public files; vendored or
fetched once and cached. `retrieval_policy::None` (purely-local
compute after the one-time vocabulary fetch).

**Destructive?** No.

**Citation source.** Vendor tokenizer specs cited per-family in
module doc. Tokenizer version pinned per `model` value so a future
silent vendor change doesn't make us misreport.

**Reference crate.** [`tiktoken-rs`](https://crates.io/crates/tiktoken-rs)
for OpenAI families. Anthropic + Llama tokenizers are larger — vendoring
their files crosses into binary-size questions worth discussing before
landing.

**Risk / scope.** Most-used tokenizers (cl100k_base, o200k_base, p50k)
all fit in `tiktoken-rs`. Anthropic specifically does NOT publish a
fully-described tokenizer, so the `anthropic` model option may have to
be a "best-effort approximation via cl100k_base" with that caveat in
the description. Don't pretend it's exact when it isn't.

---

## Honorable mentions — strong candidates, smaller surfaces

These each warrant a skill but the surfaces are smaller; one-shot
proposals to consider in a follow-up if 1-8 are oversubscribed:

- **`color` family** — `color_convert` (hex/RGB/HSL/Lab/CMYK round-trip),
  `color_contrast` (WCAG ratio for accessibility audit), `color_blend`.
  Pure compute via [`palette`](https://crates.io/crates/palette). LLMs
  hallucinate hex codes for named colors and get HSL→RGB conversion
  wrong by a few units.
- **`md` family** — `md_to_html`, `html_to_md`, `md_lint`. Pure compute
  via [`pulldown-cmark`](https://crates.io/crates/pulldown-cmark). LLMs
  often produce malformed Markdown when asked to round-trip HTML.
- **`phone` family** — `phone_parse` (E.164 normalization + country),
  `phone_format` (per-region display formatting). Pure compute via
  [`phonenumber`](https://crates.io/crates/phonenumber). LLMs fabricate
  international dial codes constantly.
- **`http` family** — `http_status_decode` (status code → RFC name +
  semantics + caveats), `http_header_parse` (well-known headers
  explained), `http_cache_decode` (Cache-Control + ETag + Vary
  interaction). Pure compute. LLMs confuse 301/302/303/307/308.

---

## Recommendation — what to land first

If we cap at three new families for 0.1.11, the highest-impact picks
based on LLM-failure rate × ubiquity are:

1. **`network` family** (CIDR math) — universal pain, tiny surface,
   pure compute, zero new deps beyond `ipnet`.
2. **`dns` family** — universal pain, network-dependent but keyless,
   shared via constellation under `Source::Other`.
3. **`cve` family** — high-stakes accuracy domain where LLM
   hallucination is actively dangerous; NIST NVD is the canonical
   source.

`tls`, `whois`, `cron`, `uuid`, `token` are all worth landing too —
the question is sequencing, not whether.

---

## Golden-rules + contributor-guide compliance checklist

For each proposal landed, the implementing PR must demonstrate:

- [ ] **GR-1 keyless by default**: implementation runs end-to-end with
      no operator-supplied credential. Optional credential (e.g. NVD
      rate-limit key) gated to `LocalOnly` retrieval policy.
- [ ] **GR-3 don't add a third-party service mid-call**: every
      external endpoint named in the module doc, no surprise hops.
- [ ] **GR-5 gateable**: every family registers in
      `src/config.rs` and `src/skills/meta.rs::families()`; the
      `[<name>].enabled` config knob exists.
- [ ] **GR-7 self-contained**: domain logic in the module, not
      `main.rs`.
- [ ] **GR-8 confirmation guard**: not applicable — none of these
      eight are destructive.
- [ ] **GR-9 one method per tool**: the tool-table splits above are
      already shaped this way (no "mode" / "kind" arg switching the
      tool's behavior).
- [ ] **GR-11 secret redaction**: `tls_pem_decode` is the only one
      that takes user-supplied input that could be secret-shaped; its
      output redacts the private key when one is mistakenly pasted in
      with the cert.
- [ ] **GR-12 citations**: each module doc cites the RFC / NIST /
      vendor spec the implementation matches, with version pinned.
- [ ] **GR-13 retrieval policy**: declared on every Skill impl per
      the table above. Public-record families (`dns`, `whois`, `cve`)
      get `Shared { source: Source::Other }`; pure-compute families
      get `None`.
- [ ] **GR-14 Mermaid**: any flow diagram in the module's
      `docs/skills/<name>.md` uses Mermaid.
- [ ] **Contributor-guide §"Worked examples and use cases"**: every
      Skill impl declares `examples()` (2-4 entries, valid JSON args
      matching the Args struct) and `use_cases()` (2-4 phrases). The
      self-check checklist applies — especially JSON validity,
      plausible domain values (e.g. `dns_lookup` example using a real
      resolved name + record type), and accurate notes.
- [ ] **`FamilyMeta` + `example_flow()`**: registered on the family
      Family struct, with a worked multi-tool flow showing the
      typical task (e.g. for `network`: classify → split → summarize;
      for `dns`: `dns_lookup A` → `dns_lookup MX` → `dns_chain`).
- [ ] **`describe_skill` round-trip**: a manual smoke check
      confirming the new tools render cleanly through `describe_skill`
      and appear in the dynamic handshake's family inventory.

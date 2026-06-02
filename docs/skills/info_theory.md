# Information theory & coding — `it_*`, `code_*`

|  |  |
| --- | --- |
| **Module** | [`src/skills/info_theory.rs`](../../src/skills/info_theory.rs) |
| **Tools** | `it_shannon_capacity`, `it_entropy`, `it_kl_divergence`, `it_js_divergence`, `it_mutual_information`, `code_hamming_distance`, `code_crc`, `code_rs_encode`, `code_convolutional_encode` |
| **Network** | none — local compute |
| **Default** | on; gateable via `[tools]` |
| **Dep** | `crc`, `reed-solomon-erasure` |

## What it does

Shannon-theory primitives and a small forward-error-correction toolkit. All
distributions are normalized internally before computing entropy / divergence
so the caller can pass unnormalized counts.

## Tools

### Information theory

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `it_shannon_capacity` | `bandwidth_hz`, `snr_linear?` or `snr_db?` | Shannon-Hartley channel capacity C = B·log₂(1+SNR) (bps). |
| `it_entropy` | `p`, `order?` | Rényi entropy of order α; α=1 (default) = Shannon, α=∞ ≈ min-entropy. |
| `it_kl_divergence` | `p`, `q` | KL divergence D(p ‖ q) — asymmetric. |
| `it_js_divergence` | `p`, `q` | Jensen-Shannon divergence — symmetric, ∈ [0, log 2]. |
| `it_mutual_information` | `joint` | Mutual information I(X;Y) from a 2-D joint distribution. |

### Channel coding

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `code_hamming_distance` | `a`, `b` (hex) | Bitwise Hamming distance between equal-length hex byte strings. |
| `code_crc` | `data` (hex), `algorithm` | Compute a CRC checksum. Algorithms: `crc8`, `crc16-ccitt`, `crc16-modbus`, `crc16-x25`, `crc32`, `crc32c`, `crc64-ecma`, `crc64-iso`. |
| `code_rs_encode` | `data_shards`, `parity_shards` | Reed-Solomon encode; returns data + parity shards. |
| `code_convolutional_encode` | `data` (hex) | K=7, rate-½ convolutional encode with generators G1=0o171, G2=0o133. |

## Example uses

- **Link-budget margin sanity.** Plug bandwidth and required Eb/N0 into
  `it_shannon_capacity` to see the theoretical ceiling — compare to a
  measured bit rate.
- **Entropy of a sample.** Count token frequencies, feed `p` to
  `it_entropy` → bits-per-symbol; pair with `code_crc` for integrity.
- **Erasure-coded storage**. `code_rs_encode` for k+m sharding; the
  matching decoder is left to the consumer.

## Notes

- Distributions don't need to sum to 1 — they're renormalized.
- Hex inputs are case-insensitive; whitespace is stripped.

## See also

- [tools.md](../tools.md)
- [skills/rf_link.md](rf_link.md) — Shannon capacity ties into link
  budgets.
- [skills/crypto_math.md](crypto_math.md) — HMAC / HKDF complement the CRC
  surface.

# Crypto primitives as math tools — `crypto_*`

|  |  |
| --- | --- |
| **Module** | [`src/skills/crypto_math.rs`](../../src/skills/crypto_math.rs) |
| **Tools** | `crypto_miller_rabin`, `crypto_modexp`, `crypto_mod_inverse`, `crypto_crt`, `crypto_hkdf`, `crypto_pbkdf2`, `crypto_argon2`, `crypto_hmac`, `crypto_jwt_decode` |
| **Network** | none — local compute |
| **Default** | on; gateable via `[tools]` |
| **Dep** | `num-bigint`, `hmac`, `sha1`, `sha2`, `hkdf`, `pbkdf2`, `argon2`, `jwt`, `base64` |

> **Educational / numeric surface — not a production crypto stack.** These
> tools exist so a model can reason about and demonstrate cryptographic
> primitives. They're **not** wired into any secret-handling path; they
> don't manage keys, they don't verify signatures (the JWT tool decodes
> without verification), and they shouldn't be used to operate real
> credentials. For real crypto, use a vetted library + a KMS.

## What it does

A small inventory of number-theoretic and password / key-derivation tools.
Big-integer arguments are accepted as decimal or `0x…` hex strings;
byte-string arguments are hex-encoded.

## Tools

### Number theory

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `crypto_miller_rabin` | `n`, `rounds?` | Probabilistic primality test (default 40 rounds). |
| `crypto_modexp` | `base`, `exponent`, `modulus` | `base^exponent mod modulus` for arbitrary big integers. |
| `crypto_mod_inverse` | `a`, `modulus` | Modular multiplicative inverse via extended GCD. |
| `crypto_crt` | `residues`, `moduli` | Solve `x ≡ rᵢ (mod mᵢ)` via the Chinese Remainder Theorem. |

### Key derivation & MAC

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `crypto_hkdf` | `ikm`, `salt?`, `info?`, `length` | HKDF-SHA-256 output as hex. |
| `crypto_pbkdf2` | `password`, `salt_hex`, `iterations`, `length` | PBKDF2-HMAC-SHA-256. |
| `crypto_argon2` | `password`, `salt_hex`, `time?`, `memory?`, `parallelism?`, `length?` | Argon2id (defaults follow OWASP 2023 low-end). |
| `crypto_hmac` | `key_hex`, `message_hex`, `algorithm` | HMAC over `sha1`, `sha256`, `sha384`, `sha512`. |

### Tokens

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `crypto_jwt_decode` | `token` | Decode (no verification) a JWT's header + claims; returns the alg, kid, and a parsed JSON body. |

## Example uses

- **Show me an Argon2id hash for "password".** `crypto_argon2 { password,
  salt_hex }` — prints the derived bytes; bump `memory` / `time` to feel
  the cost.
- **What's inside this JWT?** `crypto_jwt_decode { token }` — gets the
  header, claims, and signing alg without trusting the signature.
- **Coprime check.** `crypto_mod_inverse { a, modulus }` — succeeds iff
  `gcd(a, modulus) = 1`.

## Notes

- `crypto_jwt_decode` **does not** verify. Pair with a real verifier for
  any security-relevant use.
- Argon2 defaults: time=3, memory=65 536 KiB, parallelism=1, length=32 —
  the OWASP recommended low-end. Adjust for current guidance.

## See also

- [tools.md](../tools.md)
- [skills/info_theory.md](info_theory.md) — CRC + Reed-Solomon for
  non-secret integrity.

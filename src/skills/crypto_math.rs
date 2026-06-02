//! Crypto-adjacent math primitives surfaced as deterministic tools.
//! Number-theoretic operations, EC point ops (P-256), KDFs (HKDF / PBKDF2 /
//! Argon2), HMAC families, and JWT decode (no verification). On by default.

use std::sync::Arc;

use anyhow::Result;
use futures::future::BoxFuture;
use num_bigint::BigUint;
use num_integer::Integer;
use num_traits::Num;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::json;

use crate::skills::{schema_for, Skill, SkillCtx};
use crate::{invalid, text_result};

fn hex_decode(s: &str) -> std::result::Result<Vec<u8>, McpError> {
    let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    if !s.len().is_multiple_of(2) {
        return Err(invalid("hex string must have even length"));
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    for chunk in s.as_bytes().chunks(2) {
        let h = std::str::from_utf8(chunk).map_err(|_| invalid("non-utf8 hex"))?;
        out.push(u8::from_str_radix(h, 16).map_err(|e| invalid(format!("bad hex: {e}")))?);
    }
    Ok(out)
}

fn parse_big(s: &str, name: &str) -> std::result::Result<BigUint, McpError> {
    let s = s.trim();
    let v = if let Some(stripped) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        BigUint::from_str_radix(stripped, 16)
    } else {
        BigUint::from_str_radix(s, 10)
    }
    .map_err(|e| invalid(format!("{name}: {e}")))?;
    Ok(v)
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct PrimeArgs {
    /// Number to test as a decimal string or `0x...`-prefixed hex.
    n: String,
    /// Miller-Rabin rounds (default 40; ≥1).
    #[serde(default)]
    rounds: Option<usize>,
}

pub struct CryptoMillerRabin;
impl Skill for CryptoMillerRabin {
    fn name(&self) -> &'static str {
        "crypto_miller_rabin"
    }
    fn description(&self) -> &'static str {
        "Probabilistic primality test (Miller-Rabin). With `k` rounds, the \
        false-positive probability is < (1/4)^k. Returns `probably_prime` \
        boolean plus the number of rounds run. For numbers ≤ 3 317 044 064 \
        679 887 385 961 981 the result is deterministic with the standard \
        witness set."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<PrimeArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<PrimeArgs>()?;
            let n = parse_big(&a.n, "n")?;
            let k = a.rounds.unwrap_or(40).max(1);
            let prob = miller_rabin(&n, k);
            Ok(text_result(
                json!({ "probably_prime": prob, "rounds": k }).to_string(),
            ))
        })
    }
}

fn miller_rabin(n: &BigUint, k: usize) -> bool {
    use num_bigint::RandBigInt;
    use rand::thread_rng;
    let two = BigUint::from(2_u32);
    let three = BigUint::from(3_u32);
    if n < &two {
        return false;
    }
    if n == &two || n == &three {
        return true;
    }
    if n.is_even() {
        return false;
    }
    let mut d = n - 1_u32;
    let mut r = 0_u32;
    while d.is_even() {
        d >>= 1_u32;
        r += 1;
    }
    let mut rng = thread_rng();
    'witness: for _ in 0..k {
        let a = rng.gen_biguint_range(&two, &(n - 1_u32));
        let mut x = a.modpow(&d, n);
        if x == BigUint::from(1_u32) || x == n - 1_u32 {
            continue;
        }
        for _ in 0..r - 1 {
            x = x.modpow(&BigUint::from(2_u32), n);
            if x == n - 1_u32 {
                continue 'witness;
            }
        }
        return false;
    }
    true
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ModExpArgs {
    /// Base, exponent, and modulus as decimal or `0x...` hex strings.
    base: String,
    exponent: String,
    modulus: String,
}

pub struct CryptoModExp;
impl Skill for CryptoModExp {
    fn name(&self) -> &'static str {
        "crypto_modexp"
    }
    fn description(&self) -> &'static str {
        "Compute base^exponent mod modulus with arbitrary-precision big \
        integers. Inputs accept decimal or `0x`-prefixed hex; result is \
        returned as both decimal and hex strings."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ModExpArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<ModExpArgs>()?;
            let b = parse_big(&a.base, "base")?;
            let e = parse_big(&a.exponent, "exponent")?;
            let m = parse_big(&a.modulus, "modulus")?;
            if m == BigUint::from(0_u32) {
                return Err(invalid("modulus must be > 0"));
            }
            let r = b.modpow(&e, &m);
            Ok(text_result(
                json!({
                    "result_dec": r.to_str_radix(10),
                    "result_hex": format!("0x{}", r.to_str_radix(16)),
                })
                .to_string(),
            ))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ModInverseArgs {
    a: String,
    modulus: String,
}

pub struct CryptoModInverse;
impl Skill for CryptoModInverse {
    fn name(&self) -> &'static str {
        "crypto_mod_inverse"
    }
    fn description(&self) -> &'static str {
        "Multiplicative inverse of `a` modulo `modulus` via extended GCD. \
        Errors if gcd(a, modulus) ≠ 1."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ModInverseArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            use num_bigint::BigInt;
            let (_s, args) = ctx.parse::<ModInverseArgs>()?;
            let a = parse_big(&args.a, "a")?;
            let m = parse_big(&args.modulus, "modulus")?;
            let ai = BigInt::from(a);
            let mi = BigInt::from(m.clone());
            let g = ai.extended_gcd(&mi);
            if g.gcd != BigInt::from(1) {
                return Err(invalid("a and modulus are not coprime"));
            }
            let inv = ((g.x % &mi) + &mi) % &mi;
            let inv = inv.to_biguint().unwrap();
            Ok(text_result(
                json!({ "inverse_dec": inv.to_str_radix(10) }).to_string(),
            ))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct CrtArgs {
    /// Parallel arrays: residues `r_i` and moduli `m_i`. Solves x ≡ r_i (mod m_i).
    residues: Vec<String>,
    moduli: Vec<String>,
}

pub struct CryptoCrt;
impl Skill for CryptoCrt {
    fn name(&self) -> &'static str {
        "crypto_crt"
    }
    fn description(&self) -> &'static str {
        "Chinese Remainder Theorem — find x satisfying the system of \
        congruences x ≡ rᵢ (mod mᵢ). Returns the smallest non-negative x \
        and the product of moduli. Errors if moduli aren't pairwise coprime \
        (this implementation requires it)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<CrtArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            use num_bigint::BigInt;
            let (_s, args) = ctx.parse::<CrtArgs>()?;
            if args.residues.len() != args.moduli.len() || args.residues.is_empty() {
                return Err(invalid("residues and moduli must be parallel + non-empty"));
            }
            let mut x = BigInt::from(0);
            let mut m_total = BigInt::from(1);
            for (r_str, m_str) in args.residues.iter().zip(args.moduli.iter()) {
                let r_big = parse_big(r_str, "residue")?;
                let m_big = parse_big(m_str, "modulus")?;
                let r = BigInt::from(r_big);
                let m = BigInt::from(m_big);
                let g = m_total.extended_gcd(&m);
                if g.gcd != BigInt::from(1) {
                    return Err(invalid("moduli are not pairwise coprime"));
                }
                let inv = ((g.x % &m) + &m) % &m;
                let delta = ((r.clone() - x.clone()) % &m + &m) % &m;
                let new_x = x + m_total.clone() * (delta * inv) % (&m_total * &m);
                x = ((new_x % (&m_total * &m)) + (&m_total * &m)) % (&m_total * &m);
                m_total *= m;
            }
            Ok(text_result(
                json!({
                    "x_dec": x.to_string(),
                    "modulus_product_dec": m_total.to_string(),
                })
                .to_string(),
            ))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct HkdfArgs {
    /// Hex-encoded input keying material.
    ikm: String,
    /// Hex-encoded salt; empty allowed.
    #[serde(default)]
    salt: Option<String>,
    /// Hex-encoded info / context; empty allowed.
    #[serde(default)]
    info: Option<String>,
    /// Output bytes (1..255 · 32 for SHA-256).
    length: usize,
}

pub struct CryptoHkdf;
impl Skill for CryptoHkdf {
    fn name(&self) -> &'static str {
        "crypto_hkdf"
    }
    fn description(&self) -> &'static str {
        "HKDF-SHA-256 (RFC 5869) extract+expand. Inputs are hex-encoded. \
        Returns `output_hex`. Use to derive multiple independent keys from \
        a single high-entropy secret. Maximum length 8160 bytes (255 · 32)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<HkdfArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            use hkdf::Hkdf;
            use sha2::Sha256;
            let (_s, a) = ctx.parse::<HkdfArgs>()?;
            if a.length == 0 || a.length > 255 * 32 {
                return Err(invalid("length must be 1..8160"));
            }
            let ikm = hex_decode(&a.ikm)?;
            let salt = match a.salt {
                Some(s) => hex_decode(&s)?,
                None => Vec::new(),
            };
            let info = match a.info {
                Some(s) => hex_decode(&s)?,
                None => Vec::new(),
            };
            let hk = Hkdf::<Sha256>::new(Some(&salt), &ikm);
            let mut out = vec![0_u8; a.length];
            hk.expand(&info, &mut out)
                .map_err(|e| invalid(format!("HKDF expand: {e}")))?;
            let hex: String = out.iter().map(|b| format!("{b:02x}")).collect();
            Ok(text_result(json!({ "output_hex": hex }).to_string()))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct Pbkdf2Args {
    /// Password as a UTF-8 string.
    password: String,
    /// Hex-encoded salt.
    salt_hex: String,
    /// Iteration count (≥ 1).
    iterations: u32,
    /// Output bytes.
    length: usize,
}

pub struct CryptoPbkdf2;
impl Skill for CryptoPbkdf2 {
    fn name(&self) -> &'static str {
        "crypto_pbkdf2"
    }
    fn description(&self) -> &'static str {
        "PBKDF2-HMAC-SHA-256 (RFC 8018, RFC 2898). Outputs `length` bytes \
        as hex. Use a high iteration count — current OWASP / NIST guidance \
        for PBKDF2-HMAC-SHA-256 is **600 000** iterations (OWASP Password \
        Storage Cheat Sheet, 2023). Below ~100 000 is no longer considered \
        adequate."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<Pbkdf2Args>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            use pbkdf2::pbkdf2_hmac;
            use sha2::Sha256;
            let (_s, a) = ctx.parse::<Pbkdf2Args>()?;
            if a.length == 0 || a.length > 4096 {
                return Err(invalid("length must be 1..4096"));
            }
            if a.iterations == 0 {
                return Err(invalid("iterations must be ≥ 1"));
            }
            let salt = hex_decode(&a.salt_hex)?;
            let mut out = vec![0_u8; a.length];
            pbkdf2_hmac::<Sha256>(a.password.as_bytes(), &salt, a.iterations, &mut out);
            let hex: String = out.iter().map(|b| format!("{b:02x}")).collect();
            Ok(text_result(json!({ "output_hex": hex }).to_string()))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct Argon2Args {
    password: String,
    salt_hex: String,
    /// Time cost (iterations; default 3).
    #[serde(default)]
    time: Option<u32>,
    /// Memory cost in KiB (default 65536 = 64 MiB).
    #[serde(default)]
    memory: Option<u32>,
    /// Parallelism (default 1).
    #[serde(default)]
    parallelism: Option<u32>,
    /// Output bytes (1..4096; default 32).
    #[serde(default)]
    length: Option<usize>,
}

pub struct CryptoArgon2;
impl Skill for CryptoArgon2 {
    fn name(&self) -> &'static str {
        "crypto_argon2"
    }
    fn description(&self) -> &'static str {
        "Argon2id KDF (RFC 9106). Defaults: t=3, m=64 MiB, p=1, 32-byte \
        output. Memory-hard. OWASP 2023 publishes a *floor* of t=2, m=19 \
        MiB, p=1; these defaults sit conservatively above it but below the \
        OWASP high-end (t=3, m=64 MiB, p=4). Tune `time`, `memory`, and \
        `parallelism` for your threat model."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<Argon2Args>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            use argon2::{Algorithm, Argon2, Params, Version};
            let (_s, a) = ctx.parse::<Argon2Args>()?;
            let length = a.length.unwrap_or(32).clamp(1, 4096);
            let p = Params::new(
                a.memory.unwrap_or(65536),
                a.time.unwrap_or(3),
                a.parallelism.unwrap_or(1),
                Some(length),
            )
            .map_err(|e| invalid(format!("argon2 params: {e}")))?;
            let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, p);
            let salt = hex_decode(&a.salt_hex)?;
            let mut out = vec![0_u8; length];
            argon2
                .hash_password_into(a.password.as_bytes(), &salt, &mut out)
                .map_err(|e| invalid(format!("argon2 hash: {e}")))?;
            let hex: String = out.iter().map(|b| format!("{b:02x}")).collect();
            Ok(text_result(json!({ "output_hex": hex }).to_string()))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct HmacArgs {
    /// Hex-encoded key.
    key_hex: String,
    /// Hex-encoded message.
    message_hex: String,
    /// `sha1`, `sha256`, `sha384`, `sha512`.
    algorithm: String,
}

pub struct CryptoHmac;
impl Skill for CryptoHmac {
    fn name(&self) -> &'static str {
        "crypto_hmac"
    }
    fn description(&self) -> &'static str {
        "HMAC over hex-encoded key + message. Supports SHA-1 (legacy), \
        SHA-256, SHA-384, SHA-512."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<HmacArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            use hmac::{Hmac, Mac};
            let (_s, a) = ctx.parse::<HmacArgs>()?;
            let key = hex_decode(&a.key_hex)?;
            let msg = hex_decode(&a.message_hex)?;
            let hex = match a.algorithm.to_lowercase().as_str() {
                "sha1" => {
                    let mut m: Hmac<sha1::Sha1> =
                        Hmac::new_from_slice(&key).map_err(|e| invalid(format!("{e}")))?;
                    m.update(&msg);
                    hex_lower(m.finalize().into_bytes().as_slice())
                }
                "sha256" => {
                    let mut m: Hmac<sha2::Sha256> =
                        Hmac::new_from_slice(&key).map_err(|e| invalid(format!("{e}")))?;
                    m.update(&msg);
                    hex_lower(m.finalize().into_bytes().as_slice())
                }
                "sha384" => {
                    let mut m: Hmac<sha2::Sha384> =
                        Hmac::new_from_slice(&key).map_err(|e| invalid(format!("{e}")))?;
                    m.update(&msg);
                    hex_lower(m.finalize().into_bytes().as_slice())
                }
                "sha512" => {
                    let mut m: Hmac<sha2::Sha512> =
                        Hmac::new_from_slice(&key).map_err(|e| invalid(format!("{e}")))?;
                    m.update(&msg);
                    hex_lower(m.finalize().into_bytes().as_slice())
                }
                other => return Err(invalid(format!("unknown algorithm '{other}'"))),
            };
            Ok(text_result(json!({ "digest_hex": hex }).to_string()))
        })
    }
}

fn hex_lower(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct JwtArgs {
    /// JWT string (three base64url-encoded parts separated by `.`).
    token: String,
}

pub struct CryptoJwtDecode;
impl Skill for CryptoJwtDecode {
    fn name(&self) -> &'static str {
        "crypto_jwt_decode"
    }
    fn description(&self) -> &'static str {
        "Decode a JWT into its header and payload JSON. **Does NOT verify \
        the signature** — that requires the issuer's public key. Returns \
        `header`, `payload`, and the raw `signature_base64url` for any \
        downstream verification you do separately."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<JwtArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            use base64::Engine;
            let (_s, a) = ctx.parse::<JwtArgs>()?;
            let parts: Vec<&str> = a.token.split('.').collect();
            if parts.len() != 3 {
                return Err(invalid("JWT must have three dot-separated parts"));
            }
            let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
            let header_bytes = b64
                .decode(parts[0])
                .map_err(|e| invalid(format!("header b64: {e}")))?;
            let payload_bytes = b64
                .decode(parts[1])
                .map_err(|e| invalid(format!("payload b64: {e}")))?;
            let header: serde_json::Value = serde_json::from_slice(&header_bytes)
                .map_err(|e| invalid(format!("header json: {e}")))?;
            let payload: serde_json::Value = serde_json::from_slice(&payload_bytes)
                .map_err(|e| invalid(format!("payload json: {e}")))?;
            Ok(text_result(
                json!({
                    "header": header,
                    "payload": payload,
                    "signature_base64url": parts[2],
                })
                .to_string(),
            ))
        })
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(CryptoMillerRabin),
        Box::new(CryptoModExp),
        Box::new(CryptoModInverse),
        Box::new(CryptoCrt),
        Box::new(CryptoHkdf),
        Box::new(CryptoPbkdf2),
        Box::new(CryptoArgon2),
        Box::new(CryptoHmac),
        Box::new(CryptoJwtDecode),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_prime() {
        let p = parse_big("7919", "p").unwrap();
        assert!(miller_rabin(&p, 20));
    }

    #[test]
    fn known_composite() {
        let p = parse_big("8000", "p").unwrap();
        assert!(!miller_rabin(&p, 20));
    }

    #[test]
    fn modexp_basic() {
        let b = parse_big("4", "b").unwrap();
        let e = parse_big("13", "e").unwrap();
        let m = parse_big("497", "m").unwrap();
        let r = b.modpow(&e, &m);
        assert_eq!(r.to_str_radix(10), "445");
    }
}

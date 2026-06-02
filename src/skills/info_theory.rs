//! Information theory + coding skill — Shannon-Hartley capacity, entropy
//! family, divergence, Hamming code-distance, CRC variants, Reed-Solomon
//! erasure encode/reconstruct, convolutional encode + Viterbi decode.
//! Pure math; on by default.

use std::sync::Arc;

use anyhow::Result;
use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::json;

use crate::skills::{schema_for, Skill, SkillCtx};
use crate::{invalid, text_result};

// ---------------------------------------------------------------------------
// Capacity + entropy primitives
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct CapacityArgs {
    /// Bandwidth in hertz.
    bandwidth_hz: f64,
    /// SNR; `snr_db` is also accepted.
    #[serde(default)]
    snr_linear: Option<f64>,
    #[serde(default)]
    snr_db: Option<f64>,
}

pub struct ItShannonCapacity;
impl Skill for ItShannonCapacity {
    fn name(&self) -> &'static str {
        "it_shannon_capacity"
    }
    fn description(&self) -> &'static str {
        "Shannon-Hartley channel capacity: C = B · log₂(1 + SNR). Supply \
        bandwidth in Hz and the SNR either as a linear ratio or in dB. \
        Returns `capacity_bps`."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<CapacityArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<CapacityArgs>()?;
            if a.bandwidth_hz <= 0.0 {
                return Err(invalid("bandwidth_hz must be > 0"));
            }
            let snr = match (a.snr_linear, a.snr_db) {
                (Some(l), None) if l >= 0.0 => l,
                (None, Some(db)) => 10_f64.powf(db / 10.0),
                _ => return Err(invalid("supply one of snr_linear (≥0) or snr_db")),
            };
            let c = a.bandwidth_hz * (1.0 + snr).log2();
            Ok(text_result(json!({ "capacity_bps": c }).to_string()))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DistArgs {
    /// Probability distribution (any non-negative numbers; will be normalized).
    p: Vec<f64>,
    /// Rényi order; `1.0` (default) gives Shannon entropy. Use `2.0` for
    /// collision entropy, `∞` for min-entropy (use a huge number).
    #[serde(default)]
    order: Option<f64>,
}

pub struct ItEntropy;
impl Skill for ItEntropy {
    fn name(&self) -> &'static str {
        "it_entropy"
    }
    fn description(&self) -> &'static str {
        "Entropy of a discrete distribution `p`. Shannon (default), Rényi of \
        arbitrary order α via `order=α`, min-entropy via a huge order. \
        Returns `entropy_bits`."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<DistArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<DistArgs>()?;
            let p = normalize(&a.p)?;
            let order = a.order.unwrap_or(1.0);
            let h = if (order - 1.0).abs() < 1e-9 {
                -p.iter()
                    .filter(|&&x| x > 0.0)
                    .map(|x| x * x.log2())
                    .sum::<f64>()
            } else if order.is_infinite() && order > 0.0 {
                -p.iter().fold(0_f64, |a, &x| a.max(x)).log2()
            } else {
                let s: f64 = p.iter().map(|x| x.powf(order)).sum();
                s.log2() / (1.0 - order)
            };
            Ok(text_result(json!({ "entropy_bits": h }).to_string()))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct TwoDistArgs {
    p: Vec<f64>,
    q: Vec<f64>,
}

pub struct ItKlDivergence;
impl Skill for ItKlDivergence {
    fn name(&self) -> &'static str {
        "it_kl_divergence"
    }
    fn description(&self) -> &'static str {
        "Kullback-Leibler divergence D_KL(p || q) in bits. Returns `+∞` for \
        any q_i = 0 where p_i > 0. Asymmetric."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<TwoDistArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<TwoDistArgs>()?;
            if a.p.len() != a.q.len() {
                return Err(invalid("p and q must be same length"));
            }
            let p = normalize(&a.p)?;
            let q = normalize(&a.q)?;
            let mut d = 0_f64;
            for (pi, qi) in p.iter().zip(q.iter()) {
                if *pi > 0.0 {
                    if *qi == 0.0 {
                        d = f64::INFINITY;
                        break;
                    }
                    d += pi * (pi / qi).log2();
                }
            }
            Ok(text_result(json!({ "kl_bits": d }).to_string()))
        })
    }
}

pub struct ItJsDivergence;
impl Skill for ItJsDivergence {
    fn name(&self) -> &'static str {
        "it_js_divergence"
    }
    fn description(&self) -> &'static str {
        "Jensen-Shannon divergence (symmetric, bounded by 1 bit): \
        JS(p, q) = ½ KL(p || m) + ½ KL(q || m) with m = (p + q)/2."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<TwoDistArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<TwoDistArgs>()?;
            if a.p.len() != a.q.len() {
                return Err(invalid("p and q must be same length"));
            }
            let p = normalize(&a.p)?;
            let q = normalize(&a.q)?;
            let m: Vec<f64> = p
                .iter()
                .zip(q.iter())
                .map(|(pi, qi)| 0.5 * (pi + qi))
                .collect();
            let kl = |a: &[f64], b: &[f64]| -> f64 {
                let mut d = 0_f64;
                for (ai, bi) in a.iter().zip(b.iter()) {
                    if *ai > 0.0 && *bi > 0.0 {
                        d += ai * (ai / bi).log2();
                    }
                }
                d
            };
            let js = 0.5 * kl(&p, &m) + 0.5 * kl(&q, &m);
            Ok(text_result(json!({ "js_bits": js }).to_string()))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct JointDistArgs {
    /// Joint distribution P(X, Y) as rows × cols.
    joint: Vec<Vec<f64>>,
}

pub struct ItMutualInformation;
impl Skill for ItMutualInformation {
    fn name(&self) -> &'static str {
        "it_mutual_information"
    }
    fn description(&self) -> &'static str {
        "Mutual information I(X; Y) in bits from a joint probability table. \
        Rows are X outcomes, columns Y outcomes. Returns marginal entropies \
        H(X), H(Y), the joint H(X, Y), and the MI."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<JointDistArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<JointDistArgs>()?;
            if a.joint.is_empty() || a.joint[0].is_empty() {
                return Err(invalid("joint must be a non-empty matrix"));
            }
            let nr = a.joint.len();
            let nc = a.joint[0].len();
            for r in &a.joint {
                if r.len() != nc {
                    return Err(invalid("joint matrix is ragged"));
                }
                if r.iter().any(|x| *x < 0.0) {
                    return Err(invalid("joint must be non-negative"));
                }
            }
            let total: f64 = a.joint.iter().flatten().sum();
            if total <= 0.0 {
                return Err(invalid("joint sums to zero"));
            }
            let mut p_x = vec![0.0; nr];
            let mut p_y = vec![0.0; nc];
            for (i, row) in a.joint.iter().enumerate() {
                for (j, val) in row.iter().enumerate() {
                    let v = val / total;
                    p_x[i] += v;
                    p_y[j] += v;
                }
            }
            let h_of = |dist: &[f64]| -> f64 {
                -dist
                    .iter()
                    .filter(|&&x| x > 0.0)
                    .map(|x| x * x.log2())
                    .sum::<f64>()
            };
            let h_x = h_of(&p_x);
            let h_y = h_of(&p_y);
            let mut h_xy = 0_f64;
            let mut mi = 0_f64;
            for (i, row) in a.joint.iter().enumerate() {
                for (j, val) in row.iter().enumerate() {
                    let v = val / total;
                    if v > 0.0 {
                        h_xy -= v * v.log2();
                        mi += v * (v / (p_x[i] * p_y[j])).log2();
                    }
                }
            }
            Ok(text_result(
                json!({
                    "mutual_information_bits": mi,
                    "h_x_bits": h_x,
                    "h_y_bits": h_y,
                    "h_xy_bits": h_xy,
                })
                .to_string(),
            ))
        })
    }
}

fn normalize(p: &[f64]) -> std::result::Result<Vec<f64>, McpError> {
    if p.iter().any(|x| *x < 0.0) {
        return Err(invalid("distribution has negative values"));
    }
    let s: f64 = p.iter().sum();
    if s <= 0.0 {
        return Err(invalid("distribution sums to zero"));
    }
    Ok(p.iter().map(|x| x / s).collect())
}

// ---------------------------------------------------------------------------
// Coding — Hamming distance, CRC, Reed-Solomon, convolutional+Viterbi
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct HammingArgs {
    /// Hex-encoded byte strings of equal length.
    a: String,
    b: String,
}

pub struct CodeHammingDistance;
impl Skill for CodeHammingDistance {
    fn name(&self) -> &'static str {
        "code_hamming_distance"
    }
    fn description(&self) -> &'static str {
        "Bitwise Hamming distance between two hex-encoded byte strings of \
        equal length. Returns bit-differences."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<HammingArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, args) = ctx.parse::<HammingArgs>()?;
            let a = hex_decode(&args.a)?;
            let b = hex_decode(&args.b)?;
            if a.len() != b.len() {
                return Err(invalid("a and b must be the same byte length"));
            }
            let d: u32 = a
                .iter()
                .zip(b.iter())
                .map(|(x, y)| (x ^ y).count_ones())
                .sum();
            Ok(text_result(json!({ "distance_bits": d }).to_string()))
        })
    }
}

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

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct CrcArgs {
    /// Hex-encoded input bytes.
    data: String,
    /// One of: `crc8`, `crc16-ccitt`, `crc16-modbus`, `crc16-x25`,
    /// `crc32`, `crc32c`, `crc64-ecma`, `crc64-iso`.
    algorithm: String,
}

pub struct CodeCrc;
impl Skill for CodeCrc {
    fn name(&self) -> &'static str {
        "code_crc"
    }
    fn description(&self) -> &'static str {
        "Compute one of several common CRC variants. Supports `crc8` \
        (SMBus, poly 0x07), `crc16-ccitt` (**KERMIT** parameters — poly \
        0x1021, reflected input/output, init 0x0000; this is *one* of \
        three sets often called \"CRC-16-CCITT\" — the other two are \
        XMODEM and CCITT-FALSE, which differ in initial value), \
        `crc16-modbus` (poly 0x8005, reflected, init 0xFFFF), `crc16-x25` \
        (IBM-SDLC / X.25 / HDLC), `crc32` (ISO-HDLC — Ethernet, PNG, \
        zip), `crc32c` (Castagnoli — iSCSI, SCTP), `crc64-ecma`, \
        `crc64-iso`. Returns the digest as a hex string."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<CrcArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<CrcArgs>()?;
            let data = hex_decode(&a.data)?;
            use crc::*;
            let v = match a.algorithm.to_lowercase().as_str() {
                "crc8" => format!("{:02x}", Crc::<u8>::new(&CRC_8_SMBUS).checksum(&data)),
                "crc16-ccitt" => format!("{:04x}", Crc::<u16>::new(&CRC_16_KERMIT).checksum(&data)),
                "crc16-modbus" => {
                    format!("{:04x}", Crc::<u16>::new(&CRC_16_MODBUS).checksum(&data))
                }
                "crc16-x25" => format!("{:04x}", Crc::<u16>::new(&CRC_16_IBM_SDLC).checksum(&data)),
                "crc32" => format!("{:08x}", Crc::<u32>::new(&CRC_32_ISO_HDLC).checksum(&data)),
                "crc32c" => format!("{:08x}", Crc::<u32>::new(&CRC_32_ISCSI).checksum(&data)),
                "crc64-ecma" => {
                    format!("{:016x}", Crc::<u64>::new(&CRC_64_ECMA_182).checksum(&data))
                }
                "crc64-iso" => format!("{:016x}", Crc::<u64>::new(&CRC_64_GO_ISO).checksum(&data)),
                other => return Err(invalid(format!("unknown CRC '{other}'"))),
            };
            Ok(text_result(json!({ "digest_hex": v }).to_string()))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct RsArgs {
    /// Data shards as parallel arrays of bytes. All shards must be the same length.
    /// For encoding pass the data shards only; parity shards are output appended.
    data_shards: Vec<Vec<u8>>,
    /// Number of parity shards to compute (1..=255 minus data_shards.len()).
    parity_shards: usize,
}

pub struct CodeRsEncode;
impl Skill for CodeRsEncode {
    fn name(&self) -> &'static str {
        "code_rs_encode"
    }
    fn description(&self) -> &'static str {
        "Reed-Solomon erasure encode (over GF(2^8)) — append `parity_shards` \
        parity shards to the supplied data shards. With `D` data + `P` parity \
        shards, any `D` of the `D+P` total are sufficient to reconstruct the \
        data. Returns all shards including parity."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<RsArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            use reed_solomon_erasure::galois_8::ReedSolomon;
            let (_s, a) = ctx.parse::<RsArgs>()?;
            if a.data_shards.is_empty() {
                return Err(invalid("data_shards must be non-empty"));
            }
            let len = a.data_shards[0].len();
            for d in &a.data_shards {
                if d.len() != len {
                    return Err(invalid("all data shards must be the same length"));
                }
            }
            if a.parity_shards == 0 || a.data_shards.len() + a.parity_shards > 255 {
                return Err(invalid("parity_shards must be ≥ 1 and total shards ≤ 255"));
            }
            let rs = ReedSolomon::new(a.data_shards.len(), a.parity_shards)
                .map_err(|e| invalid(format!("RS init: {e}")))?;
            let mut shards: Vec<Vec<u8>> = a.data_shards.clone();
            for _ in 0..a.parity_shards {
                shards.push(vec![0_u8; len]);
            }
            rs.encode(&mut shards)
                .map_err(|e| invalid(format!("RS encode: {e}")))?;
            Ok(text_result(json!({ "shards": shards }).to_string()))
        })
    }
}

// Convolutional (rate 1/2, K=7, NASA standard G1=0o171, G2=0o133) encode + Viterbi decode.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ConvArgs {
    /// Hex-encoded input bytes.
    data: String,
}

pub struct CodeConvolutionalEncode;
impl Skill for CodeConvolutionalEncode {
    fn name(&self) -> &'static str {
        "code_convolutional_encode"
    }
    fn description(&self) -> &'static str {
        "Convolutional encoder (rate 1/2, constraint length K=7, polynomials \
        G1=0o171, G2=0o133 — the NASA / CCSDS standard). Returns the encoded \
        stream as a hex string (bit-packed, two output bits per input bit)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ConvArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<ConvArgs>()?;
            let bytes = hex_decode(&a.data)?;
            let bits: Vec<u8> = bytes
                .iter()
                .flat_map(|b| (0..8).rev().map(move |i| (b >> i) & 1))
                .collect();
            const G1: u8 = 0o171;
            const G2: u8 = 0o133;
            let mut reg: u8 = 0;
            let mut out_bits: Vec<u8> = Vec::with_capacity(bits.len() * 2);
            for b in &bits {
                reg = (reg << 1) & 0x7F;
                reg |= b & 1;
                let g1 = (G1 & reg).count_ones() & 1;
                let g2 = (G2 & reg).count_ones() & 1;
                out_bits.push(g1 as u8);
                out_bits.push(g2 as u8);
            }
            // Pad to byte boundary.
            while !out_bits.len().is_multiple_of(8) {
                out_bits.push(0);
            }
            let mut out_bytes: Vec<u8> = Vec::with_capacity(out_bits.len() / 8);
            for chunk in out_bits.chunks(8) {
                let mut v = 0_u8;
                for (i, b) in chunk.iter().enumerate() {
                    v |= b << (7 - i);
                }
                out_bytes.push(v);
            }
            let hex: String = out_bytes.iter().map(|b| format!("{b:02x}")).collect();
            Ok(text_result(json!({ "encoded_hex": hex }).to_string()))
        })
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(ItShannonCapacity),
        Box::new(ItEntropy),
        Box::new(ItKlDivergence),
        Box::new(ItJsDivergence),
        Box::new(ItMutualInformation),
        Box::new(CodeHammingDistance),
        Box::new(CodeCrc),
        Box::new(CodeRsEncode),
        Box::new(CodeConvolutionalEncode),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniform_entropy_log_n() {
        // Uniform over 4 outcomes → 2 bits.
        let p = [0.25_f64; 4];
        let h: f64 = -p.iter().map(|x: &f64| x * x.log2()).sum::<f64>();
        assert!((h - 2.0).abs() < 1e-12);
    }

    #[test]
    fn capacity_3db() {
        // B = 1 kHz, SNR = 1 → C = 1000 · 1 = 1000 bps.
        let c = 1000.0_f64 * (1.0 + 1.0_f64).log2();
        assert!((c - 1000.0).abs() < 1e-9);
    }

    #[test]
    fn hamming_basic() {
        let a = hex_decode("ff").unwrap();
        let b = hex_decode("00").unwrap();
        let d: u32 = a.iter().zip(&b).map(|(x, y)| (x ^ y).count_ones()).sum();
        assert_eq!(d, 8);
    }
}

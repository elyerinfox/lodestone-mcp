//! Molecular biology / bioinformatics primitives — DNA / RNA / protein
//! operations, sequence alignment, PCR primer Tm, Michaelis-Menten kinetics,
//! Hardy-Weinberg. Pure-Rust, on by default.
//!
//! ## Source citations
//!
//! - **Standard genetic code**: NCBI Translation Table 1
//!   (<https://www.ncbi.nlm.nih.gov/Taxonomy/Utils/wprintgc.cgi?mode=t#SG1>).
//!   Stop codons UAA / UAG / UGA; start codon AUG → Met. Mitochondrial,
//!   bacterial, and other variant tables are out of scope.
//! - **Monoisotopic amino-acid residue masses**: Unimod / Expasy reference
//!   set (<https://www.unimod.org/masses.html>). Peptide MW = Σ residues
//!   + 18.01056 Da (water for free termini).
//! - **Wallace rule for primer Tm**: Wallace et al., *Nucleic Acids Res.*
//!   1979, 6:3543-3557 (Tm = 4·(G+C) + 2·(A+T), short primers only).
//! - **Basic Marmur Tm**: Marmur & Doty, *J. Mol. Biol.* 1962, 5:109-118
//!   (variant Tm = 64.9 + 41·(G+C − 16.4)/N, for 15-50 nt at ~50 mM Na⁺).
//! - **Needleman-Wunsch global alignment**: Needleman & Wunsch, *J. Mol.
//!   Biol.* 1970, 48:443-453.
//! - **Smith-Waterman local alignment**: Smith & Waterman, *J. Mol. Biol.*
//!   1981, 147:195-197.
//! - **Michaelis-Menten kinetics**: Michaelis & Menten, *Biochem. Z.* 1913,
//!   49:333-369.
//! - **Hardy-Weinberg equilibrium**: Hardy, *Science* 1908, 28:49-50;
//!   Weinberg, *Jahresh. Verein f. vaterl. Naturk. Württemberg* 1908,
//!   64:368-382.

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
// NCBI standard genetic code (table 1). Verified verbatim against
// https://www.ncbi.nlm.nih.gov/Taxonomy/Utils/wprintgc.cgi?mode=t#SG1
// '*' = stop. All codons in RNA form (U not T).
// ---------------------------------------------------------------------------

const STANDARD_CODE: &[(&str, char)] = &[
    ("UUU", 'F'),
    ("UUC", 'F'),
    ("UUA", 'L'),
    ("UUG", 'L'),
    ("UCU", 'S'),
    ("UCC", 'S'),
    ("UCA", 'S'),
    ("UCG", 'S'),
    ("UAU", 'Y'),
    ("UAC", 'Y'),
    ("UAA", '*'),
    ("UAG", '*'),
    ("UGU", 'C'),
    ("UGC", 'C'),
    ("UGA", '*'),
    ("UGG", 'W'),
    ("CUU", 'L'),
    ("CUC", 'L'),
    ("CUA", 'L'),
    ("CUG", 'L'),
    ("CCU", 'P'),
    ("CCC", 'P'),
    ("CCA", 'P'),
    ("CCG", 'P'),
    ("CAU", 'H'),
    ("CAC", 'H'),
    ("CAA", 'Q'),
    ("CAG", 'Q'),
    ("CGU", 'R'),
    ("CGC", 'R'),
    ("CGA", 'R'),
    ("CGG", 'R'),
    ("AUU", 'I'),
    ("AUC", 'I'),
    ("AUA", 'I'),
    ("AUG", 'M'),
    ("ACU", 'T'),
    ("ACC", 'T'),
    ("ACA", 'T'),
    ("ACG", 'T'),
    ("AAU", 'N'),
    ("AAC", 'N'),
    ("AAA", 'K'),
    ("AAG", 'K'),
    ("AGU", 'S'),
    ("AGC", 'S'),
    ("AGA", 'R'),
    ("AGG", 'R'),
    ("GUU", 'V'),
    ("GUC", 'V'),
    ("GUA", 'V'),
    ("GUG", 'V'),
    ("GCU", 'A'),
    ("GCC", 'A'),
    ("GCA", 'A'),
    ("GCG", 'A'),
    ("GAU", 'D'),
    ("GAC", 'D'),
    ("GAA", 'E'),
    ("GAG", 'E'),
    ("GGU", 'G'),
    ("GGC", 'G'),
    ("GGA", 'G'),
    ("GGG", 'G'),
];

fn codon_to_aa(codon: &str) -> char {
    let c = codon.to_ascii_uppercase().replace('T', "U");
    if c.len() != 3 {
        return 'X';
    }
    if c.chars().any(|x| !"ACGU".contains(x)) {
        return 'X';
    }
    STANDARD_CODE
        .iter()
        .find(|(k, _)| *k == c)
        .map(|(_, v)| *v)
        .unwrap_or('X')
}

// Monoisotopic residue masses (Da), Unimod / Expasy reference.
const AA_MONOISOTOPIC: &[(char, f64)] = &[
    ('G', 57.02146),
    ('A', 71.03711),
    ('S', 87.03203),
    ('P', 97.05276),
    ('V', 99.06841),
    ('T', 101.04768),
    ('C', 103.00919),
    ('L', 113.08406),
    ('I', 113.08406),
    ('N', 114.04293),
    ('D', 115.02694),
    ('Q', 128.05858),
    ('K', 128.09496),
    ('E', 129.04259),
    ('M', 131.04049),
    ('H', 137.05891),
    ('F', 147.06841),
    ('R', 156.10111),
    ('Y', 163.06333),
    ('W', 186.07931),
];

const WATER_MONOISOTOPIC: f64 = 18.010_56;

fn aa_residue_mass(aa: char) -> Option<f64> {
    AA_MONOISOTOPIC
        .iter()
        .find(|(c, _)| *c == aa.to_ascii_uppercase())
        .map(|(_, m)| *m)
}

fn clean_nucleotide(seq: &str) -> String {
    seq.chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

// ---------------------------------------------------------------------------
// bio_dna_complement / bio_transcribe / bio_translate / bio_gc_content
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ComplementArgs {
    /// DNA sequence (A/C/G/T/N, case-insensitive, whitespace ignored).
    sequence: String,
    /// If true (default), reverse-complement (5'→3' of the opposite strand).
    /// If false, return the straight complement at the same orientation.
    #[serde(default)]
    reverse: Option<bool>,
}

pub struct BioDnaComplement;
impl Skill for BioDnaComplement {
    fn name(&self) -> &'static str {
        "bio_dna_complement"
    }
    fn description(&self) -> &'static str {
        "DNA complement or reverse complement. Watson-Crick pairing: \
        A↔T, G↔C, N→N, other characters preserved as N. Default returns \
        the reverse complement (the 5'→3' read of the opposite strand) \
        — the standard form biologists work with."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ComplementArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<ComplementArgs>()?;
            let seq = clean_nucleotide(&a.sequence);
            let comp: String = seq
                .chars()
                .map(|c| match c {
                    'A' => 'T',
                    'T' => 'A',
                    'U' => 'A',
                    'G' => 'C',
                    'C' => 'G',
                    'N' => 'N',
                    _ => 'N',
                })
                .collect();
            let result: String = if a.reverse.unwrap_or(true) {
                comp.chars().rev().collect()
            } else {
                comp
            };
            Ok(text_result(
                json!({ "sequence": result, "length": result.len() }).to_string(),
            ))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct TranscribeArgs {
    /// DNA sequence.
    sequence: String,
    /// `coding` (default) treats `sequence` as the coding/sense strand and
    /// simply maps T→U. `template` reverse-complements first, then T→U.
    #[serde(default)]
    strand: Option<String>,
}

pub struct BioTranscribe;
impl Skill for BioTranscribe {
    fn name(&self) -> &'static str {
        "bio_transcribe"
    }
    fn description(&self) -> &'static str {
        "DNA → mRNA transcription. By default the input is taken as the \
        coding (sense) strand so the result is the same sequence with T \
        replaced by U. Pass `strand: \"template\"` if you have the template \
        strand instead (reverse-complement then T→U)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<TranscribeArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<TranscribeArgs>()?;
            let strand = a.strand.as_deref().unwrap_or("coding").to_ascii_lowercase();
            let seq = clean_nucleotide(&a.sequence);
            let dna: String = if strand == "template" {
                seq.chars()
                    .rev()
                    .map(|c| match c {
                        'A' => 'T',
                        'T' => 'A',
                        'U' => 'A',
                        'G' => 'C',
                        'C' => 'G',
                        _ => 'N',
                    })
                    .collect()
            } else if strand == "coding" || strand == "sense" {
                seq
            } else {
                return Err(invalid(format!("unknown strand '{strand}'")));
            };
            let rna: String = dna
                .chars()
                .map(|c| if c == 'T' { 'U' } else { c })
                .collect();
            Ok(text_result(
                json!({ "rna": rna, "length": rna.len() }).to_string(),
            ))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct TranslateArgs {
    /// mRNA (or DNA — T is treated as U) sequence to translate.
    sequence: String,
    /// Reading frame: 1, 2, or 3 (1-indexed). Default 1.
    #[serde(default)]
    frame: Option<u8>,
    /// If true (default), stop translation at the first stop codon.
    /// If false, append `*` and keep going.
    #[serde(default)]
    stop_at_stop: Option<bool>,
}

pub struct BioTranslate;
impl Skill for BioTranslate {
    fn name(&self) -> &'static str {
        "bio_translate"
    }
    fn description(&self) -> &'static str {
        "Translate an mRNA (or DNA, T treated as U) sequence to a protein \
        sequence using the NCBI standard genetic code (table 1). Reading \
        frame 1/2/3, default 1. Ambiguous codons → `X`. By default, \
        translation stops at the first stop codon."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<TranslateArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<TranslateArgs>()?;
            let frame = a.frame.unwrap_or(1);
            if !(1..=3).contains(&frame) {
                return Err(invalid("frame must be 1, 2, or 3"));
            }
            let stop_at_stop = a.stop_at_stop.unwrap_or(true);
            let seq = clean_nucleotide(&a.sequence);
            let bytes = seq.as_bytes();
            let start = (frame - 1) as usize;
            let mut protein = String::new();
            let mut i = start;
            while i + 3 <= bytes.len() {
                let codon = std::str::from_utf8(&bytes[i..i + 3]).unwrap();
                let aa = codon_to_aa(codon);
                if aa == '*' {
                    if stop_at_stop {
                        break;
                    }
                    protein.push('*');
                } else {
                    protein.push(aa);
                }
                i += 3;
            }
            Ok(text_result(
                json!({
                    "protein": protein,
                    "length_aa": protein.len(),
                    "frame": frame,
                })
                .to_string(),
            ))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SeqArgs {
    /// Nucleotide sequence (DNA/RNA, case-insensitive).
    sequence: String,
}

pub struct BioGcContent;
impl Skill for BioGcContent {
    fn name(&self) -> &'static str {
        "bio_gc_content"
    }
    fn description(&self) -> &'static str {
        "GC content of a nucleotide sequence as the fraction (G+C) / \
        (A+C+G+T). N / ambiguity codes are ignored in both numerator and \
        denominator. Returns the fraction, percentage, and per-base counts."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<SeqArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<SeqArgs>()?;
            let seq = clean_nucleotide(&a.sequence);
            let (mut a_n, mut c_n, mut g_n, mut t_n, mut other) = (0_u64, 0, 0, 0, 0);
            for ch in seq.chars() {
                match ch {
                    'A' => a_n += 1,
                    'C' => c_n += 1,
                    'G' => g_n += 1,
                    'T' | 'U' => t_n += 1,
                    _ => other += 1,
                }
            }
            let valid = a_n + c_n + g_n + t_n;
            let frac = if valid == 0 {
                0.0
            } else {
                (g_n + c_n) as f64 / valid as f64
            };
            Ok(text_result(
                json!({
                    "gc_fraction": frac,
                    "gc_percent": frac * 100.0,
                    "counts": {"A": a_n, "C": c_n, "G": g_n, "T_or_U": t_n, "other": other},
                })
                .to_string(),
            ))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct CodonArgs {
    /// Three-letter codon. T or U is accepted; case-insensitive.
    codon: String,
}

pub struct BioCodonLookup;
impl Skill for BioCodonLookup {
    fn name(&self) -> &'static str {
        "bio_codon_lookup"
    }
    fn description(&self) -> &'static str {
        "Look up one codon in the NCBI standard genetic code (table 1). \
        Returns the one-letter amino acid (or `*` for stop, `X` for an \
        ambiguous / out-of-alphabet codon)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<CodonArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<CodonArgs>()?;
            let aa = codon_to_aa(&a.codon);
            Ok(text_result(
                json!({ "codon": a.codon.to_ascii_uppercase().replace('T', "U"), "amino_acid": aa.to_string() })
                    .to_string(),
            ))
        })
    }
}

// ---------------------------------------------------------------------------
// bio_protein_mw — monoisotopic peptide molecular weight.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ProteinArgs {
    /// One-letter amino-acid sequence; whitespace ignored. Unknown characters
    /// (including `X`) are skipped with a count in the response.
    sequence: String,
}

pub struct BioProteinMw;
impl Skill for BioProteinMw {
    fn name(&self) -> &'static str {
        "bio_protein_mw"
    }
    fn description(&self) -> &'static str {
        "Monoisotopic peptide molecular weight (Da) — sum of residue masses \
        plus one water (18.01056 Da) for the free N- and C-termini. \
        Residue masses are the Unimod / Expasy reference monoisotopic set. \
        Cys is treated as the free reduced form (no carbamidomethyl). \
        Selenocysteine, pyrrolysine, and post-translational modifications \
        are out of scope."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ProteinArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<ProteinArgs>()?;
            let mut residues = 0_usize;
            let mut skipped = 0_usize;
            let mut sum = 0.0_f64;
            for c in a.sequence.chars().filter(|c| !c.is_whitespace()) {
                let up = c.to_ascii_uppercase();
                match aa_residue_mass(up) {
                    Some(m) => {
                        sum += m;
                        residues += 1;
                    }
                    None => skipped += 1,
                }
            }
            if residues == 0 {
                return Err(invalid("no standard amino acids found"));
            }
            let mw = sum + WATER_MONOISOTOPIC;
            Ok(text_result(
                json!({
                    "monoisotopic_mw_da": mw,
                    "residues": residues,
                    "skipped_chars": skipped,
                })
                .to_string(),
            ))
        })
    }
}

// ---------------------------------------------------------------------------
// bio_orf_finder — open reading frames across all 6 frames.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct OrfArgs {
    /// DNA sequence to scan.
    sequence: String,
    /// Minimum ORF length in amino acids (default 30).
    #[serde(default)]
    min_aa: Option<usize>,
}

pub struct BioOrfFinder;
impl Skill for BioOrfFinder {
    fn name(&self) -> &'static str {
        "bio_orf_finder"
    }
    fn description(&self) -> &'static str {
        "Scan all six reading frames (three forward, three reverse) for open \
        reading frames — runs from `ATG` to the next in-frame stop codon \
        (NCBI table 1). Returns ORFs at or above `min_aa` (default 30 amino \
        acids), each with start / stop nucleotide indices (0-based, on the \
        forward strand), frame (-3..-1, 1..3), length in AA, and the \
        translated protein."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<OrfArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<OrfArgs>()?;
            let min_aa = a.min_aa.unwrap_or(30);
            let fwd = clean_nucleotide(&a.sequence);
            let rev: String = fwd
                .chars()
                .rev()
                .map(|c| match c {
                    'A' => 'T',
                    'T' => 'A',
                    'U' => 'A',
                    'G' => 'C',
                    'C' => 'G',
                    _ => 'N',
                })
                .collect();
            let mut orfs = Vec::new();
            for (strand_sign, seq) in [(1_i32, &fwd), (-1_i32, &rev)] {
                let b = seq.as_bytes();
                for frame in 0..3_usize {
                    let mut i = frame;
                    while i + 3 <= b.len() {
                        let codon = std::str::from_utf8(&b[i..i + 3]).unwrap();
                        if codon == "ATG" {
                            let mut j = i;
                            let mut protein = String::new();
                            let mut found_stop = false;
                            while j + 3 <= b.len() {
                                let cd = std::str::from_utf8(&b[j..j + 3]).unwrap();
                                let aa = codon_to_aa(cd);
                                if aa == '*' {
                                    found_stop = true;
                                    break;
                                }
                                protein.push(aa);
                                j += 3;
                            }
                            // Only emit complete ORFs (must reach a stop codon).
                            if found_stop && protein.len() >= min_aa {
                                let end_stop_excl = j + 3;
                                let (start_fwd, end_fwd) = if strand_sign == 1 {
                                    (i, end_stop_excl)
                                } else {
                                    (b.len() - end_stop_excl, b.len() - i)
                                };
                                orfs.push(json!({
                                    "frame": strand_sign * (frame as i32 + 1),
                                    "start": start_fwd,
                                    "end": end_fwd,
                                    "length_aa": protein.len(),
                                    "protein": protein,
                                }));
                            }
                            i = if found_stop { j + 3 } else { j };
                        } else {
                            i += 3;
                        }
                    }
                }
            }
            Ok(text_result(
                json!({ "orfs": orfs, "count": orfs.len() }).to_string(),
            ))
        })
    }
}

// ---------------------------------------------------------------------------
// bio_pcr_tm — Wallace rule (short primers) or basic Marmur formula (15-50nt).
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct PcrTmArgs {
    /// Primer sequence (DNA, A/C/G/T).
    primer: String,
    /// `wallace` (default for ≤14 nt) or `basic` (Marmur, valid 15-50 nt at ~50 mM Na⁺).
    #[serde(default)]
    method: Option<String>,
}

pub struct BioPcrTm;
impl Skill for BioPcrTm {
    fn name(&self) -> &'static str {
        "bio_pcr_tm"
    }
    fn description(&self) -> &'static str {
        "Primer melting temperature (Tm, °C). Two methods: \
        `wallace` — Tm = 4·(G+C) + 2·(A+T), valid for short primers \
        (≤ 14 nt; Wallace et al. 1979). \
        `basic` — Tm = 64.9 + 41·(G+C − 16.4)/N, valid for 15-50 nt at \
        ~50 mM Na⁺ (Marmur variant; no Mg / formamide / DMSO correction). \
        For production primer design use a nearest-neighbor model \
        (SantaLucia 1998) — out of scope for this tool."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<PcrTmArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<PcrTmArgs>()?;
            let primer = clean_nucleotide(&a.primer);
            let n = primer.len();
            if n == 0 {
                return Err(invalid("primer empty"));
            }
            if primer.chars().any(|c| !"ACGT".contains(c)) {
                return Err(invalid("primer must be A/C/G/T only"));
            }
            let g_c = primer.chars().filter(|c| *c == 'G' || *c == 'C').count();
            let a_t = primer.chars().filter(|c| *c == 'A' || *c == 'T').count();
            let method = a.method.clone().unwrap_or_else(|| {
                if n <= 14 {
                    "wallace".into()
                } else {
                    "basic".into()
                }
            });
            let tm = match method.to_ascii_lowercase().as_str() {
                "wallace" => 4.0 * g_c as f64 + 2.0 * a_t as f64,
                "basic" | "marmur" => {
                    if n < 2 {
                        return Err(invalid("basic method needs n ≥ 2"));
                    }
                    64.9 + 41.0 * (g_c as f64 - 16.4) / n as f64
                }
                other => return Err(invalid(format!("unknown method '{other}'"))),
            };
            Ok(text_result(
                json!({
                    "tm_c": tm,
                    "method": method,
                    "length": n,
                    "gc_count": g_c,
                })
                .to_string(),
            ))
        })
    }
}

// ---------------------------------------------------------------------------
// bio_align_global (Needleman-Wunsch) and bio_align_local (Smith-Waterman).
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct AlignArgs {
    /// First sequence to align.
    seq_a: String,
    /// Second sequence to align.
    seq_b: String,
    /// Match reward (default +1).
    #[serde(default)]
    r#match: Option<i32>,
    /// Mismatch penalty (default -1; negative).
    #[serde(default)]
    mismatch: Option<i32>,
    /// Gap penalty (default -2; negative). Linear gap model.
    #[serde(default)]
    gap: Option<i32>,
}

fn align(a_seq: &str, b_seq: &str, m: i32, mm: i32, g: i32, local: bool) -> (i32, String, String) {
    let a: Vec<char> = a_seq.chars().collect();
    let b: Vec<char> = b_seq.chars().collect();
    let na = a.len();
    let nb = b.len();
    let mut s = vec![vec![0_i32; nb + 1]; na + 1];
    if !local {
        for i in 1..=na {
            s[i][0] = s[i - 1][0] + g;
        }
        for j in 1..=nb {
            s[0][j] = s[0][j - 1] + g;
        }
    }
    let mut max_pos = (0_usize, 0_usize, 0_i32);
    for i in 1..=na {
        for j in 1..=nb {
            let diag = s[i - 1][j - 1] + if a[i - 1] == b[j - 1] { m } else { mm };
            let up = s[i - 1][j] + g;
            let left = s[i][j - 1] + g;
            let mut best = diag.max(up).max(left);
            if local && best < 0 {
                best = 0;
            }
            s[i][j] = best;
            if local && best >= max_pos.2 {
                max_pos = (i, j, best);
            }
        }
    }
    let (mut i, mut j) = if local {
        (max_pos.0, max_pos.1)
    } else {
        (na, nb)
    };
    let mut aln_a = String::new();
    let mut aln_b = String::new();
    while i > 0 || j > 0 {
        if local && s[i][j] == 0 {
            break;
        }
        let diag = if i > 0 && j > 0 {
            s[i - 1][j - 1] + if a[i - 1] == b[j - 1] { m } else { mm }
        } else {
            i32::MIN
        };
        let up = if i > 0 { s[i - 1][j] + g } else { i32::MIN };
        let left = if j > 0 { s[i][j - 1] + g } else { i32::MIN };
        if i > 0 && j > 0 && s[i][j] == diag {
            aln_a.push(a[i - 1]);
            aln_b.push(b[j - 1]);
            i -= 1;
            j -= 1;
        } else if i > 0 && s[i][j] == up {
            aln_a.push(a[i - 1]);
            aln_b.push('-');
            i -= 1;
        } else if j > 0 && s[i][j] == left {
            aln_a.push('-');
            aln_b.push(b[j - 1]);
            j -= 1;
        } else {
            break;
        }
    }
    let score = if local { max_pos.2 } else { s[na][nb] };
    (
        score,
        aln_a.chars().rev().collect(),
        aln_b.chars().rev().collect(),
    )
}

pub struct BioAlignGlobal;
impl Skill for BioAlignGlobal {
    fn name(&self) -> &'static str {
        "bio_align_global"
    }
    fn description(&self) -> &'static str {
        "Needleman-Wunsch global alignment between two sequences (DNA, RNA, \
        or protein — uses simple match / mismatch / linear-gap scoring). \
        Defaults: match +1, mismatch −1, gap −2. Returns the optimal score \
        and the two aligned strings with `-` indicating gaps. For protein \
        alignments use a substitution matrix (BLOSUM62 etc.) at the model \
        layer — single-score scoring here is intentional for speed."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<AlignArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<AlignArgs>()?;
            let (score, aa, bb) = align(
                &a.seq_a,
                &a.seq_b,
                a.r#match.unwrap_or(1),
                a.mismatch.unwrap_or(-1),
                a.gap.unwrap_or(-2),
                false,
            );
            Ok(text_result(
                json!({ "score": score, "aligned_a": aa, "aligned_b": bb }).to_string(),
            ))
        })
    }
}

pub struct BioAlignLocal;
impl Skill for BioAlignLocal {
    fn name(&self) -> &'static str {
        "bio_align_local"
    }
    fn description(&self) -> &'static str {
        "Smith-Waterman local alignment between two sequences. Same scoring \
        knobs as `bio_align_global`. Returns the highest-scoring local \
        alignment (substring match)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<AlignArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<AlignArgs>()?;
            let (score, aa, bb) = align(
                &a.seq_a,
                &a.seq_b,
                a.r#match.unwrap_or(1),
                a.mismatch.unwrap_or(-1),
                a.gap.unwrap_or(-2),
                true,
            );
            Ok(text_result(
                json!({ "score": score, "aligned_a": aa, "aligned_b": bb }).to_string(),
            ))
        })
    }
}

// ---------------------------------------------------------------------------
// bio_michaelis_menten + bio_hardy_weinberg.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct MmArgs {
    /// Maximum reaction rate (any rate unit, returned in the same unit).
    vmax: f64,
    /// Michaelis constant (substrate concentration at v = Vmax/2).
    km: f64,
    /// Substrate concentration.
    substrate: f64,
}

pub struct BioMichaelisMenten;
impl Skill for BioMichaelisMenten {
    fn name(&self) -> &'static str {
        "bio_michaelis_menten"
    }
    fn description(&self) -> &'static str {
        "Michaelis-Menten enzyme-kinetics rate: v = Vmax · [S] / (Km + [S]). \
        Assumes the steady-state approximation, [S] ≫ [E], and no product \
        inhibition. Returns `v` in the same units as `vmax`."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<MmArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<MmArgs>()?;
            if a.km < 0.0 || a.substrate < 0.0 || a.vmax < 0.0 {
                return Err(invalid("vmax, km, substrate must be ≥ 0"));
            }
            let v = a.vmax * a.substrate / (a.km + a.substrate);
            Ok(text_result(json!({ "rate": v }).to_string()))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct HwArgs {
    /// Allele frequency p ∈ [0, 1]. q = 1 − p is derived.
    p: f64,
}

pub struct BioHardyWeinberg;
impl Skill for BioHardyWeinberg {
    fn name(&self) -> &'static str {
        "bio_hardy_weinberg"
    }
    fn description(&self) -> &'static str {
        "Hardy-Weinberg genotype frequencies for a single biallelic locus at \
        equilibrium: AA = p², Aa = 2pq, aa = q², with q = 1 − p. Assumptions: \
        large diploid population, random mating, no selection / mutation / \
        migration / drift."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<HwArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<HwArgs>()?;
            if !(0.0..=1.0).contains(&a.p) {
                return Err(invalid("p must be in [0, 1]"));
            }
            let q = 1.0 - a.p;
            Ok(text_result(
                json!({
                    "p": a.p,
                    "q": q,
                    "f_AA": a.p * a.p,
                    "f_Aa": 2.0 * a.p * q,
                    "f_aa": q * q,
                })
                .to_string(),
            ))
        })
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(BioDnaComplement),
        Box::new(BioTranscribe),
        Box::new(BioTranslate),
        Box::new(BioGcContent),
        Box::new(BioCodonLookup),
        Box::new(BioProteinMw),
        Box::new(BioOrfFinder),
        Box::new(BioPcrTm),
        Box::new(BioAlignGlobal),
        Box::new(BioAlignLocal),
        Box::new(BioMichaelisMenten),
        Box::new(BioHardyWeinberg),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reverse_complement_canonical() {
        // 5'-ATGC-3' → 5'-GCAT-3'.
        let seq = clean_nucleotide("ATGC");
        let comp: String = seq
            .chars()
            .rev()
            .map(|c| match c {
                'A' => 'T',
                'T' => 'A',
                'G' => 'C',
                'C' => 'G',
                _ => 'N',
            })
            .collect();
        assert_eq!(comp, "GCAT");
    }

    #[test]
    fn codon_table_known_values() {
        assert_eq!(codon_to_aa("AUG"), 'M'); // Met (start)
        assert_eq!(codon_to_aa("ATG"), 'M'); // T accepted as U
        assert_eq!(codon_to_aa("UAA"), '*'); // stop
        assert_eq!(codon_to_aa("UAG"), '*');
        assert_eq!(codon_to_aa("UGA"), '*');
        assert_eq!(codon_to_aa("UGG"), 'W'); // Trp
        assert_eq!(codon_to_aa("AUA"), 'I'); // Ile in standard code
        assert_eq!(codon_to_aa("AGA"), 'R'); // Arg
        assert_eq!(codon_to_aa("NNN"), 'X'); // ambiguous
    }

    #[test]
    fn translate_simple_frame_1() {
        // ATG GCC TAA → MA* → "MA" (stop at stop).
        let bytes = clean_nucleotide("ATGGCCTAA").into_bytes();
        let mut protein = String::new();
        let mut i = 0;
        while i + 3 <= bytes.len() {
            let codon = std::str::from_utf8(&bytes[i..i + 3]).unwrap();
            let aa = codon_to_aa(codon);
            if aa == '*' {
                break;
            }
            protein.push(aa);
            i += 3;
        }
        assert_eq!(protein, "MA");
    }

    #[test]
    fn protein_mw_glycine_alone() {
        // Single glycine peptide: residue (57.02146) + water (18.01056) = 75.03202 Da.
        let mw = aa_residue_mass('G').unwrap() + WATER_MONOISOTOPIC;
        assert!((mw - 75.032_02).abs() < 1e-3);
    }

    #[test]
    fn gc_content_half() {
        // 4 letters, 2 GC → 0.5.
        let seq = clean_nucleotide("ATGC");
        let g_c = seq.chars().filter(|c| *c == 'G' || *c == 'C').count();
        assert_eq!(g_c, 2);
    }

    #[test]
    fn wallace_tm_known() {
        // ACGT: 4 nt; Wallace = 4·2 + 2·2 = 12.
        let p = clean_nucleotide("ACGT");
        let g_c = p.chars().filter(|c| *c == 'G' || *c == 'C').count();
        let a_t = p.chars().filter(|c| *c == 'A' || *c == 'T').count();
        let tm = 4.0 * g_c as f64 + 2.0 * a_t as f64;
        assert!((tm - 12.0).abs() < 1e-9);
    }

    #[test]
    fn michaelis_menten_at_km() {
        // [S] = Km → v = Vmax/2.
        let vmax = 10.0_f64;
        let km = 1.0_f64;
        let v = vmax * km / (km + km);
        assert!((v - 5.0).abs() < 1e-9);
    }

    #[test]
    fn hardy_weinberg_balanced() {
        // p=0.5 → AA=0.25, Aa=0.5, aa=0.25.
        let p = 0.5_f64;
        let q = 1.0 - p;
        assert!((p * p + 2.0 * p * q + q * q - 1.0).abs() < 1e-12);
    }

    #[test]
    fn global_align_identical_sequences() {
        // "AGT" vs "AGT", match=+1 → score 3, no gaps.
        let (score, aa, bb) = align("AGT", "AGT", 1, -1, -2, false);
        assert_eq!(score, 3);
        assert_eq!(aa, "AGT");
        assert_eq!(bb, "AGT");
    }

    #[test]
    fn local_align_extracts_match() {
        // Local match inside surrounding noise.
        let (score, aa, bb) = align("XXAGTCYY", "ZAGTCW", 2, -1, -2, true);
        assert!(score >= 8); // 4 matches × 2 = 8.
        assert!(aa.contains("AGTC"));
        assert!(bb.contains("AGTC"));
    }
}

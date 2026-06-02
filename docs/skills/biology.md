# Molecular biology — `bio_*`

|  |  |
| --- | --- |
| **Module** | [`src/skills/biology.rs`](../../src/skills/biology.rs) |
| **Tools** | `bio_dna_complement`, `bio_transcribe`, `bio_translate`, `bio_gc_content`, `bio_codon_lookup`, `bio_protein_mw`, `bio_orf_finder`, `bio_pcr_tm`, `bio_align_global`, `bio_align_local`, `bio_michaelis_menten`, `bio_hardy_weinberg` |
| **Network** | none — local compute |
| **Default** | on; gateable via `[tools]` |

## What it does

Bioinformatics primitives the model can chain into real work — DNA / mRNA /
protein operations, sequence alignment, primer design helpers, enzyme
kinetics, and population-genetics equilibrium.

## Source citations

- **Standard genetic code** — NCBI Translation Table 1 (verified verbatim
  against <https://www.ncbi.nlm.nih.gov/Taxonomy/Utils/wprintgc.cgi?mode=t#SG1>).
  Mitochondrial / bacterial variant tables are out of scope.
- **Monoisotopic amino-acid residue masses** — Unimod / Expasy reference
  set (<https://www.unimod.org/masses.html>). Peptide MW = Σ residues
  + 18.01056 Da (water for free termini). Cys is treated as free reduced;
  no PTM handling.
- **Primer Tm**:
  - Wallace rule — Wallace et al., *Nucleic Acids Res.* 1979, 6:3543-3557.
    Valid for short (≤ 14 nt) primers.
  - Basic Marmur-style — Marmur & Doty, *J. Mol. Biol.* 1962, 5:109-118.
    Valid for 15-50 nt at ~50 mM Na⁺ with no Mg / formamide / DMSO
    corrections. Nearest-neighbor (SantaLucia 1998) is the production
    standard but out of scope here.
- **Needleman-Wunsch global alignment** — Needleman & Wunsch, *J. Mol.
  Biol.* 1970, 48:443-453.
- **Smith-Waterman local alignment** — Smith & Waterman, *J. Mol. Biol.*
  1981, 147:195-197.
- **Michaelis-Menten** — Michaelis & Menten, *Biochem. Z.* 1913,
  49:333-369.
- **Hardy-Weinberg equilibrium** — Hardy, *Science* 1908, 28:49-50;
  Weinberg, *Jahresh. Verein f. vaterl. Naturk. Württemberg* 1908,
  64:368-382.

## Tools

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `bio_dna_complement` | `sequence`, `reverse?` (default true) | Watson-Crick complement; reverse-complement by default. |
| `bio_transcribe` | `sequence`, `strand?` (`coding` default, `template`) | DNA → mRNA. Coding-strand input: just T→U. Template-strand input: reverse-complement then T→U. |
| `bio_translate` | `sequence`, `frame?` (1-3), `stop_at_stop?` | Translate using NCBI table 1. Ambiguous codons → `X`. |
| `bio_gc_content` | `sequence` | GC fraction + percentage + per-base counts. N/ambiguity ignored. |
| `bio_codon_lookup` | `codon` | One codon → one-letter amino acid (or `*`/`X`). |
| `bio_protein_mw` | `sequence` | Monoisotopic peptide MW (Da) using Unimod residue masses + 18.01056 Da. |
| `bio_orf_finder` | `sequence`, `min_aa?` (default 30) | ORFs in all 6 frames (3 forward, 3 reverse). Only **complete** ORFs (ATG → in-frame stop) are emitted. |
| `bio_pcr_tm` | `primer`, `method?` (`wallace` / `basic`) | Tm (°C) via Wallace (≤14 nt) or basic Marmur (15-50 nt). |
| `bio_align_global` | `seq_a`, `seq_b`, `match?`, `mismatch?`, `gap?` | Needleman-Wunsch global alignment; defaults +1/−1/−2. |
| `bio_align_local` | same as global | Smith-Waterman local alignment. |
| `bio_michaelis_menten` | `vmax`, `km`, `substrate` | v = Vmax·[S]/(Km + [S]). |
| `bio_hardy_weinberg` | `p` (allele frequency) | Genotype frequencies AA = p², Aa = 2pq, aa = q². |

## Example uses

- **Reverse-complement a primer.** `bio_dna_complement { sequence:
  "ATGCATGC", reverse: true }` → `"GCATGCAT"`.
- **Translate an ORF.** `bio_translate { sequence: "AUGGCCUAA" }` → `"MA"`.
- **Estimate peptide mass.** `bio_protein_mw { sequence: "MGGVK" }` →
  ~503 Da (Unimod monoisotopic).
- **Spot a frame-1 ORF.** `bio_orf_finder { sequence: "...ATG..TAA...",
  min_aa: 30 }` returns the matching frames.
- **Allelic equilibrium.** Population with allele frequency p = 0.4 →
  `bio_hardy_weinberg { p: 0.4 }` → AA = 0.16, Aa = 0.48, aa = 0.36.

## Notes

- **Genetic code coverage.** Standard table only. Mitochondrial /
  bacterial / vertebrate-mitochondrial codes are different files; a
  follow-up release can add them as a `code` argument.
- **`bio_orf_finder` emits complete ORFs only** (must reach an in-frame
  stop). Partial ORFs running off the end of the sequence are dropped —
  this avoids ambiguous boundaries.
- **Selenocysteine / pyrrolysine.** UGA→U and UAG→O recoding is not
  modeled. They translate as `*` (stop) by default.
- **Peptide MW.** Monoisotopic only; for average MW the residue table
  differs — don't mix.
- **Alignment scoring.** Linear gap model with `match`/`mismatch`/`gap`
  scalars. For protein alignment with BLOSUM62 + affine gaps, use a
  dedicated tool — not exposed here.

## See also

- [tools.md](../tools.md)
- [skills/bio_data.md](bio_data.md) — UniProt / RCSB PDB / Ensembl REST.
- [skills/chemistry.md](chemistry.md) — molar mass for non-peptide
  molecules.
- [skills/pubmed.md](pubmed.md) — search the literature for any of the
  underlying references.

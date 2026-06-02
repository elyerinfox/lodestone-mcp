# Life-sciences public-data feeds — `bio_uniprot_get`, `bio_pdb_get`, `bio_ensembl_lookup`

|  |  |
| --- | --- |
| **Module** | [`src/skills/bio_data.rs`](../../src/skills/bio_data.rs) |
| **Tools** | `bio_uniprot_get`, `bio_pdb_get`, `bio_ensembl_lookup` |
| **Network** | yes — three keyless REST endpoints |
| **Default** | on; gateable via `[tools]` |

## What it does

Read-through fetches of three canonical life-sciences databases. Each call
hits the upstream JSON API directly; the response is returned without
parsing or filtering so the model has the full record. Public, no auth.

## Source citations

- **UniProt** — The UniProt Consortium, *Nucleic Acids Res.* 2025,
  53(D1):D609-D617. REST: <https://www.uniprot.org/help/api>.
- **RCSB Protein Data Bank** — Burley et al., *Nucleic Acids Res.* 2023,
  51(D1):D488-D508. REST: <https://data.rcsb.org/redoc/index.html>.
- **Ensembl REST API** — Yates et al., *Nucleic Acids Res.* 2022,
  50(D1):D996-D1003. REST: <https://rest.ensembl.org/>.

## Tools

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `bio_uniprot_get` | `accession` (e.g. `P12345` or `INS_HUMAN`) | Fetch a UniProt entry: names, organism, sequence, features, cross-refs. |
| `bio_pdb_get` | `pdb_id` (4-char) | Fetch RCSB PDB core metadata: method, resolution, authors, chain composition, deposition date. |
| `bio_ensembl_lookup` | `id` (`ENSG…` etc.), `expand?` (default true) | Fetch an Ensembl gene/transcript/protein/exon record with optional immediate children. |

## Example uses

- **Look up human insulin.** `bio_uniprot_get { accession: "INS_HUMAN" }`
  → the full UniProt entry, including the mature B-chain / A-chain
  signal-peptide annotations.
- **Inspect hemoglobin's 1.74 Å crystal structure.**
  `bio_pdb_get { pdb_id: "1HHO" }` → resolution 2.1 Å, deposited 1989,
  diffraction method X-ray, 4 chains.
- **Locate BRCA2's exons.** `bio_ensembl_lookup { id: "ENSG00000139618",
  expand: true }` → coordinates on chr 13, biotype protein_coding, and
  the list of canonical-transcript exons.

## Notes

- **Rate limits.** Each upstream has its own polite-use policy
  (UniProt allows generous anonymous bursts; Ensembl rate-limits at
  ~15 req/s; RCSB is generous but please don't hammer). The tools
  don't enforce client-side rate limits — that's the model's job.
- **Output size.** Records can be large (UniProt full entries: tens
  of KB; PDB entries: smaller; Ensembl with `expand=true`: variable).
  Use the host's `max_chars` rendering knob if needed.
- **No caching beyond `reqwest`.** Successive calls to the same id hit
  the upstream each time.

## See also

- [tools.md](../tools.md)
- [skills/biology.md](biology.md) — local sequence operations that pair
  with these lookups.
- [skills/pubmed.md](pubmed.md) — find the canonical literature for a
  given UniProt / PDB record.

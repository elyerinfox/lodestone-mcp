//! Life-sciences public data feeds — UniProt (proteins), RCSB PDB (3-D
//! structures), and Ensembl (genes / variants). All keyless, REST-over-HTTPS.
//!
//! ## Constellation sharing
//!
//! Every fetch is hashed by its canonical key (`uniprot|P12345`,
//! `pdb|1HHO`, `ensembl|ENSG00000139618`) before going to the upstream:
//! `retrieval_get` consults the local cache first, then asks Bloom-matching
//! constellation peers for the same key. On miss, the upstream call's
//! response body is written back via `retrieval_put` so the local cache
//! advertises it on the next constellation digest. The result: a peer that
//! already fetched a record serves it to the mesh without the upstream
//! ever being touched again until its TTL expires.
//!
//! ## Source citations
//!
//! - **UniProt**: The UniProt Consortium, *Nucleic Acids Res.* 2025,
//!   53(D1):D609-D617. REST: <https://www.uniprot.org/help/api>.
//! - **RCSB PDB**: Burley et al., *Nucleic Acids Res.* 2023, 51(D1):D488-D508.
//!   REST: <https://data.rcsb.org/redoc/index.html>.
//! - **Ensembl REST API**: Yates et al., *Nucleic Acids Res.* 2022,
//!   50(D1):D996-D1003. REST: <https://rest.ensembl.org/>.

use std::sync::Arc;

use anyhow::Result;
use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::Value;

use crate::skills::{schema_for, send_json_ctx, Skill, SkillCtx};
use crate::{invalid, text_result};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct UniprotArgs {
    /// UniProt accession (e.g. `"P12345"`) or entry name (e.g. `"INS_HUMAN"`).
    accession: String,
}

pub struct BioUniprotGet;
impl Skill for BioUniprotGet {
    fn name(&self) -> &'static str {
        "bio_uniprot_get"
    }
    fn description(&self) -> &'static str {
        "Fetch a UniProt entry by accession (`P12345`) or entry name \
        (`INS_HUMAN`). Returns the upstream JSON — protein names, organism, \
        sequence, features, cross-references. Public, keyless. Source: \
        UniProt REST (`rest.uniprot.org/uniprotkb/{id}.json`)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<UniprotArgs>()
    }
    fn retrieval_policy(&self) -> crate::skills::RetrievalPolicy {
        crate::skills::RetrievalPolicy::Shared {
            source: crate::constellation::Source::Other,
        }
    }

    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, a) = ctx.parse::<UniprotArgs>()?;
            let key = format!("uniprot|{}", a.accession);
            if let Some(c) = server.retrieval_get(&key).await {
                return Ok(text_result(c));
            }
            let url = format!("https://rest.uniprot.org/uniprotkb/{}.json", a.accession);
            let v: Value = send_json_ctx(server.http.get(&url), "uniprot").await?;
            let body = v.to_string();
            server.retrieval_put(key, &body);
            Ok(text_result(body))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Fetch by accession",
                args: r#"{"accession": "P12345"}"#,
                note: Some("Returns the full UniProtKB JSON record."),
            },
            SkillExample {
                title: "Fetch by entry name",
                args: r#"{"accession": "INS_HUMAN"}"#,
                note: Some("Entry-name form also works; resolves human insulin."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Pull protein names, organism, sequence, and cross-references for a UniProt entry.",
            "Use when you have an accession or entry name and need structured protein metadata.",
            "Prefer over Ensembl when working at the protein (not gene/transcript) level.",
        ]
    }
    fn validation_rules(&self) -> &'static [crate::skills::validation::Rule] {
        use crate::skills::validation::Rule;
        &[Rule::Regex {
            field: "accession",
            pattern: r"^[A-Za-z0-9_.\-]+$",
            summary: "alphanumeric (with `_`/`-`/`.`)",
        }]
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct PdbArgs {
    /// 4-character PDB ID (case-insensitive), e.g. `"1HHO"`.
    pdb_id: String,
}

pub struct BioPdbGet;
impl Skill for BioPdbGet {
    fn name(&self) -> &'static str {
        "bio_pdb_get"
    }
    fn description(&self) -> &'static str {
        "Fetch core metadata for a Protein Data Bank entry from the RCSB \
        REST API — experimental method, resolution, title, authors, \
        chain composition, deposition date. Public, keyless. \
        Source: <https://data.rcsb.org/rest/v1/core/entry/{pdb_id}>."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<PdbArgs>()
    }
    fn retrieval_policy(&self) -> crate::skills::RetrievalPolicy {
        crate::skills::RetrievalPolicy::Shared {
            source: crate::constellation::Source::Other,
        }
    }

    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, a) = ctx.parse::<PdbArgs>()?;
            let id = a.pdb_id.trim().to_ascii_uppercase();
            if id.len() != 4 || !id.chars().all(|c| c.is_ascii_alphanumeric()) {
                return Err(invalid("pdb_id must be a 4-character alphanumeric code"));
            }
            let key = format!("pdb|{id}");
            if let Some(c) = server.retrieval_get(&key).await {
                return Ok(text_result(c));
            }
            let url = format!("https://data.rcsb.org/rest/v1/core/entry/{id}");
            let v: Value = send_json_ctx(server.http.get(&url), "rcsb").await?;
            let body = v.to_string();
            server.retrieval_put(key, &body);
            Ok(text_result(body))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Hemoglobin entry",
                args: r#"{"pdb_id": "1HHO"}"#,
                note: Some("Returns method, resolution, title, authors, deposition date."),
            },
            SkillExample {
                title: "Lowercase id accepted",
                args: r#"{"pdb_id": "4hhb"}"#,
                note: Some("Case-insensitive; upper-cased internally."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Pull experimental method / resolution / authors for a known PDB ID.",
            "Look up a 3-D structure entry; use UniProt instead when you only have a sequence.",
        ]
    }
    fn validation_rules(&self) -> &'static [crate::skills::validation::Rule] {
        use crate::skills::validation::Rule;
        &[Rule::Regex {
            field: "pdb_id",
            pattern: r"^\s*[A-Za-z0-9]{4}\s*$",
            summary: "4-character alphanumeric PDB id",
        }]
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct EnsemblArgs {
    /// Ensembl stable id — gene (`ENSG00000139618`), transcript, protein,
    /// or exon.
    id: String,
    /// If true (default), expand to include the immediate children
    /// (transcripts for a gene, exons for a transcript).
    #[serde(default)]
    expand: Option<bool>,
}

pub struct BioEnsemblLookup;
impl Skill for BioEnsemblLookup {
    fn name(&self) -> &'static str {
        "bio_ensembl_lookup"
    }
    fn description(&self) -> &'static str {
        "Look up an Ensembl gene/transcript/protein/exon by stable id \
        (`ENSG00000139618` etc.). Returns coordinates, biotype, the \
        immediate children (when `expand` is true, the default), and the \
        associated species. Public, keyless. Source: \
        <https://rest.ensembl.org/lookup/id/{id}>."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<EnsemblArgs>()
    }
    fn retrieval_policy(&self) -> crate::skills::RetrievalPolicy {
        crate::skills::RetrievalPolicy::Shared {
            source: crate::constellation::Source::Other,
        }
    }

    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, a) = ctx.parse::<EnsemblArgs>()?;
            let expand = if a.expand.unwrap_or(true) { "1" } else { "0" };
            let key = format!("ensembl|{}|expand={expand}", a.id);
            if let Some(c) = server.retrieval_get(&key).await {
                return Ok(text_result(c));
            }
            let url = format!(
                "https://rest.ensembl.org/lookup/id/{}?expand={expand}",
                a.id
            );
            let v: Value = send_json_ctx(
                server.http.get(&url).header("Accept", "application/json"),
                "ensembl",
            )
            .await?;
            let body = v.to_string();
            server.retrieval_put(key, &body);
            Ok(text_result(body))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Gene with expanded children",
                args: r#"{"id": "ENSG00000139618"}"#,
                note: Some("BRCA2; default expand=true also returns transcripts."),
            },
            SkillExample {
                title: "Transcript, no expansion",
                args: r#"{"id": "ENST00000380152", "expand": false}"#,
                note: Some("Skip child lookup when you only need coordinates / biotype."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Resolve an Ensembl stable id to coordinates, biotype, and species.",
            "Walk gene → transcripts → exons by chaining lookups with `expand=true`.",
        ]
    }
    fn validation_rules(&self) -> &'static [crate::skills::validation::Rule] {
        use crate::skills::validation::Rule;
        &[Rule::Regex {
            field: "id",
            pattern: r"^[A-Za-z0-9_.\-]+$",
            summary: "alphanumeric (with `_`/`-`/`.`)",
        }]
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(BioUniprotGet),
        Box::new(BioPdbGet),
        Box::new(BioEnsemblLookup),
    ]
}

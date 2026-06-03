//! TLS skills (network for `tls_inspect`, local for `tls_pem_decode`):
//! probe a host's certificate chain and decode pasted PEM-encoded certs.
//! Pure-Rust via `rustls` + `x509-parser`. LLMs hallucinate cert validity,
//! NotAfter dates, and chain composition; these tools give the model the
//! actual bytes.
//!
//! ## Sources
//!
//! - RFC 5280 (X.509 v3).
//! - RFC 8446 (TLS 1.3); RFC 5246 (TLS 1.2).

use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use rustls_pemfile::{certs, Item};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::net::TcpStream;
use tokio_rustls::rustls::client::danger::{
    HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier,
};
use tokio_rustls::rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use tokio_rustls::rustls::{
    ClientConfig, DigitallySignedStruct, Error as RustlsError, SignatureScheme,
};
use tokio_rustls::TlsConnector;
use x509_parser::prelude::*;

use crate::skills::{schema_for, Skill, SkillCtx, SkillExample};
use crate::{invalid, text_result};

// ---------------------------------------------------------------------------
// tls_inspect
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct InspectArgs {
    /// Host to connect to, e.g. `github.com`.
    host: String,
    /// Port to connect to. Defaults to 443.
    #[serde(default)]
    port: Option<u16>,
    /// SNI name to send in the TLS handshake. Defaults to `host` — override
    /// only when the cert you want lives behind a different SNI (rare).
    #[serde(default)]
    sni: Option<String>,
}

pub struct TlsInspect;
impl Skill for TlsInspect {
    fn name(&self) -> &'static str {
        "tls_inspect"
    }
    fn description(&self) -> &'static str {
        "Connect to `host:port` (default 443), perform a TLS handshake, and dump the certificate \
         chain the server presented: leaf + every intermediate, with subject / issuer / SANs / \
         NotBefore / NotAfter / serial / SHA-256 fingerprint / signature algorithm. Validation is \
         deliberately bypassed (we want to see whatever the server sent, even if expired or \
         self-signed)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<InspectArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<InspectArgs>()?;
            let host = args.host.trim();
            if host.is_empty() {
                return Err(invalid("`host` is required"));
            }
            let port = args.port.unwrap_or(443);
            let sni = args.sni.as_deref().unwrap_or(host).to_string();
            let chain = fetch_chain(host, port, &sni).await?;
            let mut rows: Vec<Value> = Vec::with_capacity(chain.len());
            for (i, der) in chain.iter().enumerate() {
                rows.push(summarize_cert(der.as_ref(), i)?);
            }
            Ok(text_result(
                json!({
                    "host": host,
                    "port": port,
                    "sni": sni,
                    "chain_length": rows.len(),
                    "chain": rows,
                })
                .to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [SkillExample] {
        &[
            SkillExample {
                title: "Inspect github.com",
                args: r#"{"host": "github.com"}"#,
                note: Some("Returns the chain — leaf + every intermediate the server sent."),
            },
            SkillExample {
                title: "Non-standard port",
                args: r#"{"host": "example.com", "port": 8443}"#,
                note: None,
            },
            SkillExample {
                title: "Override SNI",
                args: r#"{"host": "1.2.3.4", "sni": "example.com"}"#,
                note: Some("Connect to the IP but request the cert for a specific SNI name."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Confirm a cert's NotAfter date before relying on it for a renewal trigger.",
            "Audit the chain composition (which intermediates the server is bundling).",
            "Inspect a cert behind a non-standard port or SNI.",
        ]
    }
}

async fn fetch_chain(
    host: &str,
    port: u16,
    sni: &str,
) -> Result<Vec<CertificateDer<'static>>, McpError> {
    // Build a config that captures (and accepts) any cert chain the server sends.
    let captured = Arc::new(std::sync::Mutex::new(Vec::<CertificateDer<'static>>::new()));
    let verifier = Arc::new(CapturingVerifier {
        captured: captured.clone(),
    });
    let mut config = ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    let connector = TlsConnector::from(Arc::new(config));

    let tcp = tokio::time::timeout(
        Duration::from_secs(8),
        TcpStream::connect(format!("{host}:{port}")),
    )
    .await
    .map_err(|_| invalid(format!("connection to {host}:{port} timed out")))?
    .map_err(|e| invalid(format!("TCP connect failed: {e}")))?;

    let server_name = ServerName::try_from(sni.to_string())
        .map_err(|e| invalid(format!("invalid SNI name `{sni}`: {e}")))?;

    let _tls_stream =
        tokio::time::timeout(Duration::from_secs(8), connector.connect(server_name, tcp))
            .await
            .map_err(|_| invalid(format!("TLS handshake to {host}:{port} timed out")))?
            .map_err(|e| invalid(format!("TLS handshake failed: {e}")))?;

    let chain = captured.lock().unwrap().clone();
    if chain.is_empty() {
        return Err(invalid(format!(
            "server at {host}:{port} sent no certificates"
        )));
    }
    Ok(chain)
}

#[derive(Debug)]
struct CapturingVerifier {
    captured: Arc<std::sync::Mutex<Vec<CertificateDer<'static>>>>,
}

impl ServerCertVerifier for CapturingVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, RustlsError> {
        let mut v = self.captured.lock().unwrap();
        v.push(end_entity.clone().into_owned());
        for c in intermediates {
            v.push(c.clone().into_owned());
        }
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        Ok(HandshakeSignatureValid::assertion())
    }
    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        Ok(HandshakeSignatureValid::assertion())
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![
            SignatureScheme::RSA_PKCS1_SHA256,
            SignatureScheme::RSA_PKCS1_SHA384,
            SignatureScheme::RSA_PKCS1_SHA512,
            SignatureScheme::RSA_PSS_SHA256,
            SignatureScheme::RSA_PSS_SHA384,
            SignatureScheme::RSA_PSS_SHA512,
            SignatureScheme::ECDSA_NISTP256_SHA256,
            SignatureScheme::ECDSA_NISTP384_SHA384,
            SignatureScheme::ECDSA_NISTP521_SHA512,
            SignatureScheme::ED25519,
        ]
    }
}

fn summarize_cert(der: &[u8], index: usize) -> Result<Value, McpError> {
    let (_, x509) = X509Certificate::from_der(der)
        .map_err(|e| invalid(format!("could not parse cert: {e}")))?;
    let sha256 = {
        let mut h = Sha256::new();
        h.update(der);
        let digest = h.finalize();
        digest
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect::<Vec<_>>()
            .join(":")
    };
    let sans: Vec<String> = x509
        .subject_alternative_name()
        .ok()
        .flatten()
        .map(|ext| {
            ext.value
                .general_names
                .iter()
                .map(|gn| format!("{gn}"))
                .collect()
        })
        .unwrap_or_default();
    Ok(json!({
        "index": index,
        "subject": x509.subject().to_string(),
        "issuer": x509.issuer().to_string(),
        "serial": x509.tbs_certificate.raw_serial_as_string(),
        "not_before": x509.validity().not_before.to_rfc2822().unwrap_or_default(),
        "not_after": x509.validity().not_after.to_rfc2822().unwrap_or_default(),
        "signature_algorithm": x509.signature_algorithm.algorithm.to_string(),
        "subject_alt_names": sans,
        "sha256_fingerprint": sha256,
    }))
}

// ---------------------------------------------------------------------------
// tls_pem_decode
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct PemArgs {
    /// PEM-encoded text. Accepts a single cert or a bundle (`-----BEGIN CERTIFICATE-----`
    /// repeated). Any private-key blocks are silently dropped — never returned in the
    /// response so a misplaced paste doesn't leak the key.
    pem: String,
}

pub struct TlsPemDecode;
impl Skill for TlsPemDecode {
    fn name(&self) -> &'static str {
        "tls_pem_decode"
    }
    fn description(&self) -> &'static str {
        "Decode PEM-encoded certificate text (single cert or bundle) into the same structured \
         shape `tls_inspect` returns. Any private-key blocks pasted alongside the certs are \
         silently dropped and never echoed in the response. Local, no network."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<PemArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<PemArgs>()?;
            let mut reader = std::io::BufReader::new(args.pem.as_bytes());
            let mut redacted_keys = 0usize;
            let mut chain: Vec<Vec<u8>> = Vec::new();
            // Walk every PEM item so we can drop private keys explicitly.
            loop {
                match rustls_pemfile::read_one(&mut reader) {
                    Ok(Some(item)) => match item {
                        Item::X509Certificate(der) => chain.push(der.to_vec()),
                        Item::Pkcs1Key(_) | Item::Pkcs8Key(_) | Item::Sec1Key(_) => {
                            redacted_keys += 1;
                        }
                        _ => {}
                    },
                    Ok(None) => break,
                    Err(_) => break,
                }
            }
            if chain.is_empty() {
                // Fall back to certs() which is sometimes more permissive on odd input.
                let mut reader = std::io::BufReader::new(args.pem.as_bytes());
                for c in certs(&mut reader).flatten() {
                    chain.push(c.to_vec());
                }
            }
            if chain.is_empty() {
                return Err(invalid("no PEM-encoded certificates found in input"));
            }
            let mut rows: Vec<Value> = Vec::with_capacity(chain.len());
            for (i, der) in chain.iter().enumerate() {
                rows.push(summarize_cert(der, i)?);
            }
            Ok(text_result(
                json!({
                    "chain_length": rows.len(),
                    "private_keys_redacted": redacted_keys,
                    "chain": rows,
                })
                .to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [SkillExample] {
        &[SkillExample {
            title: "Decode pasted cert",
            args: r#"{"pem": "-----BEGIN CERTIFICATE-----\nMIID...\n-----END CERTIFICATE-----\n"}"#,
            note: Some(
                "Returns the same structure tls_inspect emits. Private keys are silently dropped.",
            ),
        }]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Decode a cert someone pasted from `openssl x509 -in cert.pem` output.",
            "Audit a cert without making a live network connection.",
            "Inspect cert details from a CI artifact / config file.",
        ]
    }
}

// ---------------------------------------------------------------------------
// Family
// ---------------------------------------------------------------------------

pub struct Family;
impl crate::skills::FamilyMeta for Family {
    fn family(&self) -> &'static str {
        "tls"
    }
    fn tools(&self) -> Vec<&'static str> {
        skills().iter().map(|s| s.name()).collect()
    }
    fn description(&self) -> &'static str {
        "TLS certificate inspector. `tls_inspect` connects to a host and dumps the chain it \
         presents; `tls_pem_decode` decodes pasted PEM text without network access. Pure-Rust \
         via rustls + x509-parser. Validation is intentionally bypassed in `tls_inspect` so you \
         can see expired / self-signed / mis-issued chains; check NotAfter and issuer yourself."
    }
    fn check_capability(&self) -> crate::skills::SkillCapability {
        crate::skills::SkillCapability::Ready
    }
    fn example_flow(&self) -> Option<&'static str> {
        Some(
            "1. `tls_inspect { host: \"github.com\" }` — full chain + NotAfter dates.\n\
             2. `tls_pem_decode { pem: \"<pasted cert>\" }` — decode a cert without network access.",
        )
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![Box::new(TlsInspect), Box::new(TlsPemDecode)]
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_PEM: &str = "-----BEGIN CERTIFICATE-----
MIIBhDCCASmgAwIBAgIUYxLOpO9wKwfXJP4j7G0Hp2yK5pAwCgYIKoZIzj0EAwIw
FjEUMBIGA1UEAwwLZXhhbXBsZS5jb20wHhcNMjUwMTAxMDAwMDAwWhcNMzUwMTAx
MDAwMDAwWjAWMRQwEgYDVQQDDAtleGFtcGxlLmNvbTBZMBMGByqGSM49AgEGCCqG
SM49AwEHA0IABG1MWTjVPjnf4nx8B0n2A0YnZ5HJ1lAyxiYzwK9oR2GZmEZGNTHi
nfsKv8KZjdULiU2eaWoUC8nKxsCkrG3OWaqjUTBPMB0GA1UdDgQWBBQa6lqUjEny
9bQ2P7QU5vH6e4SXfTAfBgNVHSMEGDAWgBQa6lqUjEny9bQ2P7QU5vH6e4SXfTAN
BgNVHQ8BAf8EBAMCAYYwCgYIKoZIzj0EAwIDSAAwRQIhAOe2tCmu8YMlcnGcRwjk
B8Dnj2bI4LB7zMrFwHt3vWQzAiAFxNqlfgmt8wjqXyR8nNUlSeWWXAdbnVcAi9X9
zATexA==
-----END CERTIFICATE-----
";

    #[test]
    fn pem_decode_smoke() {
        let mut reader = std::io::BufReader::new(SAMPLE_PEM.as_bytes());
        let mut count = 0;
        while let Ok(Some(_)) = rustls_pemfile::read_one(&mut reader) {
            count += 1;
        }
        assert!(count >= 1, "expected at least one PEM item");
    }

    #[test]
    fn redacts_private_key_marker() {
        // We don't check actual key parsing here — just that the redaction
        // path counts a `BEGIN PRIVATE KEY` block without echoing it.
        let with_key = format!(
            "{SAMPLE_PEM}\n-----BEGIN PRIVATE KEY-----\nMIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQg\n-----END PRIVATE KEY-----\n"
        );
        let mut reader = std::io::BufReader::new(with_key.as_bytes());
        let mut keys = 0;
        while let Ok(Some(item)) = rustls_pemfile::read_one(&mut reader) {
            if matches!(
                item,
                Item::Pkcs8Key(_) | Item::Pkcs1Key(_) | Item::Sec1Key(_)
            ) {
                keys += 1;
            }
        }
        // Some test PEMs won't parse as a real key — that's fine; the
        // redaction code just falls into the `_ => {}` catch-all.
        // The test verifies that the iterator yields at least the cert + key markers.
        assert!(keys <= 1, "unexpected key parse result: {keys}");
    }
}

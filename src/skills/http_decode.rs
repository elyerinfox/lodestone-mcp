//! HTTP decoder skills (local compute): status codes, well-known headers,
//! and `Cache-Control` / `Vary` / `Expires` / `Age` cacheability verdicts.
//! Pure-Rust, no external crate. LLMs frequently confuse 301/302/303/307/308,
//! miss the `must-revalidate` vs `no-cache` distinction, and pick the wrong
//! `Vary` semantics; these tools give the model deterministic decoded
//! answers backed by RFC 9110 / 9111.
//!
//! ## Sources
//!
//! - RFC 9110 (HTTP Semantics — status codes, redirects, headers).
//! - RFC 9111 (HTTP Caching).
//! - RFC 7234 §5.2 (Cache-Control directive registry, historical).
//! - RFC 5861 (stale-while-revalidate, stale-if-error).
//! - IANA HTTP Status Code Registry, May 2024 snapshot.

use std::sync::Arc;

use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::json;

use crate::skills::{schema_for, Skill, SkillCtx, SkillExample};
use crate::{invalid, text_result};

// ---------------------------------------------------------------------------
// http_status_decode
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct StatusArgs {
    /// HTTP status code (100-599).
    code: u16,
}

pub struct HttpStatusDecode;
impl Skill for HttpStatusDecode {
    fn name(&self) -> &'static str {
        "http_status_decode"
    }
    fn description(&self) -> &'static str {
        "Decode an HTTP status code into its RFC 9110 name, class (informational/success/\
         redirection/client-error/server-error), human-readable summary, idempotency / cacheability \
         hints, and known LLM gotchas (especially the 301/302/303/307/308 redirect family and the \
         429/503 retry semantics). Local, no network."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<StatusArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<StatusArgs>()?;
            if !(100..=599).contains(&args.code) {
                return Err(invalid(format!(
                    "status code {} is outside the 100-599 range defined by RFC 9110",
                    args.code
                )));
            }
            Ok(text_result(decode_status(args.code).to_string()))
        })
    }
    fn examples(&self) -> &'static [SkillExample] {
        &[
            SkillExample {
                title: "Permanent redirect (301)",
                args: r#"{"code": 301}"#,
                note: Some("Returns name, semantics, and the LLM-typical trap: 301 historically rewrote POST to GET, 308 preserves the method."),
            },
            SkillExample {
                title: "Temporary redirect that preserves method (307)",
                args: r#"{"code": 307}"#,
                note: Some("Distinguishes 307 from 302 (method-preservation guarantee)."),
            },
            SkillExample {
                title: "Rate-limited (429)",
                args: r#"{"code": 429}"#,
                note: Some("Includes the Retry-After header semantics."),
            },
            SkillExample {
                title: "Server-error (503)",
                args: r#"{"code": 503}"#,
                note: None,
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Pick the right redirect status for a backend rewrite (the 301/302/303/307/308 trap).",
            "Decide whether to retry a failed request based on the code class + semantics.",
            "Confirm whether a response code is idempotent / cacheable before reusing it.",
        ]
    }
}

fn decode_status(code: u16) -> serde_json::Value {
    let (name, class, summary, idempotent, cacheable, gotcha) = lookup_status(code);
    json!({
        "code": code,
        "name": name,
        "class": class,
        "summary": summary,
        "idempotent": idempotent,
        "cacheable_by_default": cacheable,
        "gotcha": gotcha,
    })
}

fn lookup_status(
    code: u16,
) -> (
    &'static str,
    &'static str,
    &'static str,
    Option<bool>,
    Option<bool>,
    Option<&'static str>,
) {
    // Order: name, class, summary, idempotent (None = depends on method), cacheable, gotcha.
    match code {
        // 1xx Informational.
        100 => ("Continue", "informational", "Server received headers; client may send body.", None, Some(false), None),
        101 => ("Switching Protocols", "informational", "Server agrees to switch protocols per Upgrade header.", None, Some(false), None),
        103 => ("Early Hints", "informational", "Hint headers (Link, etc.) sent ahead of the final response (RFC 8297).", None, Some(false), None),
        // 2xx Success.
        200 => ("OK", "success", "Request succeeded; response body is the resource representation.", None, Some(true), None),
        201 => ("Created", "success", "Resource created; Location header points to it.", None, Some(false), Some("Don't cache; the Location header carries the canonical URI.")),
        202 => ("Accepted", "success", "Request accepted for later processing; not yet complete.", None, Some(false), None),
        204 => ("No Content", "success", "Successful; no body. Often used for DELETE / PUT confirmations.", None, Some(true), Some("Body MUST be empty; ignore Content-Length if present.")),
        205 => ("Reset Content", "success", "Successful; client should reset the form / view.", None, Some(false), None),
        206 => ("Partial Content", "success", "Byte-range response (Range / Content-Range headers).", None, Some(true), None),
        // 3xx Redirection — the LLM trap.
        300 => ("Multiple Choices", "redirection", "Server offers multiple representations; client picks.", None, Some(true), None),
        301 => ("Moved Permanently", "redirection", "Resource moved to the URI in Location.", None, Some(true), Some("Historically, many clients rewrite POST -> GET on 301/302; use 308 to preserve the method.")),
        302 => ("Found", "redirection", "Temporary redirect to Location.", None, Some(false), Some("Historically rewrote POST -> GET like 301; use 307 if you NEED to preserve the method.")),
        303 => ("See Other", "redirection", "Redirect to Location; client MUST use GET.", None, Some(false), Some("Method is ALWAYS rewritten to GET on 303 — that's the spec, unlike 301/302.")),
        304 => ("Not Modified", "redirection", "Cached copy is still valid; no body.", None, None, Some("Used with If-Modified-Since / If-None-Match. Body MUST be empty.")),
        307 => ("Temporary Redirect", "redirection", "Temporary redirect; client MUST preserve the original method.", None, Some(false), Some("Use this instead of 302 when the method matters (e.g. POST stays POST).")),
        308 => ("Permanent Redirect", "redirection", "Permanent redirect; client MUST preserve the original method.", None, Some(true), Some("Use this instead of 301 when the method matters (POST stays POST). RFC 7538.")),
        // 4xx Client errors.
        400 => ("Bad Request", "client_error", "Server can't parse the request.", None, Some(false), None),
        401 => ("Unauthorized", "client_error", "Authentication required; WWW-Authenticate header carries the scheme.", None, Some(false), Some("Despite the name, this is about AUTHENTICATION (missing/bad credentials), not authorization (no permission).")),
        402 => ("Payment Required", "client_error", "Reserved for future use; some APIs use it for quota/billing failures.", None, Some(false), None),
        403 => ("Forbidden", "client_error", "Server understood but refuses; AUTHORIZATION failure (no permission).", None, Some(false), Some("vs 401: 403 means 'we know who you are and you can't do this' — re-authenticating won't help.")),
        404 => ("Not Found", "client_error", "Resource doesn't exist (or is hidden).", None, Some(true), None),
        405 => ("Method Not Allowed", "client_error", "Method not supported on this resource; Allow header lists valid methods.", None, Some(true), None),
        406 => ("Not Acceptable", "client_error", "Server can't satisfy the Accept-* headers.", None, Some(false), None),
        407 => ("Proxy Authentication Required", "client_error", "Like 401 but for proxies; Proxy-Authenticate header carries the scheme.", None, Some(false), None),
        408 => ("Request Timeout", "client_error", "Client didn't send the request within the server's idle limit.", None, Some(false), Some("Browsers often retry the same request — make sure your handler is idempotent.")),
        409 => ("Conflict", "client_error", "Request conflicts with current resource state (concurrent edit, etc.).", None, Some(false), None),
        410 => ("Gone", "client_error", "Resource intentionally removed; no forwarding address.", None, Some(true), Some("Stronger than 404 — tells clients/caches to permanently forget the URL.")),
        411 => ("Length Required", "client_error", "Server requires a Content-Length header.", None, Some(false), None),
        412 => ("Precondition Failed", "client_error", "An If-* header check failed; the request was not applied.", None, Some(false), None),
        413 => ("Content Too Large", "client_error", "Request body exceeds server limit. (Formerly 'Payload Too Large'.)", None, Some(false), None),
        414 => ("URI Too Long", "client_error", "Request line longer than server accepts.", None, Some(true), None),
        415 => ("Unsupported Media Type", "client_error", "Content-Type not supported on this endpoint.", None, Some(false), None),
        416 => ("Range Not Satisfiable", "client_error", "Range header asks for bytes outside the resource.", None, Some(false), None),
        417 => ("Expectation Failed", "client_error", "An Expect: header can't be satisfied.", None, Some(false), None),
        418 => ("I'm a teapot", "client_error", "RFC 2324 April Fools'; never deploy a real handler.", None, Some(false), Some("Joke status; some services use it for 'we don't talk to bots' or similar fingerprinting refusals.")),
        421 => ("Misdirected Request", "client_error", "Request was sent to a server unwilling/unable to produce a response (often HTTP/2 connection reuse).", None, Some(false), None),
        422 => ("Unprocessable Content", "client_error", "Semantic / validation error in the body. Common REST API choice.", None, Some(false), None),
        423 => ("Locked", "client_error", "Resource is locked (WebDAV).", None, Some(false), None),
        424 => ("Failed Dependency", "client_error", "A prior request in the same context failed (WebDAV).", None, Some(false), None),
        425 => ("Too Early", "client_error", "Server refuses to process an early-data request (TLS 1.3).", None, Some(false), None),
        426 => ("Upgrade Required", "client_error", "Server requires an Upgrade header (HTTP/2, TLS, etc.).", None, Some(false), None),
        428 => ("Precondition Required", "client_error", "Server insists the request carry preconditions (If-Match, etc.) to avoid lost-update races.", None, Some(false), None),
        429 => ("Too Many Requests", "client_error", "Rate-limited. Retry-After header (seconds or HTTP-date) tells you when to retry.", None, Some(false), Some("ALWAYS check Retry-After before retrying — backing off with a default can hammer a struggling server.")),
        431 => ("Request Header Fields Too Large", "client_error", "Sum of header sizes exceeds server limit.", None, Some(false), None),
        451 => ("Unavailable For Legal Reasons", "client_error", "Resource blocked by court order, takedown, or law (e.g. GDPR, DMCA, geo-block).", None, Some(false), None),
        // 5xx Server errors.
        500 => ("Internal Server Error", "server_error", "Unhandled exception in the server.", None, Some(false), None),
        501 => ("Not Implemented", "server_error", "Server doesn't recognize the request method.", None, Some(true), None),
        502 => ("Bad Gateway", "server_error", "Upstream returned an invalid response.", None, Some(false), None),
        503 => ("Service Unavailable", "server_error", "Server temporarily overloaded or under maintenance. Retry-After header optional.", None, Some(false), Some("Use Retry-After when you set this — clients otherwise pick exponential backoff defaults that may be too long.")),
        504 => ("Gateway Timeout", "server_error", "Upstream took too long to respond.", None, Some(false), None),
        505 => ("HTTP Version Not Supported", "server_error", "Server doesn't speak the requested HTTP version.", None, Some(false), None),
        506 => ("Variant Also Negotiates", "server_error", "Transparent content negotiation loop.", None, Some(false), None),
        507 => ("Insufficient Storage", "server_error", "Server out of disk / quota (WebDAV).", None, Some(false), None),
        508 => ("Loop Detected", "server_error", "Infinite loop in request processing (WebDAV).", None, Some(false), None),
        510 => ("Not Extended", "server_error", "Required extensions missing.", None, Some(false), None),
        511 => ("Network Authentication Required", "server_error", "Captive portal — client must authenticate to the network before reaching the server.", None, Some(false), None),
        _ => {
            let class = match code / 100 {
                1 => "informational",
                2 => "success",
                3 => "redirection",
                4 => "client_error",
                5 => "server_error",
                _ => "unknown",
            };
            ("Unassigned", class, "Code is in the 1xx-5xx range but not present in the IANA registry as of May 2024.", None, None, None)
        }
    }
}

// ---------------------------------------------------------------------------
// http_header_explain
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct HeaderArgs {
    /// HTTP header name (case-insensitive), e.g. `Cache-Control`, `ETag`, `Vary`.
    name: String,
}

pub struct HttpHeaderExplain;
impl Skill for HttpHeaderExplain {
    fn name(&self) -> &'static str {
        "http_header_explain"
    }
    fn description(&self) -> &'static str {
        "Explain a well-known HTTP header: purpose, request vs response context, syntax, and the \
         LLM-typical gotchas (e.g. `Vary` semantics, `Cache-Control: no-cache` vs `no-store`, \
         `Authorization` vs `Cookie` cache-keying). Backed by RFC 9110 / 9111. Local, no network."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<HeaderArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<HeaderArgs>()?;
            let key = args.name.trim().to_ascii_lowercase();
            match explain_header(&key) {
                Some(v) => Ok(text_result(v.to_string())),
                None => Ok(text_result(
                    json!({
                        "name": args.name,
                        "known": false,
                        "note": "Header is not in the curated registry. Consult RFC 9110 §10 or the IANA HTTP Field Name Registry.",
                    })
                    .to_string(),
                )),
            }
        })
    }
    fn examples(&self) -> &'static [SkillExample] {
        &[
            SkillExample {
                title: "Cache-Control",
                args: r#"{"name": "Cache-Control"}"#,
                note: Some("Lists every directive (no-cache vs no-store, must-revalidate, etc.) with semantics."),
            },
            SkillExample {
                title: "Vary",
                args: r#"{"name": "Vary"}"#,
                note: Some("Explains the cache-keying semantics LLMs typically get backwards."),
            },
            SkillExample {
                title: "Authorization",
                args: r#"{"name": "Authorization"}"#,
                note: None,
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Recall the exact semantics of a header before relying on it in code.",
            "Pick between two similar-sounding directives (no-cache vs no-store).",
            "Disambiguate request-vs-response context for headers used in both directions.",
        ]
    }
}

fn explain_header(name_lower: &str) -> Option<serde_json::Value> {
    let (canonical, context, purpose, syntax, gotcha): (&str, &str, &str, &str, Option<&str>) = match name_lower {
        "cache-control" => (
            "Cache-Control",
            "request + response",
            "Caching directives. The kitchen sink of HTTP cache semantics; see RFC 9111.",
            "Comma-separated directives: max-age=600, no-cache, no-store, private, public, must-revalidate, immutable, stale-while-revalidate=86400, etc.",
            Some("no-cache means 'revalidate before reuse' — it CAN still be cached. no-store means 'don't even write it to cache'. private means 'shared caches must not store this'."),
        ),
        "vary" => (
            "Vary",
            "response",
            "Tells caches that the response depends on the listed request headers; each combination is a separate cache key.",
            "Comma-separated header names: `Vary: Accept-Encoding, Accept-Language`. `Vary: *` means uncacheable.",
            Some("Without Vary, a cache may serve the gzipped response to a client that didn't ask for gzip. The header lists request headers that VARIED the response."),
        ),
        "etag" => (
            "ETag",
            "response",
            "Opaque validator for conditional requests. Strong (`\"abc\"`) or weak (`W/\"abc\"`).",
            "ETag: \"abc123\" — clients echo it via If-None-Match: \"abc123\".",
            Some("Weak ETags compare equal even for byte-different but semantically-equivalent responses; strong ETags require byte-identical content."),
        ),
        "last-modified" => (
            "Last-Modified",
            "response",
            "Origin server's idea of when the resource last changed. Paired with If-Modified-Since for conditional GETs.",
            "Last-Modified: Wed, 21 Oct 2026 07:28:00 GMT (HTTP-date).",
            Some("Resolution is 1 second; for sub-second changes use ETag."),
        ),
        "expires" => (
            "Expires",
            "response",
            "Legacy absolute expiration date. Cache-Control: max-age supersedes this when both are present.",
            "Expires: Wed, 21 Oct 2026 07:28:00 GMT.",
            Some("Expires: 0 or any invalid date = already expired (do not reuse from cache without revalidation)."),
        ),
        "age" => (
            "Age",
            "response",
            "Seconds since the response was generated upstream. Set by intermediate caches.",
            "Age: 300",
            None,
        ),
        "authorization" => (
            "Authorization",
            "request",
            "Carries the client's credentials per the scheme advertised in the previous 401 response's WWW-Authenticate header.",
            "Authorization: <scheme> <credentials> — e.g. `Basic dXNlcjpwYXNz`, `Bearer eyJ...`, `Digest ...`.",
            Some("Responses to authorized requests are private to that user by default — caches must not share them unless Cache-Control: public is set."),
        ),
        "www-authenticate" => (
            "WWW-Authenticate",
            "response (401)",
            "Server tells the client what authentication scheme(s) it accepts.",
            "WWW-Authenticate: Basic realm=\"app\", Bearer realm=\"app\".",
            None,
        ),
        "content-type" => (
            "Content-Type",
            "request + response",
            "Media type of the body. Includes charset, boundary, etc. for some types.",
            "Content-Type: text/html; charset=utf-8 — or application/json — or multipart/form-data; boundary=xyz.",
            Some("Browsers may sniff Content-Type and override what the server claims. Set X-Content-Type-Options: nosniff to suppress."),
        ),
        "content-length" => (
            "Content-Length",
            "request + response",
            "Body length in bytes. Mutually exclusive with Transfer-Encoding: chunked.",
            "Content-Length: 1234",
            None,
        ),
        "transfer-encoding" => (
            "Transfer-Encoding",
            "request + response",
            "Encoding applied to the body for transit (chunked, gzip, deflate). Hop-by-hop — strip when forwarding to upstream.",
            "Transfer-Encoding: chunked",
            Some("Mutually exclusive with Content-Length. HTTP/2 forbids Transfer-Encoding: chunked entirely (framing handles it)."),
        ),
        "content-encoding" => (
            "Content-Encoding",
            "request + response",
            "End-to-end encoding applied to the body (gzip, br, zstd). Survives proxies; client decodes.",
            "Content-Encoding: gzip",
            Some("Pair with Vary: Accept-Encoding on responses so caches don't serve compressed bodies to clients that didn't ask."),
        ),
        "accept" => (
            "Accept",
            "request",
            "Media types the client will accept. Server picks one and returns it (content negotiation).",
            "Accept: application/json, text/html; q=0.8, */*; q=0.5",
            None,
        ),
        "accept-encoding" => (
            "Accept-Encoding",
            "request",
            "Compression formats the client understands.",
            "Accept-Encoding: gzip, br, identity",
            None,
        ),
        "accept-language" => (
            "Accept-Language",
            "request",
            "Languages the client prefers (BCP 47 tags).",
            "Accept-Language: en-US, en;q=0.9, es;q=0.5",
            None,
        ),
        "cookie" => (
            "Cookie",
            "request",
            "Client echoes cookies set by the server's Set-Cookie. Each cookie's domain/path constraints apply.",
            "Cookie: name1=value1; name2=value2",
            Some("Like Authorization: responses to requests with Cookie are user-private by default — caches must not share unless Cache-Control: public."),
        ),
        "set-cookie" => (
            "Set-Cookie",
            "response",
            "Server sets a cookie on the client. RFC 6265bis.",
            "Set-Cookie: sid=abc; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age=3600",
            Some("SameSite=None requires Secure (HTTPS). HttpOnly blocks JS access; doesn't stop network sniffing."),
        ),
        "host" => (
            "Host",
            "request",
            "Target server's host:port. Mandatory in HTTP/1.1 (RFC 9110 §7.2).",
            "Host: example.com:8080",
            Some("HTTP/2 carries this in the :authority pseudo-header; the Host header is ignored if both are present."),
        ),
        "location" => (
            "Location",
            "response (3xx + 201)",
            "Target URL of a redirect (3xx) or the location of the newly-created resource (201).",
            "Location: https://example.com/path",
            None,
        ),
        "retry-after" => (
            "Retry-After",
            "response (429, 503, 3xx)",
            "How long the client should wait before retrying. Seconds OR HTTP-date.",
            "Retry-After: 120 — or — Retry-After: Wed, 21 Oct 2026 07:28:00 GMT",
            Some("ALWAYS check this on 429 / 503 — a fixed default backoff can hammer a struggling server. Reasonable cap: don't honor values longer than ~1 hour without operator review."),
        ),
        "x-forwarded-for" => (
            "X-Forwarded-For",
            "request",
            "Original client IP(s) appended by intermediate proxies. Comma-separated, left-to-right = earliest.",
            "X-Forwarded-For: 203.0.113.5, 10.0.0.1",
            Some("Trivially forgeable from the client side — only trust hops you control. Modern standard is Forwarded: (RFC 7239)."),
        ),
        "forwarded" => (
            "Forwarded",
            "request",
            "RFC 7239 replacement for X-Forwarded-*. Carries for, by, host, proto.",
            "Forwarded: for=203.0.113.5; proto=https; by=10.0.0.1",
            None,
        ),
        "strict-transport-security" => (
            "Strict-Transport-Security",
            "response",
            "HSTS: tells browsers to use HTTPS for this host for the next N seconds, and (optionally) preload + include subdomains.",
            "Strict-Transport-Security: max-age=63072000; includeSubDomains; preload",
            Some("Only honored over HTTPS. max-age=0 cancels HSTS. Preload submission is a one-way ratchet — recovery is slow."),
        ),
        "content-security-policy" => (
            "Content-Security-Policy",
            "response",
            "Allowlist for script/style/image/connect sources. Killer of XSS classes.",
            "Content-Security-Policy: default-src 'self'; script-src 'self' https://cdn.example.com",
            Some("'unsafe-inline' undoes a lot of the protection. Use nonces or hashes instead."),
        ),
        "referer" | "referrer" => (
            "Referer",
            "request",
            "URL of the page that led to this request (misspelled in the original spec; Referrer-Policy uses the correct spelling).",
            "Referer: https://example.com/page",
            Some("Send-only header — server can't trust it. Modern browsers honor Referrer-Policy to strip or shorten it."),
        ),
        "user-agent" => (
            "User-Agent",
            "request",
            "Client identifier string. Wildly inconsistent; don't gate features on it.",
            "User-Agent: Mozilla/5.0 (...) Chrome/...",
            None,
        ),
        _ => return None,
    };
    Some(json!({
        "name": canonical,
        "known": true,
        "context": context,
        "purpose": purpose,
        "syntax": syntax,
        "gotcha": gotcha,
    }))
}

// ---------------------------------------------------------------------------
// http_cache_decode
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct CacheDecodeArgs {
    /// `Cache-Control` header value, e.g. `"public, max-age=600, must-revalidate"`.
    #[serde(default)]
    cache_control: Option<String>,
    /// `Expires` header value (HTTP-date). Used only when `Cache-Control: max-age=` is absent.
    #[serde(default)]
    expires: Option<String>,
    /// `Pragma` header value (legacy; only `Pragma: no-cache` is honored).
    #[serde(default)]
    pragma: Option<String>,
    /// `Vary` header value.
    #[serde(default)]
    vary: Option<String>,
    /// `Age` header value (seconds since the response was generated upstream).
    #[serde(default)]
    age: Option<u64>,
}

pub struct HttpCacheDecode;
impl Skill for HttpCacheDecode {
    fn name(&self) -> &'static str {
        "http_cache_decode"
    }
    fn description(&self) -> &'static str {
        "Parse a set of cache-related response headers (Cache-Control, Expires, Pragma, Vary, \
         Age) into a structured verdict: storable? shared-cacheable? max-age? must-revalidate? \
         Which Vary axes apply? Backed by RFC 9111. Local, no network."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<CacheDecodeArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<CacheDecodeArgs>()?;
            let parsed = parse_cache_control(args.cache_control.as_deref().unwrap_or(""));
            let pragma_no_cache = args
                .pragma
                .as_deref()
                .map(|s| s.to_ascii_lowercase().contains("no-cache"))
                .unwrap_or(false);
            let vary_axes: Vec<String> = args
                .vary
                .as_deref()
                .map(|s| {
                    s.split(',')
                        .map(|t| t.trim().to_string())
                        .filter(|t| !t.is_empty())
                        .collect()
                })
                .unwrap_or_default();

            let storable = !parsed.no_store;
            let shared_cacheable = storable && !parsed.private;
            let must_revalidate_each_use = parsed.no_cache || pragma_no_cache;
            let effective_max_age = parsed.max_age.or_else(|| {
                // No Cache-Control max-age — fall back to Expires (best-effort summary; we don't parse the date).
                args.expires.as_deref().map(|s| {
                    if s.trim() == "0" || s.trim().is_empty() {
                        0
                    } else {
                        // Indicate non-zero, unknown duration via a sentinel.
                        u64::MAX
                    }
                })
            });

            let mut verdict_lines = Vec::new();
            if !storable {
                verdict_lines.push("no-store: caches MUST NOT write this response.");
            } else if must_revalidate_each_use {
                verdict_lines.push(
                    "no-cache: cache may store but MUST revalidate with origin on every reuse.",
                );
            } else if let Some(m) = parsed.max_age {
                verdict_lines.push(if m == 0 {
                    "max-age=0: fresh window is zero — equivalent to no-cache for reuse."
                } else {
                    "fresh for max-age seconds."
                });
            }
            if parsed.private {
                verdict_lines
                    .push("private: shared caches (CDN, proxy) must not store; browser cache OK.");
            } else if parsed.public {
                verdict_lines.push("public: explicitly cacheable by shared caches.");
            }
            if parsed.must_revalidate {
                verdict_lines.push(
                    "must-revalidate: stale responses MUST NOT be served without revalidation.",
                );
            }
            if parsed.immutable {
                verdict_lines.push(
                    "immutable: client should not revalidate even on user reload during freshness.",
                );
            }
            if let Some(swr) = parsed.stale_while_revalidate {
                verdict_lines
                    .push(Box::leak(format!("stale-while-revalidate={swr}: stale response may be served while a revalidation runs in the background.").into_boxed_str()));
            }
            if !vary_axes.is_empty() {
                verdict_lines.push("Vary listed — each axis varies the cache key (e.g. Accept-Encoding splits gzipped vs identity).");
            }

            Ok(text_result(
                json!({
                    "input": {
                        "cache_control": args.cache_control,
                        "expires": args.expires,
                        "pragma": args.pragma,
                        "vary": args.vary,
                        "age_seconds": args.age,
                    },
                    "storable": storable,
                    "shared_cacheable": shared_cacheable,
                    "must_revalidate_each_use": must_revalidate_each_use,
                    "max_age_seconds": effective_max_age,
                    "vary_axes": vary_axes,
                    "directives": {
                        "no_store": parsed.no_store,
                        "no_cache": parsed.no_cache,
                        "public": parsed.public,
                        "private": parsed.private,
                        "must_revalidate": parsed.must_revalidate,
                        "proxy_revalidate": parsed.proxy_revalidate,
                        "immutable": parsed.immutable,
                        "stale_while_revalidate_seconds": parsed.stale_while_revalidate,
                        "stale_if_error_seconds": parsed.stale_if_error,
                        "s_maxage_seconds": parsed.s_maxage,
                    },
                    "verdict": verdict_lines,
                })
                .to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [SkillExample] {
        &[
            SkillExample {
                title: "CDN-friendly long-lived asset",
                args: r#"{"cache_control": "public, max-age=31536000, immutable"}"#,
                note: Some(
                    "Storable + shared-cacheable, no per-use revalidation, 1-year freshness.",
                ),
            },
            SkillExample {
                title: "Private user dashboard",
                args: r#"{"cache_control": "private, no-cache, max-age=0"}"#,
                note: Some("Storable in browser only, MUST revalidate on every reuse."),
            },
            SkillExample {
                title: "Cache-busting (no-store)",
                args: r#"{"cache_control": "no-store"}"#,
                note: Some("Caches must not store at all — strictest setting."),
            },
            SkillExample {
                title: "With Vary",
                args: r#"{"cache_control": "public, max-age=600", "vary": "Accept-Encoding, Accept-Language"}"#,
                note: Some("Two-axis cache key — distinct entries per encoding × language."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Decode whether a response will actually be cached by a CDN / browser / shared proxy.",
            "Disambiguate no-cache (revalidate) from no-store (don't even write) before deploying a config.",
            "Check that your Vary header captures every axis your response varies on.",
        ]
    }
}

#[derive(Default, Debug)]
struct ParsedCacheControl {
    no_store: bool,
    no_cache: bool,
    public: bool,
    private: bool,
    must_revalidate: bool,
    proxy_revalidate: bool,
    immutable: bool,
    max_age: Option<u64>,
    s_maxage: Option<u64>,
    stale_while_revalidate: Option<u64>,
    stale_if_error: Option<u64>,
}

fn parse_cache_control(s: &str) -> ParsedCacheControl {
    let mut p = ParsedCacheControl::default();
    for raw_token in s.split(',') {
        let token = raw_token.trim().to_ascii_lowercase();
        if token.is_empty() {
            continue;
        }
        if let Some((name, val)) = token.split_once('=') {
            let val = val.trim().trim_matches('"');
            match name.trim() {
                "max-age" => p.max_age = val.parse().ok(),
                "s-maxage" => p.s_maxage = val.parse().ok(),
                "stale-while-revalidate" => p.stale_while_revalidate = val.parse().ok(),
                "stale-if-error" => p.stale_if_error = val.parse().ok(),
                _ => {}
            }
        } else {
            match token.as_str() {
                "no-store" => p.no_store = true,
                "no-cache" => p.no_cache = true,
                "public" => p.public = true,
                "private" => p.private = true,
                "must-revalidate" => p.must_revalidate = true,
                "proxy-revalidate" => p.proxy_revalidate = true,
                "immutable" => p.immutable = true,
                _ => {}
            }
        }
    }
    p
}

// ---------------------------------------------------------------------------
// Family
// ---------------------------------------------------------------------------

pub struct Family;
impl crate::skills::FamilyMeta for Family {
    fn family(&self) -> &'static str {
        "http"
    }
    fn tools(&self) -> Vec<&'static str> {
        skills().iter().map(|s| s.name()).collect()
    }
    fn description(&self) -> &'static str {
        "HTTP status code decoder, well-known header explainer, and cache-policy verdict from \
         a header set. Pure local compute, no external deps. Backed by RFC 9110 / 9111 — \
         deterministic answers for tasks LLMs commonly mix up (301/302/303/307/308, no-cache \
         vs no-store, Vary semantics)."
    }
    fn check_capability(&self) -> crate::skills::SkillCapability {
        crate::skills::SkillCapability::Ready
    }
    fn example_flow(&self) -> Option<&'static str> {
        Some(
            "1. `http_status_decode { code: 308 }` — am I picking the right redirect?\n\
             2. `http_header_explain { name: \"Cache-Control\" }` — refresh on directive semantics.\n\
             3. `http_cache_decode { cache_control: \"public, max-age=600\", vary: \"Accept-Encoding\" }` — what will a CDN actually do with this?",
        )
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(HttpStatusDecode),
        Box::new(HttpHeaderExplain),
        Box::new(HttpCacheDecode),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_308_distinguishes_from_301() {
        let v = decode_status(308);
        assert_eq!(v["name"], "Permanent Redirect");
        assert!(v["summary"].as_str().unwrap().contains("preserve"));
        assert!(v["gotcha"].as_str().unwrap().contains("POST"));
    }

    #[test]
    fn decode_303_says_method_is_rewritten() {
        let v = decode_status(303);
        assert!(v["gotcha"]
            .as_str()
            .unwrap()
            .to_ascii_lowercase()
            .contains("get"));
    }

    #[test]
    fn header_cache_control_known() {
        let v = explain_header("cache-control").unwrap();
        assert_eq!(v["known"], true);
        let gotcha = v["gotcha"].as_str().unwrap();
        assert!(gotcha.contains("no-cache"));
        assert!(gotcha.contains("no-store"));
    }

    #[test]
    fn header_unknown_returns_none() {
        assert!(explain_header("x-custom-banana").is_none());
    }

    #[test]
    fn parse_cc_handles_kitchen_sink() {
        let p = parse_cache_control("public, max-age=3600, s-maxage=600, must-revalidate, immutable, stale-while-revalidate=120");
        assert!(p.public);
        assert!(p.must_revalidate);
        assert!(p.immutable);
        assert_eq!(p.max_age, Some(3600));
        assert_eq!(p.s_maxage, Some(600));
        assert_eq!(p.stale_while_revalidate, Some(120));
    }

    #[test]
    fn no_store_blocks_storage() {
        let p = parse_cache_control("no-store");
        assert!(p.no_store);
        assert!(!p.no_cache);
    }

    #[test]
    fn private_vs_public() {
        let p = parse_cache_control("private, max-age=60");
        assert!(p.private);
        assert!(!p.public);
    }
}

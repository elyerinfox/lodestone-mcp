//! Finance & accounting skills: `compound_interest` and `loan_payment` are local
//! (no network); `currency_convert` uses the **keyless** Frankfurter API (European
//! Central Bank reference rates) — no API key — and caches the rate.

use std::sync::Arc;

use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::Value;

use crate::skills::{schema_for, Skill, SkillCtx};
use crate::{internal, invalid, text_result};

/// Round to cents for display.
fn money(x: f64) -> String {
    format!("{:.2}", (x * 100.0).round() / 100.0)
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct CompoundArgs {
    /// Initial principal amount.
    principal: f64,
    /// Annual interest rate, as a percent (e.g. 5 for 5%).
    annual_rate_percent: f64,
    /// Number of years.
    years: f64,
    /// Compounding periods per year (e.g. 12 monthly, 4 quarterly, 1 annual).
    /// Defaults to 1.
    #[serde(default)]
    compounds_per_year: Option<f64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct LoanArgs {
    /// Loan principal (amount borrowed).
    principal: f64,
    /// Annual interest rate, as a percent (APR).
    annual_rate_percent: f64,
    /// Term in months.
    months: u32,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct CurrencyArgs {
    /// Amount in the source currency.
    amount: f64,
    /// Source currency code (ISO 4217, e.g. "USD").
    from: String,
    /// Target currency code (ISO 4217, e.g. "EUR").
    to: String,
}

pub struct CompoundInterest;
impl Skill for CompoundInterest {
    fn name(&self) -> &'static str {
        "compound_interest"
    }
    fn description(&self) -> &'static str {
        "Compute compound-interest future value (local, no network): principal grows at \
        annual_rate_percent over `years`, compounded compounds_per_year times (default 1). Returns \
        the future value and the interest earned."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<CompoundArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, a) = ctx.parse::<CompoundArgs>()?;
            let n = a.compounds_per_year.unwrap_or(1.0);
            if n <= 0.0 || a.years < 0.0 {
                return Err(invalid("compounds_per_year must be > 0 and years >= 0"));
            }
            let r = a.annual_rate_percent / 100.0;
            let fv = a.principal * (1.0 + r / n).powf(n * a.years);
            let interest = fv - a.principal;
            Ok(text_result(format!(
                "Principal {} at {}%/yr, {} yr(s), compounded {}×/yr:\n  future value: {}\n  interest earned: {}",
                money(a.principal),
                a.annual_rate_percent,
                a.years,
                n,
                money(fv),
                money(interest),
            )))
        })
    }
}

pub struct LoanPayment;
impl Skill for LoanPayment {
    fn name(&self) -> &'static str {
        "loan_payment"
    }
    fn description(&self) -> &'static str {
        "Compute the level monthly payment of an amortized loan (local, no network) from principal, \
        annual APR percent, and term in months. Returns the monthly payment, total paid, and total \
        interest."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<LoanArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, a) = ctx.parse::<LoanArgs>()?;
            if a.months == 0 {
                return Err(invalid("months must be >= 1"));
            }
            let i = a.annual_rate_percent / 100.0 / 12.0;
            let n = a.months as f64;
            let payment = if i.abs() < 1e-12 {
                a.principal / n
            } else {
                a.principal * i / (1.0 - (1.0 + i).powf(-n))
            };
            let total = payment * n;
            let interest = total - a.principal;
            Ok(text_result(format!(
                "Loan {} at {}% APR over {} months:\n  monthly payment: {}\n  total paid: {}\n  total interest: {}",
                money(a.principal),
                a.annual_rate_percent,
                a.months,
                money(payment),
                money(total),
                money(interest),
            )))
        })
    }
}

pub struct CurrencyConvert;
impl Skill for CurrencyConvert {
    fn name(&self) -> &'static str {
        "currency_convert"
    }
    fn description(&self) -> &'static str {
        "Convert an amount between currencies using keyless European Central Bank reference rates \
        (Frankfurter API; no API key). Returns the converted amount, the rate, and the rate date. \
        Rates are cached; they're reference rates, not live trading prices."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<CurrencyArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<CurrencyArgs>()?;
            let from = args.from.trim().to_ascii_uppercase();
            let to = args.to.trim().to_ascii_uppercase();
            let code_ok = |c: &str| c.len() == 3 && c.bytes().all(|b| b.is_ascii_alphabetic());
            if !code_ok(&from) || !code_ok(&to) {
                return Err(invalid(
                    "currency codes must be 3 letters (ISO 4217, e.g. USD)",
                ));
            }
            if from == to {
                return Ok(text_result(format!(
                    "{} {from} = {} {to} (same currency, rate 1.0)",
                    money(args.amount),
                    money(args.amount)
                )));
            }

            // Cache the per-1-unit rate (+ its date) so repeated conversions don't
            // re-hit the API. Value stored as "rate|date".
            let key = format!("fx|{from}|{to}");
            let (rate, date) = match server.retrieval_get(&key).await {
                Some(v) => {
                    let mut it = v.splitn(2, '|');
                    let rate: f64 = it.next().and_then(|s| s.parse().ok()).unwrap_or(0.0);
                    let date = it.next().unwrap_or("").to_string();
                    (rate, date)
                }
                None => {
                    let url = format!("https://api.frankfurter.app/latest?from={from}&to={to}");
                    let v: Value = server
                        .http
                        .get(&url)
                        .send()
                        .await
                        .map_err(|e| internal(e.into()))?
                        .error_for_status()
                        .map_err(|_| {
                            invalid(format!("unknown or unsupported currency pair {from}/{to}"))
                        })?
                        .json()
                        .await
                        .map_err(|e| internal(e.into()))?;
                    let rate = v
                        .pointer(&format!("/rates/{to}"))
                        .and_then(|x| x.as_f64())
                        .ok_or_else(|| invalid(format!("no rate returned for {from}/{to}")))?;
                    let date = v
                        .get("date")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string();
                    server.retrieval_put(key, &format!("{rate}|{date}"));
                    (rate, date)
                }
            };

            let converted = args.amount * rate;
            Ok(text_result(format!(
                "{} {from} = {} {to}\n  rate: 1 {from} = {} {to}{}",
                money(args.amount),
                money(converted),
                rate,
                if date.is_empty() {
                    String::new()
                } else {
                    format!(" (ECB ref, {date})")
                },
            )))
        })
    }
}

/// The skills this module contributes.
#[cfg(test)]
mod live {
    fn http() -> reqwest::Client {
        reqwest::Client::builder()
            .user_agent("lodestone-mcp/0.1.0 (+https://github.com/elyerinfox/lodestone-mcp)")
            .build()
            .unwrap()
    }

    /// ECB publishes a daily XML reference-rates file — the source the
    /// currency_convert skill reads. Stable URL, small payload.
    #[tokio::test]
    #[ignore]
    async fn ecb_reference_rates_live() {
        let r = http()
            .get("https://www.ecb.europa.eu/stats/eurofxref/eurofxref-daily.xml")
            .send().await.expect("network").error_for_status().unwrap();
        let body = r.text().await.unwrap();
        assert!(body.contains("<gesmes:Envelope") || body.contains("Envelope"));
        // Major-currency entries we expect.
        for ccy in ["USD", "JPY", "GBP", "CHF"] {
            assert!(body.contains(&format!("'{ccy}'")), "missing currency {ccy}");
        }
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(CompoundInterest),
        Box::new(LoanPayment),
        Box::new(CurrencyConvert),
    ]
}

#[cfg(test)]
mod tests {
    #[test]
    fn money_rounds_to_cents() {
        assert_eq!(super::money(1234.5678), "1234.57");
        assert_eq!(super::money(0.1 + 0.2), "0.30");
    }
}

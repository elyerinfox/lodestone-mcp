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
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Annual compounding",
                args: r#"{"principal": 10000, "annual_rate_percent": 5, "years": 10}"#,
                note: Some("Defaults to compounds_per_year=1."),
            },
            SkillExample {
                title: "Monthly compounding savings",
                args: r#"{"principal": 5000, "annual_rate_percent": 4.5, "years": 5, "compounds_per_year": 12}"#,
                note: None,
            },
            SkillExample {
                title: "Daily-compounded short term",
                args: r#"{"principal": 1000, "annual_rate_percent": 6, "years": 0.5, "compounds_per_year": 365}"#,
                note: None,
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Project future value of a savings/investment principal.",
            "Compare scenarios at different compounding frequencies.",
            "Compute interest earned over a time horizon.",
        ]
    }
    fn validation_rules(&self) -> &'static [crate::skills::validation::Rule] {
        use crate::skills::validation::Rule;
        &[Rule::Range {
            field: "years",
            min: Some(0.0),
            max: None,
        }]
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
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "30-year fixed mortgage",
                args: r#"{"principal": 400000, "annual_rate_percent": 6.5, "months": 360}"#,
                note: Some("Returns monthly payment, total paid, total interest."),
            },
            SkillExample {
                title: "5-year auto loan",
                args: r#"{"principal": 28000, "annual_rate_percent": 7.25, "months": 60}"#,
                note: None,
            },
            SkillExample {
                title: "Zero-interest financing",
                args: r#"{"principal": 1200, "annual_rate_percent": 0, "months": 12}"#,
                note: Some("Handles APR=0 cleanly (payment = principal / months)."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Compute the amortized monthly payment of a loan or mortgage.",
            "See total interest paid over the life of a loan.",
            "Compare loan terms (rate, principal, months).",
        ]
    }
    fn validation_rules(&self) -> &'static [crate::skills::validation::Rule] {
        use crate::skills::validation::Rule;
        &[Rule::Range {
            field: "months",
            min: Some(1.0),
            max: None,
        }]
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
    fn retrieval_policy(&self) -> crate::skills::RetrievalPolicy {
        crate::skills::RetrievalPolicy::Shared {
            source: crate::constellation::Source::Other,
        }
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
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "USD to EUR",
                args: r#"{"amount": 100, "from": "USD", "to": "EUR"}"#,
                note: Some("Uses ECB reference rates (not live trading prices)."),
            },
            SkillExample {
                title: "Yen to pounds",
                args: r#"{"amount": 50000, "from": "JPY", "to": "GBP"}"#,
                note: None,
            },
            SkillExample {
                title: "Same currency short-circuit",
                args: r#"{"amount": 42, "from": "USD", "to": "USD"}"#,
                note: Some("No network call; rate is 1.0."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Convert an amount between two ISO 4217 currencies at the ECB reference rate.",
            "Get the current FX reference rate for a pair.",
            "Estimate the value of a foreign-currency price in a familiar one.",
        ]
    }
    fn validation_rules(&self) -> &'static [crate::skills::validation::Rule] {
        use crate::skills::validation::Rule;
        &[
            Rule::Regex {
                field: "from",
                pattern: r"^[A-Za-z]{3}$",
                summary: "ISO 4217 alpha-3 code",
            },
            Rule::Regex {
                field: "to",
                pattern: r"^[A-Za-z]{3}$",
                summary: "ISO 4217 alpha-3 code",
            },
        ]
    }
}

/// The skills this module contributes.
#[cfg(test)]
mod live {
    fn http() -> reqwest::Client {
        crate::skills::live_http()
    }

    /// ECB publishes a daily XML reference-rates file — the source the
    /// currency_convert skill reads. Stable URL, small payload.
    #[tokio::test]
    #[ignore]
    async fn ecb_reference_rates_live() {
        let r = http()
            .get("https://www.ecb.europa.eu/stats/eurofxref/eurofxref-daily.xml")
            .send()
            .await
            .expect("network")
            .error_for_status()
            .unwrap();
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

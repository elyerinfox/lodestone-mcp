# Finance — `compound_interest` / `loan_payment` / `currency_convert`

|  |  |
| --- | --- |
| **Module** | [`src/skills/finance.rs`](../../src/skills/finance.rs) |
| **Tools** | `compound_interest`, `loan_payment`, `currency_convert` |
| **Network** | local-only, except `currency_convert` (keyless API) |
| **Default** | on |
| **Config** | none — `currency_convert` hits the keyless Frankfurter/ECB endpoint; rates are cached |

## What it does
Financial calculations. `compound_interest` and `loan_payment` are pure local math (no
network). `currency_convert` fetches a rate from the keyless Frankfurter API (European
Central Bank reference rates — no API key) and caches it. Money values are rounded to
cents for display.

## Tools
| Tool | Arguments | Purpose |
| --- | --- | --- |
| `compound_interest` | `principal`, `annual_rate_percent`, `years`, `compounds_per_year?` | Future value of `principal` growing at the annual rate over `years`, compounded `compounds_per_year` times (default 1). Returns future value + interest earned. |
| `loan_payment` | `principal`, `annual_rate_percent`, `months` | Level monthly payment of an amortized loan (APR + term in months). Returns monthly payment, total paid, total interest. |
| `currency_convert` | `amount`, `from`, `to` | Convert between ISO-4217 currencies via keyless ECB reference rates (Frankfurter). Returns the converted amount, the rate, and the rate date. |

## Configuration & gating
No configuration. `compound_interest` and `loan_payment` are fully local.
`currency_convert` calls the keyless Frankfurter endpoint and caches the per-unit rate
(plus its date) keyed on the pair, so repeated conversions don't re-hit the API; same-currency
conversions short-circuit at rate 1.0. Rates are ECB *reference* rates, not live trading
prices. Each tool is independently gateable via `[tools]`.

## Example uses
- **Convert then budget** — `currency_convert` a price to your currency, then `loan_payment` to amortize it.
- **Compare growth** — `compound_interest` across different `compounds_per_year` to see the effect of compounding.
- **Loan cost** — `loan_payment` to get the monthly payment and total interest on a mortgage.

## See also
[tools.md](../tools.md)

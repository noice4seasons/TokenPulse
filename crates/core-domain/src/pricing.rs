use crate::models::{PricingPrecision, TokenVector, UnpricedReason, UsageCostEstimate, UsageEntry};

const PRICE_CUT: i64 = 1785369600000;
const ASTRA_START: i64 = 1788393600000;
const SONNET_START: i64 = 1788220800000;

#[derive(Debug)]
pub struct PriceRule {
    pub rule_id: &'static str,
    pub model: &'static str,
    pub effective_from_ms: i64,
    pub effective_until_ms: Option<i64>,
    pub long_context_tiered: bool,
    // Thousandths of a micro-dollar per token: input, output, write, read.
    rates: [u32; 4],
}
const fn rule(
    rule_id: &'static str,
    model: &'static str,
    effective_from_ms: i64,
    effective_until_ms: Option<i64>,
    long_context_tiered: bool,
    rates: [u32; 4],
) -> PriceRule {
    PriceRule {
        rule_id,
        model,
        effective_from_ms,
        effective_until_ms,
        long_context_tiered,
        rates,
    }
}
#[rustfmt::skip]
pub static RULES:&[PriceRule]=&[
    rule("gpt-6-astra-standard-2026-09-03-v1", "gpt-6-astra", ASTRA_START, None, true, [10000, 50000, 12500, 1000]),
    rule("gpt-5.6-sol-v1", "gpt-5.6-sol", i64::MIN, None, true, [5000, 30000, 6250, 500]),
    rule("gpt-5.6-terra-launch-v1", "gpt-5.6-terra", i64::MIN, Some(PRICE_CUT), true, [2500, 15000, 3125, 250]),
    rule("gpt-5.6-terra-2026-07-30-v1", "gpt-5.6-terra", PRICE_CUT, None, true, [2000, 12000, 2500, 200]),
    rule("gpt-5.6-luna-launch-v1", "gpt-5.6-luna", i64::MIN, Some(PRICE_CUT), true, [1000, 6000, 1250, 100]),
    rule("gpt-5.6-luna-2026-07-30-v1", "gpt-5.6-luna", PRICE_CUT, None, true, [200, 1200, 250, 20]),
    rule("gpt-5.4-mini-v1", "gpt-5.4-mini", i64::MIN, None, false, [750, 4500, 750, 75]),
    rule("gpt-5.4-nano-v1", "gpt-5.4-nano", i64::MIN, None, false, [200, 1250, 200, 20]),
    rule("gpt-5.4-v1", "gpt-5.4", i64::MIN, None, false, [2500, 15000, 2500, 250]),
    rule("gpt-5.3-codex-v1", "gpt-5.3-codex", i64::MIN, None, false, [1750, 14000, 1750, 175]),
    rule("gpt-5.2-codex-v1", "gpt-5.2-codex", i64::MIN, None, false, [1750, 14000, 1750, 175]),
    rule("gpt-5.1-codex-mini-v1", "gpt-5.1-codex-mini", i64::MIN, None, false, [250, 2000, 250, 25]),
    rule("gpt-5.1-codex-v1", "gpt-5.1-codex", i64::MIN, None, false, [1250, 10000, 1250, 125]),
    rule("gpt-5-codex-v1", "gpt-5-codex", i64::MIN, None, false, [1250, 10000, 1250, 125]),
    rule("codex-mini-latest-v1", "codex-mini-latest", i64::MIN, None, false, [1500, 6000, 1500, 375]),
    rule("gpt-4o-v1", "gpt-4o", i64::MIN, None, false, [2500, 10000, 0, 1250]),
    rule("gpt-4-v1", "gpt-4", i64::MIN, None, false, [2000, 8000, 0, 500]),
    rule("claude-fable-5-v1", "claude-fable-5", i64::MIN, None, false, [10000, 50000, 12500, 1000]),
    rule("claude-opus-5-v1", "claude-opus-5", i64::MIN, None, false, [5000, 25000, 6250, 500]),
    rule("claude-opus-4-v1", "claude-opus-4", i64::MIN, None, false, [15000, 75000, 18750, 1500]),
    rule("claude-sonnet-4-v1", "claude-sonnet-4", i64::MIN, None, false, [3000, 15000, 3750, 300]),
    rule("claude-haiku-4-v1", "claude-haiku-4", i64::MIN, None, false, [1000, 5000, 1250, 100]),
    rule("claude-haiku-v1", "claude-haiku", i64::MIN, None, false, [800, 4000, 1000, 80]),
    rule("claude-sonnet-5-promo-v1", "claude-sonnet-5", i64::MIN, Some(SONNET_START), false, [2000, 10000, 2500, 200]),
    rule("claude-sonnet-5-standard-v1", "claude-sonnet-5", SONNET_START, None, false, [3000, 15000, 3750, 300]),
];

/// Accept exact names and explicitly recognized dated snapshots.
fn base_name(model: &str) -> &str {
    if let Some((base, suffix)) = model.rsplit_once('-') {
        if suffix.len() == 8 && suffix.bytes().all(|c| c.is_ascii_digit()) {
            return base;
        }
    }
    let bytes = model.as_bytes();
    if let Some(n) = bytes.len().checked_sub(11) {
        if bytes[n..].iter().enumerate().all(|(i, c)| {
            if matches!(i, 0 | 5 | 8) {
                *c == b'-'
            } else {
                c.is_ascii_digit()
            }
        }) {
            return &model[..n];
        }
    }
    model
}
pub fn resolve_rule(raw: &str, timestamp_ms: i64) -> Option<&'static PriceRule> {
    let lower = raw.trim().to_ascii_lowercase();
    let model = match base_name(&lower) {
        "gpt-5.6" => "gpt-5.6-sol",
        "gpt-5.1-codex-max" => "gpt-5.1-codex",
        "claude-opus-4-5" | "claude-opus-4-6" | "claude-opus-4-7" | "claude-opus-4-8" => {
            "claude-opus-5"
        }
        "claude-sonnet" => "claude-sonnet-4",
        other => other,
    };
    RULES.iter().find(|r| {
        r.model == model
            && timestamp_ms >= r.effective_from_ms
            && r.effective_until_ms.map_or(true, |end| timestamp_ms < end)
    })
}
fn unpriced(reason: UnpricedReason) -> PricingPrecision {
    PricingPrecision::Unpriced { reason }
}

pub fn estimate_usage_cost(
    model: &str,
    timestamp_ms: i64,
    t: &TokenVector,
    single_request_evidence: bool,
) -> Result<UsageCostEstimate, PricingPrecision> {
    let rule =
        resolve_rule(model, timestamp_ms).ok_or_else(|| unpriced(UnpricedReason::UnknownModel))?;
    let invalid = || unpriced(UnpricedReason::InvalidTokenCount);
    if t.reasoning_output_tokens > t.output_tokens
        || u128::from(t.input_tokens)
            != u128::from(t.non_cached_input_tokens) + u128::from(t.cache_read_tokens)
    {
        return Err(invalid());
    }
    let long = rule.long_context_tiered && t.prompt_tokens() > 272_000;
    if long && !single_request_evidence {
        return Err(unpriced(
            UnpricedReason::MissingSingleRequestEvidenceForLongContext,
        ));
    }
    let [input, output, write, read] = rule.rates.map(u128::from);
    // Keep half-rate output multipliers exact; round once per final amount.
    let input_factor = if long { 4 } else { 2 };
    let output_factor = if long { 3 } else { 2 };
    let prompt = input * u128::from(t.non_cached_input_tokens)
        + write * u128::from(t.cache_creation_tokens)
        + read * u128::from(t.cache_read_tokens);
    let cost = prompt * input_factor + output * u128::from(t.output_tokens) * output_factor;
    let savings = input.saturating_sub(read) * u128::from(t.cache_read_tokens) * input_factor;
    let round = |n: u128| i64::try_from((n + 1000) / 2000).map_err(|_| invalid());
    Ok(UsageCostEstimate {
        cost_micro_usd: round(cost)?,
        cache_savings_micro_usd: round(savings)?,
        precision: PricingPrecision::Exact {
            rule_id: rule.rule_id.to_owned(),
        },
    })
}
impl UsageEntry {
    pub fn reprice(&mut self) {
        let result = estimate_usage_cost(
            &self.raw_model,
            self.timestamp_ms,
            &self.tokens,
            self.single_request_evidence,
        );
        self.cost_micro_usd = result.as_ref().ok().map(|v| v.cost_micro_usd);
        self.cache_savings_micro_usd = result.as_ref().ok().map(|v| v.cache_savings_micro_usd);
        self.pricing_precision = result.map_or_else(|p| p, |v| v.precision);
        self.price_rule_id = match &self.pricing_precision {
            PricingPrecision::Exact { rule_id } => Some(rule_id.clone()),
            PricingPrecision::Unpriced { .. } => None,
        };
    }
}

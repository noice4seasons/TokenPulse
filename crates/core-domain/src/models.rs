use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderId {
    Codex,
    Antigravity,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenVector {
    /// Provider's inclusive input counter. Never add it to the disjoint counters.
    pub input_tokens: u64,
    pub non_cached_input_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
    pub output_tokens: u64,
    /// Already included in output_tokens.
    pub reasoning_output_tokens: u64,
}

impl TokenVector {
    pub fn prompt_tokens(&self) -> u128 {
        u128::from(self.non_cached_input_tokens)
            + u128::from(self.cache_read_tokens)
            + u128::from(self.cache_creation_tokens)
    }
    pub fn total_tokens(&self) -> u128 {
        self.prompt_tokens() + u128::from(self.output_tokens)
    }
    pub fn cache_hit_ratio(&self) -> Option<f64> {
        let input = u128::from(self.non_cached_input_tokens) + u128::from(self.cache_read_tokens);
        (input != 0).then(|| self.cache_read_tokens as f64 / input as f64)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum UnpricedReason {
    UnknownModel,
    MissingSingleRequestEvidenceForLongContext,
    InvalidTokenCount,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PricingPrecision {
    Exact { rule_id: String },
    Unpriced { reason: UnpricedReason },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageCostEstimate {
    pub cost_micro_usd: i64,
    pub cache_savings_micro_usd: i64,
    pub precision: PricingPrecision,
}
impl UsageCostEstimate {
    pub fn cost_usd(&self) -> f64 {
        self.cost_micro_usd as f64 / 1_000_000.0
    }
    pub fn cache_savings_usd(&self) -> f64 {
        self.cache_savings_micro_usd as f64 / 1_000_000.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageEntry {
    pub raw_model: String,
    pub display_model: String,
    pub request_id: Option<String>,
    pub identity_key: String,
    pub identity_origin: String,
    pub identity_aliases: Vec<String>,
    pub timestamp_ms: i64,
    pub provider: ProviderId,
    #[serde(flatten)]
    pub tokens: TokenVector,
    pub cost_micro_usd: Option<i64>,
    pub cache_savings_micro_usd: Option<i64>,
    pub price_rule_id: Option<String>,
    pub pricing_precision: PricingPrecision,
    pub single_request_evidence: bool,
}

pub fn normalize_codex_model(raw: &str) -> String {
    let model = raw.trim();
    let lower = model.to_ascii_lowercase();
    if model.is_empty() {
        return "Unknown (codex)".into();
    }
    if lower.contains("gpt-5") {
        return model.to_uppercase();
    }
    for (needle, label) in [
        ("opus", "Opus"),
        ("sonnet", "Sonnet"),
        ("haiku", "Haiku"),
        ("gpt-4o", "GPT-4o"),
        ("gpt-4", "GPT-4"),
        ("glm", "GLM"),
    ] {
        if lower.contains(needle) {
            return label.into();
        }
    }
    raw.to_owned()
}

/// Keep checked addition and subtraction in lockstep with the persisted counters.
macro_rules! metrics {
    ($($field:ident: $ty:ty),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
        pub struct UsageMetrics { $(pub $field: $ty),+ }
        impl UsageMetrics {
            pub fn checked_add(&self, other: &Self) -> Option<Self> {
                Some(Self { $($field: self.$field.checked_add(other.$field)?),+ })
            }
            pub fn checked_sub(&self, other: &Self) -> Option<Self> {
                if false $(|| self.$field < other.$field)+ { return None }
                Some(Self { $($field: self.$field.checked_sub(other.$field)?),+ })
            }
        }
    }
}
metrics! {
    request_count:u64, input_tokens:u64, non_cached_input_tokens:u64, cache_read_tokens:u64,
    cache_creation_tokens:u64, output_tokens:u64, reasoning_output_tokens:u64, total_tokens:u64,
    priced_request_count:u64, unpriced_request_count:u64, priced_tokens:u64,
    cost_micro_usd:i64, cache_savings_micro_usd:i64,
}
impl UsageMetrics {
    pub fn priced_token_coverage_ratio(&self) -> Option<f64> {
        (self.total_tokens != 0).then(|| self.priced_tokens as f64 / self.total_tokens as f64)
    }
    pub fn from_entry(entry: &UsageEntry) -> Option<Self> {
        let t = entry.tokens;
        let total_tokens = u64::try_from(t.total_tokens()).ok()?;
        let (priced, cost, savings) = match (
            &entry.pricing_precision,
            &entry.price_rule_id,
            entry.cost_micro_usd,
            entry.cache_savings_micro_usd,
        ) {
            (PricingPrecision::Exact { rule_id }, Some(id), Some(c), Some(s))
                if id == rule_id && c >= 0 && s >= 0 =>
            {
                (true, c, s)
            }
            (PricingPrecision::Unpriced { .. }, None, None, None) => (false, 0, 0),
            _ => return None,
        };
        Some(Self {
            request_count: 1,
            input_tokens: t.input_tokens,
            non_cached_input_tokens: t.non_cached_input_tokens,
            cache_read_tokens: t.cache_read_tokens,
            cache_creation_tokens: t.cache_creation_tokens,
            output_tokens: t.output_tokens,
            reasoning_output_tokens: t.reasoning_output_tokens,
            total_tokens,
            priced_request_count: priced.into(),
            unpriced_request_count: (!priced).into(),
            priced_tokens: if priced { total_tokens } else { 0 },
            cost_micro_usd: cost,
            cache_savings_micro_usd: savings,
        })
    }
}

use crate::models::{ProviderId, UsageMetrics};
use crate::quota::{CreditState, ProviderAvailability, QuotaPeriodKind, QuotaWindowState};
use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 1;
const DAY_MS: i64 = 86_400_000;
const MAX_PAGE_SIZE: u32 = 200;

// Reject unsupported versions on both ingress and egress.
mod version {
    use super::PROTOCOL_VERSION;
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(v: &u32, s: S) -> Result<S::Ok, S::Error> {
        if *v != PROTOCOL_VERSION {
            return Err(serde::ser::Error::custom("unsupported protocol version"));
        }
        s.serialize_u32(*v)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
        let v = u32::deserialize(d)?;
        if v != PROTOCOL_VERSION {
            return Err(serde::de::Error::custom("unsupported protocol version"));
        }
        Ok(v)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Coverage {
    Complete,
    Partial,
    Unknown,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AggregateDto {
    pub from_ms: i64,
    pub to_ms: i64,
    pub coverage: Coverage,
    pub metrics: UsageMetrics,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PrivacyBadge {
    LocalOnly,
    VerifiedLoopbackEnabled,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuotaBarDto {
    /// Display label supplied by the local adapter for the selected instance.
    pub target_label: String,
    pub period_kind: QuotaPeriodKind,
    pub state: QuotaWindowState,
    pub observed_at_ms: Option<i64>,
    pub account_confirmed: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverlaySnapshotDto {
    #[serde(with = "version")]
    pub protocol_version: u32,
    pub codex: Vec<QuotaBarDto>,
    pub antigravity: Vec<QuotaBarDto>,
    pub today_tokens: Option<u64>,
    pub today_coverage: Coverage,
    pub cache_hit_ratio: Option<f64>,
    pub latest_turn_cache_hit_ratio: Option<f64>,
    pub privacy_badge: PrivacyBadge,
    pub updated_at_ms: i64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DailyHeatmapCellDto {
    pub date_utc_start_ms: i64,
    pub total_tokens: u64,
    pub request_count: u64,
    /// Intensity bucket in 0..=4 (0 = inactive, 1..=4 = quartile activity tiers).
    pub level: u8,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelUsageDto {
    pub display_model: String,
    pub provider: ProviderId,
    pub metrics: UsageMetrics,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreditCardDto {
    pub state: CreditState,
    pub plan_type: Option<String>,
    pub service_tier: Option<String>,
    pub event_time_ms: Option<i64>,
    pub ingested_at_ms: i64,
    pub account_confirmed: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderCardDto {
    pub provider: ProviderId,
    pub availability: ProviderAvailability,
    pub quotas: Vec<QuotaBarDto>,
    pub credits: Option<CreditCardDto>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DashboardSnapshotDto {
    #[serde(with = "version")]
    pub protocol_version: u32,
    pub today: AggregateDto,
    pub last_7_days: AggregateDto,
    pub last_30_days: AggregateDto,
    pub model_distribution: Vec<ModelUsageDto>,
    pub providers: Vec<ProviderCardDto>,
    /// Share of today's tokens with an exact API-equivalent estimate.
    pub priced_token_coverage_ratio: Option<f64>,
    /// Cache hit ratio of the most recent request with prompt input tokens.
    pub latest_turn_cache_hit_ratio: Option<f64>,
    /// Daily 5-tier activity heatmap cells ordered chronologically.
    pub daily_heatmap: Vec<DailyHeatmapCellDto>,
    pub updated_at_ms: i64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum QueryGranularity {
    Hour,
    Day,
    Month,
}
impl QueryGranularity {
    pub fn max_span_ms(self) -> i64 {
        match self {
            Self::Hour => 31 * DAY_MS,
            Self::Day => 366 * DAY_MS,
            Self::Month => 3660 * DAY_MS,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsageQueryRequest {
    #[serde(with = "version")]
    pub protocol_version: u32,
    pub from_ms: i64,
    pub to_ms: i64,
    pub granularity: QueryGranularity,
    pub offset: u32,
    pub limit: u32,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryError {
    Version,
    TimeRange,
    Page,
    BucketOrder,
}
impl UsageQueryRequest {
    pub fn validate(&self) -> Result<(), QueryError> {
        if self.protocol_version != PROTOCOL_VERSION {
            return Err(QueryError::Version);
        }
        let span = self
            .to_ms
            .checked_sub(self.from_ms)
            .ok_or(QueryError::TimeRange)?;
        if !(1..=self.granularity.max_span_ms()).contains(&span) {
            return Err(QueryError::TimeRange);
        }
        if !(1..=MAX_PAGE_SIZE).contains(&self.limit)
            || self.offset.checked_add(self.limit).is_none()
        {
            return Err(QueryError::Page);
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsageQueryResponse {
    #[serde(with = "version")]
    pub protocol_version: u32,
    pub request: UsageQueryRequest,
    /// May be coarser for historical queries outside detail retention.
    pub available_granularity: QueryGranularity,
    pub coverage: Coverage,
    pub rows: Vec<AggregateDto>,
    pub next_offset: Option<u32>,
}
impl UsageQueryResponse {
    pub fn validate(&self) -> Result<(), QueryError> {
        if self.protocol_version != PROTOCOL_VERSION {
            return Err(QueryError::Version);
        }
        self.request.validate()?;
        if self.available_granularity.max_span_ms() < self.request.granularity.max_span_ms() {
            return Err(QueryError::TimeRange);
        }
        if self.rows.len() > self.request.limit as usize {
            return Err(QueryError::Page);
        }
        if let Some(offset) = self.next_offset {
            if self.rows.is_empty() || offset != self.request.offset + self.rows.len() as u32 {
                return Err(QueryError::Page);
            }
        }
        let mut end = self.request.from_ms;
        for row in &self.rows {
            if row.from_ms < end || row.from_ms >= row.to_ms || row.to_ms > self.request.to_ms {
                return Err(QueryError::BucketOrder);
            }
            end = row.to_ms;
        }
        Ok(())
    }
}

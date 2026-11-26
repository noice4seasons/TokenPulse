use crate::models::ProviderId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CreditState {
    Unknown,
    None,
    Unlimited,
    Limited { balance_text: Option<String> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum QuotaPeriodKind {
    FiveHour,
    Weekly,
    Unknown(Option<u64>),
}
impl From<Option<u64>> for QuotaPeriodKind {
    fn from(minutes: Option<u64>) -> Self {
        match minutes {
            Some(300) => Self::FiveHour,
            Some(10080) => Self::Weekly,
            other => Self::Unknown(other),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum QuotaSourceKind {
    LocalLogSession,
    LocalRpcVerified,
    CachedSnapshot,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum QuotaWindowState {
    Active {
        used_pct: f64,
        remaining_pct: f64,
        resets_in_ms: Option<i64>,
    },
    AwaitingNewActivity {
        last_used_pct: Option<f64>,
        expired_at_ms: i64,
    },
    PlanRestricted {
        reason: Option<String>,
    },
    NoData,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuotaObservation {
    pub provider: ProviderId,
    pub scope_id: String,
    pub account_confirmed: bool,
    pub target_id: String,
    pub target_label: String,
    pub period_kind: QuotaPeriodKind,
    pub used_pct: Option<f64>,
    pub resets_at_ms: Option<i64>,
    pub event_time_ms: Option<i64>,
    pub event_time_trusted: bool,
    pub ingested_at_ms: i64,
    pub source_kind: QuotaSourceKind,
}
impl QuotaObservation {
    pub fn window_state(&self, now_ms: i64) -> QuotaWindowState {
        let Some(used_pct) = self.used_pct.filter(|p| (0.0..=100.0).contains(p)) else {
            return QuotaWindowState::NoData;
        };
        if let Some(end) = self.resets_at_ms {
            if now_ms >= end {
                return QuotaWindowState::AwaitingNewActivity {
                    last_used_pct: Some(used_pct),
                    expired_at_ms: end,
                };
            }
        }
        QuotaWindowState::Active {
            used_pct,
            remaining_pct: 100.0 - used_pct,
            resets_in_ms: self.resets_at_ms.map(|end| end.saturating_sub(now_ms)),
        }
    }
}
pub fn should_alert_low_quota(
    obs: &QuotaObservation,
    threshold_remaining_pct: f64,
    now_ms: i64,
) -> bool {
    (0.0..=100.0).contains(&threshold_remaining_pct)
        && matches!(obs.window_state(now_ms),QuotaWindowState::Active{remaining_pct,..} if remaining_pct<=threshold_remaining_pct)
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreditObservation {
    pub provider: ProviderId,
    pub scope_id: String,
    pub event_time_ms: Option<i64>,
    pub ingested_at_ms: i64,
    pub plan_type: Option<String>,
    pub service_tier: Option<String>,
    pub state: CreditState,
    pub account_confirmed: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProviderAvailability {
    Available,
    NotRunning,
    AccessDenied,
    Unsupported,
    TlsUntrusted,
    CachedOnly,
    Disabled,
}

use crate::accounting::timestamp_ms;
use serde::Deserialize;
use serde_json::{value::RawValue, Value};
use tokenpulse_core_domain::{
    models::ProviderId,
    quota::{CreditObservation, CreditState, QuotaObservation, QuotaPeriodKind, QuotaSourceKind},
};

// Preserve even negative zero, trailing zeros, and exponent spelling. A Value
// alone cannot recover the source lexeme after parsing a JSON number.
#[derive(Deserialize)]
struct Envelope<'a> {
    #[serde(borrow)]
    payload: Payload<'a>,
}
#[derive(Deserialize)]
struct Payload<'a> {
    #[serde(borrow)]
    rate_limits: Limits<'a>,
}
#[derive(Deserialize)]
struct Limits<'a> {
    #[serde(borrow)]
    credits: Credit<'a>,
}
#[derive(Deserialize)]
struct Credit<'a> {
    #[serde(borrow)]
    balance: &'a RawValue,
}

fn reset_ms(v: &Value) -> Option<i64> {
    let n = v.as_i64()?;
    if n <= 0 {
        return None;
    }
    if n < 100_000_000_000 {
        n.checked_mul(1000)
    } else {
        Some(n)
    }
}
#[derive(Default, Debug)]
pub struct RateLimits {
    pub quota_observations: Vec<QuotaObservation>,
    pub credit_observations: Vec<CreditObservation>,
}
impl RateLimits {
    /// Retain each session/window and session balance independently. An event
    /// with no provider timestamp can never displace a dated observation.
    pub fn merge(&mut self, other: Self) {
        for q in other.quota_observations {
            if let Some(old) = self
                .quota_observations
                .iter_mut()
                .find(|old| old.scope_id == q.scope_id && old.target_id == q.target_id)
            {
                if q.event_time_ms > old.event_time_ms {
                    *old = q;
                }
            } else {
                self.quota_observations.push(q);
            }
        }
        for c in other.credit_observations {
            if let Some(old) = self
                .credit_observations
                .iter_mut()
                .find(|old| old.scope_id == c.scope_id)
            {
                if c.event_time_ms > old.event_time_ms {
                    *old = c;
                }
            } else {
                self.credit_observations.push(c);
            }
        }
    }
}

pub fn parse_rate_limits(
    line: &[u8],
    scope: &str,
    ingested_at_ms: i64,
) -> serde_json::Result<RateLimits> {
    let event: Value = serde_json::from_slice(line)?;
    Ok(from_event(&event, line, scope, ingested_at_ms))
}
pub(crate) fn from_event(
    event: &Value,
    line: &[u8],
    scope: &str,
    ingested_at_ms: i64,
) -> RateLimits {
    let mut out = RateLimits::default();
    let limits = &event["payload"]["rate_limits"];
    if !limits.is_object() {
        return out;
    }
    let time = event["timestamp"].as_str().and_then(timestamp_ms);
    for (key, label) in [("primary", "Primary"), ("secondary", "Secondary")] {
        let w = &limits[key];
        if !w.is_object() {
            continue;
        }
        out.quota_observations.push(QuotaObservation {
            provider: ProviderId::Codex,
            scope_id: scope.into(),
            account_confirmed: false,
            target_id: key.into(),
            target_label: label.into(),
            period_kind: QuotaPeriodKind::from(w["window_minutes"].as_u64()),
            used_pct: w["used_percent"]
                .as_f64()
                .filter(|n| n.is_finite() && (0.0..=100.0).contains(n)),
            resets_at_ms: reset_ms(&w["resets_at"]),
            event_time_ms: time,
            event_time_trusted: time.is_some(),
            ingested_at_ms,
            source_kind: QuotaSourceKind::LocalLogSession,
        });
    }
    let c = &limits["credits"];
    let state = match (c["has_credits"].as_bool(), c["unlimited"].as_bool()) {
        (Some(false), _) => CreditState::None,
        (Some(true), Some(true)) => CreditState::Unlimited,
        (Some(true), Some(false)) => {
            let balance_text = match &c["balance"] {
                Value::String(s) => Some(s.clone()),
                Value::Number(_) => serde_json::from_slice::<Envelope<'_>>(line)
                    .ok()
                    .map(|e| e.payload.rate_limits.credits.balance.get().to_owned()),
                _ => None,
            };
            CreditState::Limited { balance_text }
        }
        _ => CreditState::Unknown,
    };
    // A window-only event has no timestamp for a credit or plan observation.
    // Keep Unknown representable without advancing the previous balance's clock.
    let credit_time =
        time.filter(|_| limits.get("credits").is_some() || limits.get("plan_type").is_some());
    out.credit_observations.push(CreditObservation {
        provider: ProviderId::Codex,
        scope_id: scope.into(),
        event_time_ms: credit_time,
        ingested_at_ms,
        plan_type: limits["plan_type"].as_str().map(str::to_owned),
        service_tier: None,
        state,
        account_confirmed: false,
    });
    out
}

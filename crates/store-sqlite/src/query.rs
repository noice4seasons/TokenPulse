use crate::{provider, schema, timezone::*, Result, StoreError, StoreHandle};
use rusqlite::{params, Connection, TransactionBehavior};
use serde::Serialize;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokenpulse_core_domain::{dto::*, models::*, quota::*};

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ExactRange {
    pub aggregate: AggregateDto,
    pub available_granularity: QueryGranularity,
    pub models: Vec<ModelUsageDto>,
}
pub type CancelToken = Arc<AtomicBool>;

const PAGE_SQL: &str = "SELECT json_extract(value,'$[0]'),json_extract(value,'$[1]') FROM json_each(?1) ORDER BY key LIMIT ?2 OFFSET ?3";

impl StoreHandle {
    /// Bind cancellation to this clone only. Shared in-memory reads remove the
    /// progress callback before the next message is serviced.
    pub fn with_cancellation(&self, token: CancelToken) -> Self {
        let mut handle = self.clone();
        handle.cancel = Some(token);
        handle
    }
    fn snapshot<T: Send + 'static>(
        &self,
        f: impl FnOnce(&Connection) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let cancel = self.cancel.clone();
        self.read(move |db| {
            if cancel.as_ref().is_some_and(|v| v.load(Ordering::Relaxed)) {
                return Err(StoreError::Cancelled);
            }
            if let Some(v) = cancel.clone() {
                db.progress_handler(500, Some(move || v.load(Ordering::Relaxed)))
            }
            let result = (|| {
                let tx = db.transaction_with_behavior(TransactionBehavior::Deferred)?;
                let out = f(&tx)?;
                tx.commit()?;
                Ok(out)
            })();
            db.progress_handler(0, None::<fn() -> bool>);
            if cancel.as_ref().is_some_and(|v| v.load(Ordering::Relaxed)) {
                Err(StoreError::Cancelled)
            } else {
                result
            }
        })
    }
    pub fn query_exact_range(
        &self,
        from_ms: i64,
        to_ms: i64,
        provider_filter: Option<ProviderId>,
    ) -> Result<ExactRange> {
        self.snapshot(move |db| exact(db, from_ms, to_ms, provider_filter))
    }
    pub fn model_distribution(
        &self,
        from: i64,
        to: i64,
        p: Option<ProviderId>,
    ) -> Result<ExactRange> {
        self.query_exact_range(from, to, p)
    }
    pub fn query_series(&self, request: UsageQueryRequest) -> Result<UsageQueryResponse> {
        request
            .validate()
            .map_err(|_| StoreError::Invalid("invalid series request"))?;
        self.snapshot(move |db| {
            let partial = request.from_ms < schema::retained(db)?;
            let g = match (partial, request.granularity) {
                (true, QueryGranularity::Hour) => QueryGranularity::Day,
                (_, g) => g,
            };
            let mut cursor = request.from_ms;
            // Build UTC calendar boundaries only. SQLite pages this small calendar
            // before touching either the detail index or the bucket index.
            let mut ranges = Vec::new();
            while cursor < request.to_ms {
                let boundary = match g {
                    QueryGranularity::Month => next_month(cursor).unwrap_or(i64::MAX),
                    QueryGranularity::Hour => {
                        cursor.saturating_add(HOUR_MS - cursor.rem_euclid(HOUR_MS))
                    }
                    QueryGranularity::Day => {
                        cursor.saturating_add(DAY_MS - cursor.rem_euclid(DAY_MS))
                    }
                };
                let end = boundary.min(request.to_ms);
                ranges.push([cursor, end]);
                cursor = end;
            }
            let mut q = db.prepare(PAGE_SQL)?;
            let page = q.query_map(
                params![
                    serde_json::to_string(&ranges)?,
                    request.limit,
                    request.offset
                ],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
            )?;
            let mut rows = Vec::new();
            for bounds in page {
                let (a, b) = bounds?;
                rows.push(exact(db, a, b, None)?.aggregate);
            }
            let end = request.offset + rows.len() as u32;
            Ok(UsageQueryResponse {
                protocol_version: PROTOCOL_VERSION,
                next_offset: ((end as usize) < ranges.len()).then_some(end),
                request,
                rows,
                available_granularity: g,
                coverage: coverage(db, partial, None)?,
            })
        })
    }
    pub fn latest_quotas(
        &self,
        p: Option<ProviderId>,
        scope: Option<String>,
    ) -> Result<Vec<QuotaObservation>> {
        self.snapshot(move |db| quotas(db, p, scope.as_deref()))
    }
    pub fn latest_credits(
        &self,
        p: Option<ProviderId>,
        scope: Option<String>,
    ) -> Result<Vec<CreditObservation>> {
        self.snapshot(move |db| credits(db, p, scope.as_deref()))
    }
    /// Scope selects one adapter instance/session. Without a selected scope,
    /// provider cards contain no account data.
    pub fn query_dashboard_snapshot(
        &self,
        now: i64,
        tz: TimeZoneSpec,
        scope: Option<String>,
    ) -> Result<DashboardSnapshotDto> {
        self.snapshot(move |db| {
            let range = |days| -> Result<ExactRange> {
                let (a, b) = last_n_days_bounds_utc(now, days, tz)?;
                exact(db, a, b, None)
            };
            let today = range(1)?;
            let mut providers = vec![];
            for p in [ProviderId::Codex, ProviderId::Antigravity] {
                let (observations, credit) = if let Some(id) = scope.as_deref() {
                    (
                        quotas(db, Some(p), Some(id))?,
                        credits(db, Some(p), Some(id))?.pop(),
                    )
                } else {
                    (vec![], None)
                };
                providers.push(ProviderCardDto {
                    provider: p,
                    availability: ProviderAvailability::CachedOnly,
                    quotas: observations
                        .into_iter()
                        .map(|q| QuotaBarDto {
                            state: q.window_state(now),
                            observed_at_ms: Some(
                                q.event_time_ms
                                    .filter(|_| q.event_time_trusted)
                                    .unwrap_or(q.ingested_at_ms),
                            ),
                            target_label: q.target_label,
                            period_kind: q.period_kind,
                            account_confirmed: q.account_confirmed,
                        })
                        .collect(),
                    credits: credit.map(|c| CreditCardDto {
                        state: c.state,
                        plan_type: c.plan_type,
                        service_tier: c.service_tier,
                        event_time_ms: c.event_time_ms,
                        ingested_at_ms: c.ingested_at_ms,
                        account_confirmed: c.account_confirmed,
                    }),
                });
            }
            Ok(DashboardSnapshotDto {
                protocol_version: PROTOCOL_VERSION,
                priced_token_coverage_ratio: today.aggregate.metrics.priced_token_coverage_ratio(),
                today: today.aggregate,
                last_7_days: range(7)?.aggregate,
                last_30_days: range(30)?.aggregate,
                model_distribution: today.models,
                providers,
                updated_at_ms: now,
            })
        })
    }
}
fn coverage(
    db: &Connection,
    partial: bool,
    provider_filter: Option<ProviderId>,
) -> Result<Coverage> {
    if partial {
        return Ok(Coverage::Partial);
    }
    let gaps: Option<bool> = db.query_row(
        "SELECT max(scan_incomplete) FROM usage_source WHERE ?1 IS NULL OR provider=?1",
        [provider_filter.map(provider)],
        |row| row.get(0),
    )?;
    Ok(match gaps {
        None => Coverage::Unknown,
        Some(true) => Coverage::Partial,
        Some(false) => Coverage::Complete,
    })
}
const COLUMNS:&str="request_count,input_tokens,non_cached_input_tokens,cache_read_tokens,cache_creation_tokens,output_tokens,reasoning_output_tokens,total_tokens,priced_request_count,unpriced_request_count,priced_tokens,cost_micro_usd,cache_savings_micro_usd";
const TOTALS:&str="sum(request_count),sum(input_tokens),sum(non_cached_input_tokens),sum(cache_read_tokens),sum(cache_creation_tokens),sum(output_tokens),sum(reasoning_output_tokens),sum(total_tokens),sum(priced_request_count),sum(unpriced_request_count),sum(priced_tokens),sum(cost_micro_usd),sum(cache_savings_micro_usd)";
fn exact(db: &Connection, from: i64, to: i64, p: Option<ProviderId>) -> Result<ExactRange> {
    if from >= to {
        return Err(StoreError::Invalid("empty time range"));
    }
    let left = from.checked_add((HOUR_MS - from.rem_euclid(HOUR_MS)) % HOUR_MS);
    let right = to.checked_sub(to.rem_euclid(HOUR_MS));
    let interior = left.zip(right).filter(|(a, b)| a < b);
    let detail = |lo: &str, hi: &str| {
        format!("SELECT provider,display_model,{COLUMNS} FROM entry_metrics WHERE timestamp_ms>={lo} AND timestamp_ms<{hi} AND (?3 IS NULL OR provider=?3)")
    };
    let mut parts = vec![];
    if let Some((a, b)) = interior {
        parts.push(format!("SELECT provider,display_model,{COLUMNS} FROM usage_bucket WHERE bucket_kind='hour' AND bucket_start_ms>=?4 AND bucket_start_ms<?5 AND (?3 IS NULL OR provider=?3)"));
        if from < a {
            parts.push(detail("?1", "?4"))
        }
        if b < to {
            parts.push(detail("?5", "?2"))
        }
    } else {
        parts.push(detail("?1", "?2"))
    }
    let sql=format!("SELECT provider,display_model,{TOTALS} FROM ({}) GROUP BY provider,display_model ORDER BY sum(total_tokens) DESC,provider,display_model",parts.join(" UNION ALL "));
    let mut stmt = db.prepare_cached(&sql)?;
    let filter = p.map(provider);
    let mut rows = if let Some((a, b)) = interior {
        stmt.query(params![from, to, filter, a, b])?
    } else {
        stmt.query(params![from, to, filter])?
    };
    let mut metrics = UsageMetrics::default();
    let mut models = vec![];
    while let Some(r) = rows.next()? {
        let part = UsageMetrics {
            request_count: r.get(2)?,
            input_tokens: r.get(3)?,
            non_cached_input_tokens: r.get(4)?,
            cache_read_tokens: r.get(5)?,
            cache_creation_tokens: r.get(6)?,
            output_tokens: r.get(7)?,
            reasoning_output_tokens: r.get(8)?,
            total_tokens: r.get(9)?,
            priced_request_count: r.get(10)?,
            unpriced_request_count: r.get(11)?,
            priced_tokens: r.get(12)?,
            cost_micro_usd: r.get(13)?,
            cache_savings_micro_usd: r.get(14)?,
        };
        metrics = metrics
            .checked_add(&part)
            .ok_or(StoreError::Invalid("aggregate overflow"))?;
        models.push(ModelUsageDto {
            provider: crate::parse_provider(&r.get::<_, String>(0)?)?,
            display_model: r.get(1)?,
            metrics: part,
        });
    }
    let partial = from < schema::retained(db)?;
    Ok(ExactRange {
        aggregate: AggregateDto {
            from_ms: from,
            to_ms: to,
            metrics,
            coverage: coverage(db, partial, p)?,
        },
        models,
        available_granularity: if partial {
            QueryGranularity::Day
        } else {
            QueryGranularity::Hour
        },
    })
}
fn quotas(
    db: &Connection,
    p: Option<ProviderId>,
    scope: Option<&str>,
) -> Result<Vec<QuotaObservation>> {
    let mut stmt=db.prepare_cached("SELECT observation_json FROM (SELECT *,row_number() OVER (PARTITION BY provider,scope_id,target_id,period_kind ORDER BY effective_ms DESC) AS n FROM quota_observation WHERE (?1 IS NULL OR provider=?1) AND (?2 IS NULL OR scope_id=?2)) WHERE n=1 ORDER BY provider,scope_id,target_id,period_kind")?;
    let rows = stmt.query_map(params![p.map(provider), scope], |r| {
        let s: String = r.get(0)?;
        serde_json::from_str(&s).map_err(crate::json_column)
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}
fn credits(
    db: &Connection,
    p: Option<ProviderId>,
    scope: Option<&str>,
) -> Result<Vec<CreditObservation>> {
    let mut stmt=db.prepare_cached("SELECT provider,scope_id,event_time_ms,ingested_at_ms,plan_type,service_tier,state_json,account_confirmed FROM (SELECT *,row_number() OVER (PARTITION BY provider,scope_id ORDER BY effective_ms DESC) AS n FROM credit_observation WHERE (?1 IS NULL OR provider=?1) AND (?2 IS NULL OR scope_id=?2)) WHERE n=1 ORDER BY provider,scope_id")?;
    let rows = stmt.query_map(params![p.map(provider), scope], |r| {
        Ok(CreditObservation {
            provider: crate::parse_provider(&r.get::<_, String>(0)?)?,
            scope_id: r.get(1)?,
            event_time_ms: r.get(2)?,
            ingested_at_ms: r.get(3)?,
            plan_type: r.get(4)?,
            service_tier: r.get(5)?,
            state: serde_json::from_str(&r.get::<_, String>(6)?).map_err(crate::json_column)?,
            account_confirmed: r.get(7)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

#![forbid(unsafe_code)]
pub mod export;
pub mod query;
mod schema;
pub mod timezone;
pub mod writer;
pub use export::{export, ExportFormat};
pub use query::{CancelToken, ExactRange};
pub use timezone::*;
use tokenpulse_core_domain::models::ProviderId;
pub use writer::*;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("SQLite error: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("invalid metadata encoding: {0}")]
    Json(#[from] serde_json::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Invalid(&'static str),
    #[error("writer stopped")]
    Stopped,
    #[error("query cancelled")]
    Cancelled,
}
pub type Result<T> = std::result::Result<T, StoreError>;
fn provider(p: ProviderId) -> &'static str {
    match p {
        ProviderId::Codex => "codex",
        ProviderId::Antigravity => "antigravity",
    }
}
fn parse_provider(s: &str) -> rusqlite::Result<ProviderId> {
    match s {
        "codex" => Ok(ProviderId::Codex),
        "antigravity" => Ok(ProviderId::Antigravity),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}
fn json_column(e: serde_json::Error) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{atomic::AtomicBool, Arc};
    use tokenpulse_core_domain::{dto::*, models::*, quota::*};
    fn entry(id: &str, ms: i64) -> UsageEntry {
        UsageEntry {
            raw_model: "private raw text".into(),
            display_model: "GPT-4".into(),
            request_id: Some(id.into()),
            identity_key: format!("stable-{id}"),
            identity_origin: "session".into(),
            identity_aliases: vec![format!("alias-{id}")],
            timestamp_ms: ms,
            provider: ProviderId::Codex,
            tokens: TokenVector {
                input_tokens: 12,
                non_cached_input_tokens: 5,
                cache_read_tokens: 6,
                cache_creation_tokens: 1,
                output_tokens: 7,
                reasoning_output_tokens: 3,
            },
            cost_micro_usd: Some(123),
            cache_savings_micro_usd: Some(11),
            price_rule_id: Some("r".into()),
            pricing_precision: PricingPrecision::Exact {
                rule_id: "r".into(),
            },
            single_request_evidence: true,
        }
    }
    fn batch(store: &StoreHandle, id: &str, entries: Vec<UsageEntry>) -> CommitBatch {
        let old = store.source_state(id).unwrap();
        let expected = old
            .as_ref()
            .map(SourceState::expected_version)
            .unwrap_or_default();
        let offset = expected.expected_offset.unwrap_or(0) + 1;
        CommitBatch {
            expected,
            source: SourceState {
                source_id: id.into(),
                provider: ProviderId::Codex,
                current_path_hash: "old".into(),
                volume_serial: "vol".into(),
                file_index: id.into(),
                creation_time_100ns: None,
                generation: 1,
                committed_offset: offset,
                version_token: format!("v{offset}"),
                checkpoint_json: "{}".into(),
                resume_state_json: "{}".into(),
                sealed_before_ms: None,
                updated_at_ms: 0,
            },
            entries,
            identity_links: vec![],
            identity_seeds: vec![],
            mode: CommitMode::Tail,
            quotas: vec![],
            credits: vec![],
        }
    }
    fn rebuild(ids: &[&str], complete: bool) -> CommitMode {
        CommitMode::Rebuild {
            complete_without_gaps: complete,
            seen_request_ids: ids.iter().map(|s| s.to_string()).collect(),
        }
    }
    fn metrics(s: &StoreHandle) -> UsageMetrics {
        s.query_exact_range(0, DAY_MS, None)
            .unwrap()
            .aggregate
            .metrics
    }
    fn committed(s: &StoreHandle, b: CommitBatch) -> CommitCounts {
        match s.commit(b).unwrap() {
            CommitOutcome::Committed(c) => c,
            other => panic!("unexpected {other:?}"),
        }
    }
    #[test]
    fn rebuild_needs_set_difference_and_proof() {
        let s = StoreHandle::open_in_memory().unwrap();
        let rows = vec![entry("a", 1), entry("b", HOUR_MS)];
        for n in 0..2 {
            let mut b = batch(&s, "s", rows.clone());
            b.mode = rebuild(&["a", "b"], true);
            let c = committed(&s, b);
            assert_eq!(c.removed, 0);
            assert_eq!(c.unchanged, 2 * n);
            assert_eq!(metrics(&s).request_count, 2);
        }
        let mut b = batch(&s, "s", vec![]);
        b.mode = rebuild(&["a"], false);
        assert_eq!(committed(&s, b).removed, 0);
        assert_eq!(metrics(&s).request_count, 2);
        let mut revised = entry("a", 2 * HOUR_MS);
        revised.tokens.output_tokens = 9;
        let mut b = batch(&s, "s", vec![revised.clone()]);
        b.mode = rebuild(&["a"], true);
        let c = committed(&s, b);
        assert_eq!((c.updated, c.removed), (1, 1));
        assert_eq!(metrics(&s), UsageMetrics::from_entry(&revised).unwrap());
        assert_eq!(
            s.query_exact_range(0, 2 * HOUR_MS, None)
                .unwrap()
                .aggregate
                .metrics,
            UsageMetrics::default()
        );
    }
    #[test]
    fn stale_writers_and_aliases() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.db");
        let s = StoreHandle::open(&path).unwrap();
        let b = batch(&s, "s", vec![entry("a", 1)]);
        let other = StoreHandle::open(&path).unwrap();
        let pending = b.clone();
        let thread = std::thread::spawn(move || other.commit(pending).unwrap());
        let outcomes = [s.commit(b).unwrap(), thread.join().unwrap()];
        assert_eq!(
            outcomes
                .iter()
                .filter(|o| **o == CommitOutcome::StaleConflict)
                .count(),
            1
        );
        let stale = batch(&s, "s", vec![]);
        committed(&s, stale.clone());
        assert_eq!(s.commit(stale).unwrap(), CommitOutcome::StaleConflict);
        assert_eq!(s.source_state("s").unwrap().unwrap().committed_offset, 2);
        assert!(!s.update_source_path_alias("s", "wrong", "s", "no").unwrap());
        assert!(s.update_source_path_alias("s", "vol", "s", "new").unwrap());
        assert_eq!(
            s.source_state("s").unwrap().unwrap().current_path_hash,
            "new"
        );
        let mut copy = entry("b", 1);
        copy.identity_aliases = vec!["alias-a".into()];
        let mut b = batch(&s, "copy", vec![copy]);
        b.source.volume_serial = "vol2".into();
        assert_eq!(committed(&s, b).skipped, 1);
        assert_eq!(metrics(&s).request_count, 1);
    }
    #[test]
    fn calendar_edges_equal_detail_fold() {
        let s = StoreHandle::open_in_memory().unwrap();
        let tz = TimeZoneSpec::DstRule {
            std_offset_minutes: -300,
            dst_offset_minutes: -240,
            spring_transition_utc_ms: 7 * HOUR_MS,
            fall_transition_utc_ms: 181 * DAY_MS + 6 * HOUR_MS,
        };
        let spring = today_bounds_utc(12 * HOUR_MS, tz).unwrap();
        let fall = today_bounds_utc(181 * DAY_MS + 12 * HOUR_MS, tz).unwrap();
        assert_eq!(spring.1 - spring.0, 23 * HOUR_MS);
        assert_eq!(fall.1 - fall.0, 25 * HOUR_MS);
        let mut ranges = vec![
            (1, 37),
            (HOUR_MS / 2, 3 * DAY_MS + 1),
            spring,
            fall,
            (-HOUR_MS, 0),
        ];
        for off in [330, 345] {
            ranges.push(
                today_bounds_utc(
                    0,
                    TimeZoneSpec::FixedOffset {
                        offset_minutes: off,
                    },
                )
                .unwrap(),
            );
        }
        let mut rows = vec![];
        for &(a, b) in &ranges {
            for ts in [a - 1, a, a + 1, a + (b - a) / 2, b - 1, b] {
                let mut e = entry(&rows.len().to_string(), ts);
                if rows.len() % 2 == 0 {
                    e.pricing_precision = PricingPrecision::Unpriced {
                        reason: UnpricedReason::UnknownModel,
                    };
                    e.price_rule_id = None;
                    e.cost_micro_usd = None;
                    e.cache_savings_micro_usd = None;
                }
                rows.push(e);
            }
        }
        committed(&s, batch(&s, "s", rows.clone()));
        for (a, b) in ranges {
            let expected = rows
                .iter()
                .filter(|e| (a..b).contains(&e.timestamp_ms))
                .fold(UsageMetrics::default(), |sum, e| {
                    sum.checked_add(&UsageMetrics::from_entry(e).unwrap())
                        .unwrap()
                });
            assert_eq!(
                s.query_exact_range(a, b, None).unwrap().aggregate.metrics,
                expected
            );
        }
    }
    #[test]
    fn compact_replay_and_series() {
        let s = StoreHandle::open_in_memory().unwrap();
        committed(&s, batch(&s, "s", vec![entry("a", 1), entry("b", DAY_MS)]));
        assert_eq!(s.compact(2 * DAY_MS, 1).unwrap(), 1);
        committed(&s, batch(&s, "s", vec![entry("a", 1)]));
        let q = s.query_exact_range(0, DAY_MS, None).unwrap();
        assert_eq!(q.aggregate.metrics.request_count, 1);
        assert_eq!(
            (q.aggregate.coverage, q.available_granularity),
            (Coverage::Partial, QueryGranularity::Day)
        );
        let req = UsageQueryRequest {
            protocol_version: 1,
            from_ms: 0,
            to_ms: 2 * DAY_MS,
            granularity: QueryGranularity::Hour,
            offset: 0,
            limit: 1,
        };
        let response = s.query_series(req).unwrap();
        response.validate().unwrap();
        assert_eq!(response.next_offset, Some(1));
        assert!(matches!(
            s.with_cancellation(Arc::new(AtomicBool::new(true)))
                .query_exact_range(0, 1, None),
            Err(StoreError::Cancelled)
        ));
    }
    #[test]
    fn observation_order_and_atomic_rollback() {
        let s = StoreHandle::open_in_memory().unwrap();
        let mut q = QuotaObservation {
            provider: ProviderId::Codex,
            scope_id: "local".into(),
            target_id: "x".into(),
            target_label: "bar".into(),
            period_kind: QuotaPeriodKind::FiveHour,
            used_pct: Some(54.5),
            resets_at_ms: None,
            event_time_ms: Some(90),
            event_time_trusted: true,
            ingested_at_ms: 100,
            account_confirmed: false,
            source_kind: QuotaSourceKind::LocalLogSession,
        };
        let c = CreditObservation {
            provider: q.provider,
            scope_id: q.scope_id.clone(),
            event_time_ms: Some(65),
            ingested_at_ms: 100,
            plan_type: None,
            service_tier: None,
            state: CreditState::Limited {
                balance_text: Some("1.000".into()),
            },
            account_confirmed: false,
        };
        let mut b = batch(&s, "s", vec![]);
        b.credits = vec![c.clone()];
        b.quotas = vec![q.clone(), q.clone()];
        committed(&s, b);
        let newer = q.clone();
        q.event_time_ms = Some(30);
        q.ingested_at_ms = 500;
        let mut b = batch(&s, "s", vec![]);
        b.quotas.push(q);
        committed(&s, b);
        assert_eq!(s.latest_quotas(None, None).unwrap(), vec![newer]);
        assert_eq!(s.latest_credits(None, None).unwrap(), vec![c]);
        let mut bad = entry("bad", 3);
        bad.cost_micro_usd = None;
        assert!(s
            .commit(batch(&s, "s", vec![entry("good", 2), bad]))
            .is_err());
        assert_eq!(metrics(&s).request_count, 0);
        assert_eq!(s.source_state("s").unwrap().unwrap().committed_offset, 2);
    }
    #[test]
    fn checkpoint_metadata_is_validated_and_preserved_verbatim() {
        let s = StoreHandle::open_in_memory().unwrap();
        let mut b = batch(&s, "s", vec![]);
        b.source.resume_state_json = r#"{"message":"private text"}"#.into();
        assert!(s.commit(b.clone()).is_err());
        assert!(s.source_state("s").unwrap().is_none());
        b.source.resume_state_json = r#"{ "segment" : 3 }"#.into();
        b.source.checkpoint_json = r#"{ "version": 1 }"#.into();
        committed(&s, b.clone());
        let saved = s.source_state("s").unwrap().unwrap();
        assert_eq!(saved.resume_state_json, b.source.resume_state_json);
        assert_eq!(saved.checkpoint_json, b.source.checkpoint_json);
    }
    #[test]
    fn gaps_and_provider_filter_reach_dashboard() {
        let s = StoreHandle::open_in_memory().unwrap();
        let tz = TimeZoneSpec::FixedOffset {
            offset_minutes: 330,
        };
        assert_eq!(
            s.query_dashboard_snapshot(0, tz, None)
                .unwrap()
                .today
                .coverage,
            Coverage::Unknown
        );
        let mut b = batch(&s, "s", vec![entry("a", 1)]);
        b.mode = rebuild(&["a"], false);
        committed(&s, b);
        let snapshot = s.query_dashboard_snapshot(2, tz, None).unwrap();
        assert_eq!(snapshot.today.coverage, Coverage::Partial);
        assert_eq!(snapshot.today.metrics.request_count, 1);
        let filtered = s
            .query_exact_range(0, DAY_MS, Some(ProviderId::Antigravity))
            .unwrap();
        assert!(filtered.models.is_empty());
        let mut b = batch(&s, "s", vec![]);
        b.mode = rebuild(&["a"], true);
        committed(&s, b);
        assert_eq!(
            s.query_dashboard_snapshot(2, tz, None)
                .unwrap()
                .today
                .coverage,
            Coverage::Complete
        );
    }
    #[test]
    fn export_has_only_aggregate_fields() {
        let s = StoreHandle::open_in_memory().unwrap();
        let mut e = entry("credential-secret", 2);
        e.display_model = r"C:\Users\private\log".into();
        committed(&s, batch(&s, "s", vec![e]));
        let q = s.query_exact_range(0, 5, None).unwrap();
        for format in [ExportFormat::Json, ExportFormat::Csv] {
            let data = export(&q, format).unwrap();
            assert!(data.contains("model-1"));
            assert!(data.contains("cost_micro_usd"));
            for forbidden in ["private", "Users", "credential", "identity", "session"] {
                assert!(!data.contains(forbidden));
            }
            if matches!(format, ExportFormat::Json) {
                assert_eq!(
                    serde_json::from_str::<serde_json::Value>(&data).unwrap()[0]["total_tokens"],
                    19
                );
            } else {
                assert_eq!(data.lines().count(), 3);
            }
        }
    }
    #[test]
    fn latest_turn_cache_ratio_and_daily_heatmap() {
        let s = StoreHandle::open_in_memory().unwrap();
        let tz = TimeZoneSpec::FixedOffset { offset_minutes: 0 };
        assert_eq!(s.latest_turn_cache_hit_ratio(None).unwrap(), None);
        let mut e1 = entry("r1", 10);
        e1.tokens = TokenVector {
            input_tokens: 100,
            non_cached_input_tokens: 20,
            cache_read_tokens: 80,
            cache_creation_tokens: 0,
            output_tokens: 10,
            reasoning_output_tokens: 0,
        };
        e1.reprice();
        let mut e2 = entry("r2", DAY_MS + 10);
        e2.tokens = TokenVector {
            input_tokens: 400,
            non_cached_input_tokens: 300,
            cache_read_tokens: 100,
            cache_creation_tokens: 0,
            output_tokens: 20,
            reasoning_output_tokens: 0,
        };
        e2.reprice();
        committed(&s, batch(&s, "s", vec![e1, e2]));
        let latest = s.latest_turn_cache_hit_ratio(Some(ProviderId::Codex)).unwrap().unwrap();
        assert!((latest - 0.25).abs() < 1e-9);
        let hm = s.query_daily_heatmap(DAY_MS + 100, 3, tz, None).unwrap();
        assert_eq!(hm.len(), 3);
        assert_eq!(hm[0].level, 0);
        assert_eq!(hm[1].level, 2);
        assert_eq!(hm[2].level, 4);
    }
}

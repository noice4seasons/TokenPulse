#![forbid(unsafe_code)]
pub mod config;
pub mod dto;
pub mod models;
pub mod pricing;
pub mod quota;

#[cfg(test)]
mod tests {
    use super::{config::*, dto::*, models::*, pricing::*, quota::*};

    fn vector(input: u64, read: u64, write: u64, out: u64) -> TokenVector {
        TokenVector {
            input_tokens: input + read,
            non_cached_input_tokens: input,
            cache_read_tokens: read,
            cache_creation_tokens: write,
            output_tokens: out,
            reasoning_output_tokens: 0,
        }
    }
    fn unpriced(reason: UnpricedReason) -> PricingPrecision {
        PricingPrecision::Unpriced { reason }
    }
    #[test]
    fn utc_price_revisions() {
        let cut = 1785369600000;
        let t = vector(10, 0, 0, 0);
        for (name, before, after) in [
            ("gpt-5.6-terra", 25, 20),
            ("gpt-5.6-luna", 10, 2),
            ("gpt-5.6-sol", 50, 50),
        ] {
            let price = |at| estimate_usage_cost(name, at, &t, true).unwrap();
            assert_eq!(price(cut - 1).cost_micro_usd, before);
            assert_eq!(price(cut).cost_micro_usd, after);
            assert_eq!(
                price(cut - 1).precision == price(cut).precision,
                before == after
            );
        }
        let launch = 1788393600000;
        assert!(resolve_rule("gpt-6-astra", launch - 1).is_none());
        assert_eq!(
            resolve_rule("gpt-6-astra", launch).unwrap().rule_id,
            "gpt-6-astra-standard-2026-09-03-v1"
        );
    }
    #[test]
    fn revisions_have_unique_ids_and_half_open_intervals() {
        for (i, r) in RULES.iter().enumerate() {
            assert!(RULES[..i].iter().all(|old| old.rule_id != r.rule_id));
            assert_eq!(
                resolve_rule(r.model, r.effective_from_ms).unwrap().rule_id,
                r.rule_id
            );
            if let Some(end) = r.effective_until_ms {
                assert_eq!(resolve_rule(r.model, end - 1).unwrap().rule_id, r.rule_id);
                assert_ne!(
                    resolve_rule(r.model, end).map(|x| x.rule_id),
                    Some(r.rule_id)
                );
            }
        }
    }
    #[test]
    fn long_context_requires_request_evidence() {
        let price = |tokens, evidence| estimate_usage_cost("gpt-5.6-sol", 0, &tokens, evidence);
        let at = vector(270_000, 1000, 1000, 2);
        assert_eq!(price(at, false), price(at, true));
        assert_eq!(price(at, false).unwrap().cost_micro_usd, 1_356_810);
        let above = vector(270_001, 1000, 1000, 2);
        assert_eq!(
            price(above, false),
            Err(unpriced(
                UnpricedReason::MissingSingleRequestEvidenceForLongContext
            ))
        );
        let exact = price(above, true).unwrap();
        assert_eq!(
            (exact.cost_micro_usd, exact.cache_savings_micro_usd),
            (2_713_600, 9000)
        );
        assert!(estimate_usage_cost("gpt-5.4", 0, &above, false).is_ok());
    }
    #[test]
    fn rounding_occurs_after_summing_components() {
        let estimate = |t| estimate_usage_cost("gpt-5.4-mini", 0, &t, true).unwrap();
        assert_eq!(estimate(vector(1, 0, 1, 0)).cost_micro_usd, 2);
        assert_eq!(estimate(vector(0, 6, 0, 0)).cost_micro_usd, 0);
        assert_eq!(estimate(vector(0, 7, 0, 0)).cost_micro_usd, 1);
        assert_eq!(estimate(vector(2, 0, 0, 0)).cost_usd(), 0.000002);
        let huge = vector(0, 0, 0, u64::MAX);
        assert_eq!(
            estimate_usage_cost("gpt-5.4", 0, &huge, true),
            Err(unpriced(UnpricedReason::InvalidTokenCount))
        );
    }
    #[test]
    fn unknown_models_and_invalid_counts() {
        for model in ["", "gpt-4o-unknown", "custom-gpt-5.4", "codex"] {
            assert_eq!(
                estimate_usage_cost(model, 0, &TokenVector::default(), true),
                Err(unpriced(UnpricedReason::UnknownModel))
            );
        }
        let mut t = vector(1, 0, 0, 0);
        t.reasoning_output_tokens = 1;
        assert_eq!(
            estimate_usage_cost("gpt-5.4", 0, &t, true),
            Err(unpriced(UnpricedReason::InvalidTokenCount))
        );
        assert_eq!(
            resolve_rule("GPT-4o-2024-08-06", 0).unwrap().model,
            "gpt-4o"
        );
    }
    #[test]
    fn token_dimensions_and_display() {
        let mut t = vector(3, 1, 2, 5);
        t.reasoning_output_tokens = 3;
        assert_eq!(t.total_tokens(), 11);
        assert_eq!(t.cache_hit_ratio(), Some(0.25));
        assert_eq!(vector(0, 0, 5, 0).cache_hit_ratio(), None);
        assert_eq!(normalize_codex_model("  "), "Unknown (codex)");
        assert_eq!(normalize_codex_model("gpt-5.4-mini"), "GPT-5.4-MINI");
        assert_eq!(normalize_codex_model("Custom-Model"), "Custom-Model");
        assert_eq!(
            serde_json::to_string(&ProviderId::Antigravity).unwrap(),
            r#""antigravity""#
        );
    }
    #[test]
    fn repricing_clears_stale_costs_and_updates_coverage() {
        let mut e = UsageEntry {
            raw_model: "gpt-5.6-sol".into(),
            display_model: "GPT-5.6-SOL".into(),
            request_id: None,
            identity_key: "k".into(),
            identity_origin: "local".into(),
            identity_aliases: vec![],
            timestamp_ms: 0,
            provider: ProviderId::Codex,
            tokens: vector(1, 0, 0, 0),
            cost_micro_usd: None,
            cache_savings_micro_usd: None,
            price_rule_id: None,
            pricing_precision: unpriced(UnpricedReason::UnknownModel),
            single_request_evidence: true,
        };
        e.reprice();
        let exact = UsageMetrics::from_entry(&e).unwrap();
        e.tokens = vector(272_001, 0, 0, 0);
        e.single_request_evidence = false;
        e.reprice();
        assert_eq!(
            (
                e.cost_micro_usd,
                e.cache_savings_micro_usd,
                e.price_rule_id.as_ref()
            ),
            (None, None, None)
        );
        let unknown = UsageMetrics::from_entry(&e).unwrap();
        let both = exact.checked_add(&unknown).unwrap();
        assert_eq!(both.priced_token_coverage_ratio(), Some(1.0 / 272002.0));
        assert_eq!(
            (
                both.priced_request_count,
                both.unpriced_request_count,
                both.cost_micro_usd
            ),
            (1, 1, 5)
        );
        assert_eq!(both.checked_sub(&unknown), Some(exact));
        assert!(exact.checked_sub(&both).is_none());
        assert!(UsageMetrics {
            request_count: u64::MAX,
            ..exact
        }
        .checked_add(&exact)
        .is_none());
        assert_eq!(UsageMetrics::default().priced_token_coverage_ratio(), None);
    }

    fn observation() -> QuotaObservation {
        QuotaObservation {
            provider: ProviderId::Codex,
            scope_id: "s".into(),
            account_confirmed: false,
            target_id: "p".into(),
            target_label: "Primary".into(),
            period_kind: QuotaPeriodKind::FiveHour,
            used_pct: Some(95.0),
            resets_at_ms: Some(100),
            event_time_ms: None,
            event_time_trusted: false,
            ingested_at_ms: 2,
            source_kind: QuotaSourceKind::LocalLogSession,
        }
    }
    #[test]
    fn quota_expiry_stops_alerts() {
        let obs = observation();
        assert_eq!(
            obs.window_state(99),
            QuotaWindowState::Active {
                used_pct: 95.,
                remaining_pct: 5.,
                resets_in_ms: Some(1)
            }
        );
        assert!(should_alert_low_quota(&obs, 5., 99));
        for now in [100, 101] {
            assert_eq!(
                obs.window_state(now),
                QuotaWindowState::AwaitingNewActivity {
                    last_used_pct: Some(95.),
                    expired_at_ms: 100
                }
            );
            assert!(!should_alert_low_quota(&obs, 10., now));
        }
    }
    #[test]
    fn missing_and_invalid_percentages() {
        for used_pct in [None, Some(f64::NAN), Some(-1.), Some(101.)] {
            let obs = QuotaObservation {
                used_pct,
                ..observation()
            };
            assert_eq!(obs.window_state(50), QuotaWindowState::NoData);
            assert!(!should_alert_low_quota(&obs, 100., 50));
        }
        for minutes in [None, Some(301)] {
            assert_eq!(
                QuotaPeriodKind::from(minutes),
                QuotaPeriodKind::Unknown(minutes)
            );
        }
        assert_eq!(QuotaPeriodKind::from(Some(10080)), QuotaPeriodKind::Weekly);
        assert_eq!(QuotaPeriodKind::from(Some(300)), QuotaPeriodKind::FiveHour);
    }
    #[test]
    fn credit_states_round_trip_exact_text() {
        for state in [
            CreditState::Unknown,
            CreditState::None,
            CreditState::Unlimited,
            CreditState::Limited { balance_text: None },
            CreditState::Limited {
                balance_text: Some("000.0100000000000000000001".into()),
            },
        ] {
            let json = serde_json::to_string(&state).unwrap();
            assert_eq!(serde_json::from_str::<CreditState>(&json).unwrap(), state);
        }
    }
    fn query() -> UsageQueryRequest {
        UsageQueryRequest {
            protocol_version: 1,
            from_ms: 0,
            to_ms: 1,
            granularity: QueryGranularity::Hour,
            limit: 200,
            offset: 0,
        }
    }
    #[test]
    fn query_bounds_and_wire_contract() {
        assert!(query().validate().is_ok());
        for q in [
            UsageQueryRequest {
                limit: 201,
                ..query()
            },
            UsageQueryRequest {
                limit: 0,
                ..query()
            },
            UsageQueryRequest {
                protocol_version: 2,
                ..query()
            },
            UsageQueryRequest {
                to_ms: 0,
                ..query()
            },
            UsageQueryRequest {
                from_ms: i64::MIN,
                to_ms: i64::MAX,
                ..query()
            },
            UsageQueryRequest {
                offset: u32::MAX,
                ..query()
            },
        ] {
            assert!(q.validate().is_err());
        }
        for g in [
            QueryGranularity::Hour,
            QueryGranularity::Day,
            QueryGranularity::Month,
        ] {
            let mut q = UsageQueryRequest {
                granularity: g,
                to_ms: g.max_span_ms(),
                ..query()
            };
            assert!(q.validate().is_ok());
            q.to_ms += 1;
            assert_eq!(q.validate(), Err(QueryError::TimeRange));
        }
        for (field, value) in [
            ("granularity", serde_json::json!("Second")),
            ("protocol_version", serde_json::json!(2)),
            ("extra", serde_json::json!(true)),
        ] {
            let mut wire = serde_json::to_value(query()).unwrap();
            wire[field] = value;
            assert!(serde_json::from_value::<UsageQueryRequest>(wire).is_err());
        }
    }
    #[test]
    fn response_rejects_overlap_and_invalid_pagination() {
        let row = AggregateDto {
            from_ms: 0,
            to_ms: 1,
            coverage: Coverage::Partial,
            metrics: UsageMetrics::default(),
        };
        let mut page = UsageQueryResponse {
            protocol_version: 1,
            request: query(),
            available_granularity: QueryGranularity::Day,
            coverage: Coverage::Partial,
            rows: vec![row.clone()],
            next_offset: None,
        };
        assert!(page.validate().is_ok());
        page.next_offset = Some(0);
        assert_eq!(page.validate(), Err(QueryError::Page));
        page.next_offset = None;
        page.rows.push(row);
        assert_eq!(page.validate(), Err(QueryError::BucketOrder));
    }
    #[test]
    fn config_clamps_both_ends() {
        let mut c = AppConfig::default();
        assert!(!c.antigravity.enabled);
        assert_eq!(c.antigravity.poll_interval_secs, 90);
        for (n, expected) in [(0, (30, 60, 0, 1)), (u32::MAX, (100, 300, 100, 1440))] {
            c.overlay_opacity_pct = n;
            c.antigravity.poll_interval_secs = n;
            c.low_quota_threshold_pct = n;
            c.cooldown_minutes = n;
            let fixed = c.clone().sanitized();
            assert_eq!(
                (
                    fixed.overlay_opacity_pct,
                    fixed.antigravity.poll_interval_secs,
                    fixed.low_quota_threshold_pct,
                    fixed.cooldown_minutes
                ),
                expected
            );
            assert_eq!(fixed.clone().sanitized(), fixed);
        }
    }
}

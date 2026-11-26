use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    cell::RefCell,
    fmt::Display,
    path::Path,
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use tokenpulse_core_domain::{
    dto::DashboardSnapshotDto,
    models::{ProviderId, UsageMetrics},
    quota::{CreditObservation, QuotaObservation},
};
use tokenpulse_provider_codex::{
    discover_jsonl_files, read_default_service_tier, scan_file, validate_safe_local_path,
    CodexRoots, DualTrackCheckpoint, ScanOptions,
};
use tokenpulse_store_sqlite::{
    export, CommitBatch, CommitMode, CommitOutcome, ExportFormat, SourceState, StoreHandle,
    TimeZoneSpec,
};

fn err(e: impl Display) -> String {
    e.to_string()
}

#[derive(Default, Debug, Serialize)]
pub struct SyncSummary {
    pub discovered_files: usize,
    pub scanned_files: usize,
    pub committed_entries: usize,
    pub skipped_stale: usize,
    pub max_validation_bytes_read: u64,
    pub total_validation_bytes_read: u64,
    pub total_payload_bytes_read: u64,
    pub rebased_files: usize,
    pub files_with_gaps: usize,
    pub elapsed_ms: u128,
    pub service_tier: Option<String>,
}

fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub fn sync_codex_once(
    roots: &CodexRoots,
    store: &StoreHandle,
    now_ms: i64,
    force_rebuild: bool,
) -> Result<SyncSummary, String> {
    let start = Instant::now();
    let files = discover_jsonl_files(roots).map_err(err)?;
    let mut summary = SyncSummary {
        discovered_files: files.len(),
        service_tier: read_default_service_tier(roots),
        ..Default::default()
    };
    for file in files {
        let path_hash = digest(file.path.as_os_str().as_encoded_bytes());
        let identity = &file.identity;
        let physical_id = identity
            .volume_serial
            .zip(identity.file_index)
            .map(|(v, i)| digest(format!("codex:file:{v}:{i}").as_bytes()));
        let mut old = match &physical_id {
            Some(id) => store.source_state(id).map_err(err)?,
            None => None,
        };
        if old.is_none() {
            old = store.source_state(&path_hash).map_err(err)?;
        }
        let id = old
            .as_ref()
            .map(|s| s.source_id.as_str())
            .or(physical_id.as_deref())
            .unwrap_or(&path_hash);
        let previous = old.as_ref().filter(|_| !force_rebuild);
        let checkpoint = previous
            .map(|s| serde_json::from_str::<DualTrackCheckpoint>(&s.checkpoint_json))
            .transpose()
            .map_err(err)?;
        let lookup_error = RefCell::new(None);
        let lookup = |key: &str| match store.identity_source(ProviderId::Codex, key) {
            Ok(owner) => owner,
            Err(e) => {
                *lookup_error.borrow_mut() = Some(err(e));
                None
            }
        };
        let mut options = ScanOptions::new(id, now_ms);
        options.checkpoint = checkpoint.as_ref();
        options.resume_state_json = previous.map(|s| s.resume_state_json.as_str());
        options.identity_source = Some(&lookup);
        options.is_cancelled = None;
        options.rebuild = force_rebuild;
        let mut scan = scan_file(roots, &file.path, &options).map_err(err)?;
        if let Some(e) = lookup_error.into_inner() {
            return Err(e);
        }
        let cp = &scan.next_checkpoint;
        if cp.file_identity.as_ref() != Some(identity) {
            return Err("source replaced between discovery and scan".into());
        }
        summary.scanned_files += 1;
        summary.rebased_files += usize::from(scan.rebased);
        summary.files_with_gaps += usize::from(scan.has_coverage_gap);
        summary.max_validation_bytes_read = summary
            .max_validation_bytes_read
            .max(scan.validation_bytes_read);
        summary.total_validation_bytes_read += scan.validation_bytes_read;
        summary.total_payload_bytes_read += scan.payload_bytes_read;
        let mode = if force_rebuild || scan.rebased || old.is_none() || scan.has_coverage_gap {
            CommitMode::Rebuild {
                complete_without_gaps: scan.full_rebuild_coverage(),
                seen_request_ids: scan
                    .entries
                    .iter()
                    .map(|e| e.request_id.as_ref().unwrap_or(&e.identity_key).clone())
                    .collect(),
            }
        } else {
            CommitMode::Tail
        };

        // Fold in-batch aliases into their owning entry; cross-batch links and
        // protected-prefix seeds persist atomically via usage_identity.
        let owners: std::collections::HashMap<_, _> = scan
            .entries
            .iter()
            .enumerate()
            .map(|(i, e)| (e.identity_key.clone(), i))
            .collect();
        let mut cross_batch_links = Vec::new();
        for link in &scan.identity_links {
            if let Some(&index) = owners.get(&link.target) {
                scan.entries[index].identity_aliases.push(link.alias.clone());
            } else {
                cross_batch_links.push((link.alias.clone(), link.target.clone()));
            }
        }
        let identity_seeds: Vec<(String, String, i64)> = scan
            .identity_seeds
            .iter()
            .map(|seed| (seed.key.clone(), seed.request_id.clone(), seed.timestamp_ms))
            .collect();
        for c in &mut scan.credit_observations {
            if c.service_tier.is_none() {
                c.service_tier.clone_from(&summary.service_tier);
            }
        }
        let volume = identity
            .volume_serial
            .map(|x| x.to_string())
            .unwrap_or_default();
        let index = identity
            .file_index
            .map(|x| x.to_string())
            .unwrap_or_default();
        let expected = old
            .as_ref()
            .map(SourceState::expected_version)
            .unwrap_or_default();
        let checkpoint_json = serde_json::to_string(cp).map_err(err)?;
        let revision = digest(
            format!(
                "{}|{}|{}",
                id,
                expected.expected_version_token.as_deref().unwrap_or(""),
                checkpoint_json
            )
            .as_bytes(),
        );
        // Keep the writer's optimistic-lock generation monotonic even when a
        // forced scan starts a fresh scanner generation at byte zero.
        let generation = match &old {
            Some(s) if force_rebuild || scan.rebased => {
                s.generation.checked_add(1).ok_or("generation overflow")?
            }
            Some(s) => s.generation,
            None => cp.generation,
        };
        if old.as_ref().is_some_and(|s| {
            s.current_path_hash != path_hash && s.volume_serial == volume && s.file_index == index
        }) && !store
            .update_source_path_alias(id, &volume, &index, &path_hash)
            .map_err(err)?
        {
            summary.skipped_stale += 1;
            continue;
        }
        let batch = CommitBatch {
            expected,
            source: SourceState {
                source_id: id.to_owned(),
                provider: ProviderId::Codex,
                current_path_hash: path_hash.clone(),
                volume_serial: volume,
                file_index: index,
                creation_time_100ns: identity
                    .creation_time_100ns
                    .map(i64::try_from)
                    .transpose()
                    .map_err(err)?,
                generation,
                committed_offset: cp.byte_offset,
                version_token: revision,
                checkpoint_json,
                resume_state_json: scan.resume_state_json,
                sealed_before_ms: old.as_ref().and_then(|s| s.sealed_before_ms),
                updated_at_ms: now_ms,
            },
            mode,
            entries: scan.entries,
            identity_links: cross_batch_links,
            identity_seeds,
            quotas: scan.quota_observations,
            credits: scan.credit_observations,
        };
        match store.commit(batch).map_err(err)? {
            CommitOutcome::Committed(counts) => summary.committed_entries += counts.inserted,
            CommitOutcome::StaleConflict => summary.skipped_stale += 1,
        }
    }
    summary.elapsed_ms = start.elapsed().as_millis();
    Ok(summary)
}

#[derive(Debug, Serialize)]
pub struct VerifyReport {
    pub passed: bool,
    pub round1: SyncSummary,
    pub round2: SyncSummary,
    pub round3: SyncSummary,
    pub round1_all_metrics: UsageMetrics,
    pub round2_all_metrics: UsageMetrics,
    pub round3_all_metrics: UsageMetrics,
    pub dashboard: DashboardSnapshotDto,
    pub latest_quotas: Vec<QuotaObservation>,
    pub latest_credits: Vec<CreditObservation>,
    pub export_json_bytes: usize,
    pub incremental_validation_within_limit: bool,
    pub incremental_no_inserts: bool,
    pub incremental_metrics_equal: bool,
    pub rebuild_metrics_equal: bool,
    pub export_privacy_passed: bool,
    pub report_privacy_passed: bool,
}

fn privacy_ok(json: &str, roots: &CodexRoots) -> bool {
    let home = roots
        .home
        .to_string_lossy()
        .replace('\\', "/")
        .to_lowercase();
    let normalized = json.replace("\\\\", "/").replace('\\', "/").to_lowercase();
    !normalized.contains(&home) && !normalized.contains("auth.json")
}

pub fn run_verification(roots: &CodexRoots, db_path: &Path) -> Result<VerifyReport, String> {
    let path = std::env::current_dir().map_err(err)?.join(db_path);
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let mut name = path.clone().into_os_string();
        name.push(suffix);
        let candidate = Path::new(&name);
        let parent = candidate.parent().ok_or("missing database parent")?;
        match validate_safe_local_path(parent, candidate) {
            Ok(_) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(_) => return Err("unsafe database location".into()),
        }
    }
    let store = StoreHandle::open(path).map_err(err)?;
    let now = i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(err)?
            .as_millis(),
    )
    .map_err(err)?;
    let all = || {
        store
            .query_exact_range(i64::MIN, i64::MAX, Some(ProviderId::Codex))
            .map_err(err)
    };
    let round1 = sync_codex_once(roots, &store, now, false)?;
    let first = all()?;
    let latest_quotas = store
        .latest_quotas(Some(ProviderId::Codex), None)
        .map_err(err)?;
    let latest_credits = store
        .latest_credits(Some(ProviderId::Codex), None)
        .map_err(err)?;
    let scope = latest_credits
        .iter()
        .max_by_key(|c| c.event_time_ms.unwrap_or(c.ingested_at_ms))
        .map(|c| c.scope_id.clone());
    let dashboard = store
        .query_dashboard_snapshot(now, TimeZoneSpec::FixedOffset { offset_minutes: 0 }, scope)
        .map_err(err)?;
    let exported = export(&first, ExportFormat::Json).map_err(err)?;
    let round2 = sync_codex_once(roots, &store, now, false)?;
    let second = all()?.aggregate.metrics;
    let round3 = sync_codex_once(roots, &store, now, true)?;
    let third = all()?.aggregate.metrics;
    let metrics = first.aggregate.metrics;
    let mut report = VerifyReport {
        passed: false,
        incremental_validation_within_limit: round2.max_validation_bytes_read <= 8192,
        incremental_no_inserts: round2.committed_entries == 0,
        incremental_metrics_equal: metrics == second,
        rebuild_metrics_equal: metrics == third,
        export_json_bytes: exported.len(),
        export_privacy_passed: privacy_ok(&exported, roots),
        report_privacy_passed: false,
        round1,
        round2,
        round3,
        round1_all_metrics: metrics,
        round2_all_metrics: second,
        round3_all_metrics: third,
        dashboard,
        latest_quotas,
        latest_credits,
    };
    report.report_privacy_passed = privacy_ok(&serde_json::to_string(&report).map_err(err)?, roots);
    report.passed = report.round1.discovered_files > 0
        && [&report.round1, &report.round2, &report.round3]
            .iter()
            .all(|r| r.skipped_stale == 0)
        && report.incremental_validation_within_limit
        && report.incremental_no_inserts
        && report.incremental_metrics_equal
        && report.rebuild_metrics_equal
        && report.export_privacy_passed
        && report.report_privacy_passed;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use std::{fs, io::Write, path::PathBuf};

    struct Fixture(PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn rows(path: &Path, events: &[Value]) {
        let mut file = fs::File::create(path).unwrap();
        for event in events {
            writeln!(file, "{event}").unwrap();
        }
    }
    fn notice(n: u64, last: u64) -> Value {
        json!({"type":"event_msg","payload":{"type":"token_count","info":{
            "total_token_usage":{"input_tokens":n},"last_token_usage":{"input_tokens":last}
        }}})
    }

    #[test]
    fn offline_fork_dual_views_and_archive_move() {
        let dir = std::env::temp_dir().join(format!(
            "tokenpulse-e2e-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&dir).unwrap();
        let fixture = Fixture(dir);
        let roots = CodexRoots::new(fixture.0.join("home"));
        let [active, archive, _] = roots.scan_roots();
        fs::create_dir_all(&active).unwrap();
        fs::create_dir(&archive).unwrap();
        fs::write(
            roots.global_state_path(),
            r#"{"electron-persisted-atom-state":{"default-service-tier":"fast"}}"#,
        )
        .unwrap();
        let parent = active.join("parent.jsonl");
        rows(
            &parent,
            &[
                json!({"type":"session_meta","payload":{"id":"parent","model":"gpt-5.4"}}),
                json!({"type":"event_msg","payload":{"type":"task_started","turn_id":"t"}}),
                notice(20, 20),
                json!({"type":"token_usage_record","payload":{"turn_id":"t","response_id":"reply",
                "usage":{"input_tokens":20},"thread_token_usage":{"input_tokens":100},"turn_token_usage":{"input_tokens":20}}}),
                json!({"timestamp":"2024-03-01T00:00:01Z","payload":{"rate_limits":{
                    "primary":{"window_minutes":300,"used_percent":17},
                    "credits":{"has_credits":true,"unlimited":false,"balance":"001.2300"}
                }}}),
            ],
        );
        rows(
            &active.join("child.jsonl"),
            &[
                json!({"type":"session_meta","payload":{"id":"child","timestamp":"2024-03-01T00:00:02.123Z","forked_from_id":"parent"}}),
                json!({"type":"event_msg","payload":{"type":"task_started","turn_id":"old","started_at":1709251201}}),
                notice(100, 100),
                json!({"type":"event_msg","payload":{"type":"task_started","turn_id":"child","started_at":1709251202}}),
                notice(105, 5),
            ],
        );
        fs::write(active.join("short.jsonl"), b"{}\nmalformed\n").unwrap();

        let db = fixture.0.join("usage.db");
        let report = run_verification(&roots, &db).unwrap();
        assert!(report.passed, "{report:?}");
        assert_eq!(report.round1_all_metrics.request_count, 2);
        assert_eq!(report.round1_all_metrics.input_tokens, 25);
        assert_eq!(report.round1.files_with_gaps, 1);
        assert_eq!(report.round2.total_payload_bytes_read, 0);
        assert_eq!(report.latest_quotas.len(), 1);
        assert_eq!(report.latest_credits.len(), 1);
        assert_eq!(
            report.latest_credits[0].service_tier.as_deref(),
            Some("fast")
        );
        assert_eq!(
            report.latest_credits[0].state,
            tokenpulse_core_domain::quota::CreditState::Limited {
                balance_text: Some("001.2300".into())
            }
        );

        let moved = archive.join("moved.jsonl");
        fs::rename(parent, &moved).unwrap();
        let after = run_verification(&roots, &db).unwrap();
        assert!(after.passed, "{after:?}");
        assert_eq!(after.round1_all_metrics, report.round1_all_metrics);
        assert_eq!(after.round1.total_payload_bytes_read, 0);
        assert_eq!(after.round1.committed_entries, 0);
    }
}

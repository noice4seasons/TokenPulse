//! Offline Codex JSONL ingestion. Persist each scan batch atomically with its checkpoint.
pub mod accounting;
pub mod checkpoint;
pub mod global_state;
pub mod paths;
pub mod rate_limits;
pub mod scanner;

pub use accounting::{
    codex_usage_decision, CodexAccountingState, CodexUsageDecision, DecisionKind, Mode,
};
pub use checkpoint::{
    verify_full_prefix, verify_o1, AuditTarget, DualTrackCheckpoint, FileIdentity,
    FullVerifyWorker, Verification, VerifyOutcome,
};
pub use global_state::read_default_service_tier;
pub use paths::{discover_jsonl_files, validate_safe_local_path, CodexRoots, DiscoveredFile};
pub use rate_limits::{parse_rate_limits, RateLimits};
pub use scanner::{
    full_verify_due, scan_file, IdentityLink, IdentitySeed, ScanBatchOutput, ScanOptions,
};
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use std::{
        fs::{self, File, OpenOptions},
        io::{Seek, SeekFrom, Write},
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
    };
    use tokenpulse_core_domain::{
        models::{PricingPrecision, UnpricedReason},
        quota::{CreditState, QuotaPeriodKind, QuotaSourceKind},
    };

    struct Fixture {
        roots: CodexRoots,
        file: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let home = loop {
                let d = std::env::temp_dir().join(format!(
                    "tokenpulse-test-{}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                ));
                match fs::create_dir(&d) {
                    Ok(()) => break d,
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(e) => panic!("{e}"),
                }
            };
            let roots = CodexRoots::new(home);
            let dir = &roots.scan_roots()[0];
            fs::create_dir(dir).unwrap();
            let file = dir.join("a.jsonl");
            File::create(&file).unwrap();
            Self { roots, file }
        }
        fn append(&self, b: &[u8]) {
            OpenOptions::new()
                .append(true)
                .open(&self.file)
                .unwrap()
                .write_all(b)
                .unwrap();
        }
        fn scan(&self) -> ScanBatchOutput {
            scan_file(&self.roots, &self.file, &ScanOptions::new("s", 42)).unwrap()
        }
        fn resume(&self, b: &ScanBatchOutput) -> ScanBatchOutput {
            let mut o = ScanOptions::new("s", 43);
            o.checkpoint = Some(&b.next_checkpoint);
            o.resume_state_json = Some(&b.resume_state_json);
            scan_file(&self.roots, &self.file, &o).unwrap()
        }
        fn row(&self, v: Value) {
            self.append(v.to_string().as_bytes());
            self.append(b"\n");
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.roots.home);
        }
    }
    fn line(len: usize) -> Vec<u8> {
        let mut b = vec![b' '; len];
        b[0] = b'{';
        b[1] = b'}';
        b[len - 1] = b'\n';
        b
    }
    fn usage(n: u64) -> Value {
        json!({"input_tokens":n})
    }
    fn notice(total: u64, last: u64) -> Value {
        json!({"type":"event_msg","timestamp":"1970-01-01T00:00:01Z",
        "payload":{"type":"token_count","info":{"total_token_usage":usage(total),"last_token_usage":usage(last)}}})
    }
    fn detail(id: &str, n: u64) -> Value {
        json!({"type":"token_usage_record","payload":{"response_id":id,"usage":usage(n)}})
    }
    fn decide(state: &mut CodexAccountingState, v: Value) -> Option<CodexUsageDecision> {
        codex_usage_decision(state, &v, "s", "p", |_| None)
    }

    #[test]
    fn short_appends_hash_only_committed_bytes() {
        let f = Fixture::new();
        f.append(&line(500));
        let a = f.scan();
        assert_eq!(
            (
                a.next_checkpoint.head_len,
                a.next_checkpoint.tail_offset,
                a.next_checkpoint.tail_len
            ),
            (500, 0, 500)
        );
        let mut prev = a;
        for (append, total, read) in [
            (1500, 2000, 500),
            (4000, 6000, 2000),
            (8000, 14000, 6000),
            (300, 14300, 8192),
        ] {
            f.append(&line(append));
            let v = verify_o1(&mut File::open(&f.file).unwrap(), &prev.next_checkpoint).unwrap();
            assert_eq!(
                (v.outcome, v.validation_bytes_read),
                (VerifyOutcome::Match, read)
            );
            let next = f.resume(&prev);
            assert!(!next.rebased && next.complete_without_gaps);
            assert_eq!(
                (
                    next.next_checkpoint.byte_offset,
                    next.validation_bytes_read,
                    next.payload_bytes_read
                ),
                (total, read, append as u64)
            );
            assert_eq!(
                next.next_checkpoint.full_prefix_sha256,
                crate::checkpoint::digest(&fs::read(&f.file).unwrap())
            );
            prev = next;
        }
    }
    #[test]
    fn sha_continuation_every_block_boundary() {
        let f = Fixture::new();
        let mut prev = f.scan();
        for n in 3..=135 {
            f.append(&line(n));
            let b = f.resume(&prev);
            assert_eq!(
                b.next_checkpoint.full_prefix_sha256,
                crate::checkpoint::digest(&fs::read(&f.file).unwrap())
            );
            prev = b;
        }
    }
    #[test]
    fn middle_overwrite_and_scheduled_full_audit() {
        let f = Fixture::new();
        f.append(&line(14000));
        let old = f.scan();
        let mut edit = OpenOptions::new().write(true).open(&f.file).unwrap();
        edit.seek(SeekFrom::Start(7000)).unwrap();
        edit.write_all(b"\t").unwrap();
        drop(edit);
        let cp = &old.next_checkpoint;
        let mut file = File::open(&f.file).unwrap();
        assert_eq!(
            verify_o1(&mut file, cp).unwrap().outcome,
            VerifyOutcome::Match
        );
        let full = verify_full_prefix(&mut file, cp).unwrap();
        assert_eq!(full.validation_bytes_read, 14000);
        assert_eq!(full.outcome, VerifyOutcome::RebaseRequired);
        let mut opt = ScanOptions::new("s", 42 + scanner::FULL_VERIFY_INTERVAL_MS);
        opt.checkpoint = Some(cp);
        opt.resume_state_json = Some(&old.resume_state_json);
        let b = scan_file(&f.roots, &f.file, &opt).unwrap();
        assert!(b.rebased && b.full_rebuild_coverage());
        assert_eq!(b.next_checkpoint.generation, 1);
        assert_eq!(b.validation_bytes_read, 14000 + 8192);
    }
    #[test]
    fn truncation_and_edge_overwrite_rebase() {
        let f = Fixture::new();
        f.append(&line(100));
        let a = f.scan();
        fs::write(&f.file, line(50)).unwrap();
        let b = f.resume(&a);
        assert!(b.rebased);
        assert_eq!(b.validation_bytes_read, 0);
        let mut content = line(50);
        content[2] = b'\t';
        fs::write(&f.file, content).unwrap();
        let c = f.resume(&b);
        assert!(c.rebased);
        assert_eq!(c.next_checkpoint.generation, 2);
    }
    #[test]
    fn partial_line_and_corrupt_record() {
        let f = Fixture::new();
        f.row(detail("a", 5));
        let first_end = fs::metadata(&f.file).unwrap().len();
        let d = detail("b", 12).to_string();
        f.append(d.as_bytes());
        let first = f.scan();
        assert_eq!(first.entries.len(), 1);
        assert!(!first.complete_without_gaps);
        assert_eq!(first.next_checkpoint.byte_offset, first_end);
        assert_eq!(first.malformed_line_count, 0);
        f.append(b"\r\n");
        let next = f.resume(&first);
        assert_eq!(next.entries[0].tokens.input_tokens, 12);
        assert!(next.complete_without_gaps);
        assert!(!next.full_rebuild_coverage());
        f.append(b"private-invalid-body\n");
        f.row(detail("c", 7));
        let bad = f.scan();
        assert_eq!(bad.entries.len(), 3);
        assert_eq!(bad.malformed_line_count, 1);
        assert!(bad.has_coverage_gap && !bad.complete_without_gaps && !bad.full_rebuild_coverage());
        assert!(!format!("{bad:?}").contains("private-invalid-body"));
    }
    #[test]
    fn oversized_line_is_streamed_and_only_counted_after_lf() {
        let f = Fixture::new();
        f.append(&vec![b'x'; scanner::MAX_LINE_BYTES + 17]);
        let a = f.scan();
        assert_eq!(a.oversized_line_count, 0);
        assert_eq!(a.next_checkpoint.byte_offset, 0);
        f.append(b"\n");
        f.row(detail("after", 5));
        let b = f.resume(&a);
        assert_eq!(
            (
                b.oversized_line_count,
                b.malformed_line_count,
                b.entries.len()
            ),
            (1, 0, 1)
        );
        assert!(!b.complete_without_gaps);
    }
    #[test]
    fn limit_cancel_and_concurrent_append_preserve_frozen_end() {
        use std::cell::Cell;
        let f = Fixture::new();
        f.append(&line(60000));
        let mut opt = ScanOptions::new("s", 9);
        opt.payload_budget = Some(32768);
        let b = scan_file(&f.roots, &f.file, &opt).unwrap();
        assert_eq!(b.payload_bytes_read, 32768);
        assert_eq!(b.next_checkpoint.byte_offset, 0);
        assert!(!b.complete_without_gaps);
        opt.payload_budget = None;
        let count = Cell::new(0);
        let cancel = || {
            count.set(count.get() + 1);
            count.get() > 1
        };
        opt.is_cancelled = Some(&cancel);
        let b = scan_file(&f.roots, &f.file, &opt).unwrap();
        assert!(b.cancelled && !b.full_rebuild_coverage());
        let once = Cell::new(true);
        let append = || {
            if once.replace(false) {
                f.row(detail("new", 8));
            }
            false
        };
        opt.is_cancelled = Some(&append);
        let b = scan_file(&f.roots, &f.file, &opt).unwrap();
        assert_eq!(
            (b.frozen_end_offset, b.next_checkpoint.byte_offset),
            (60000, 60000)
        );
        assert!(b.entries.is_empty());
        assert_eq!(f.resume(&b).entries.len(), 1);
    }
    #[test]
    fn obsolete_resume_replays_protected_prefix_as_seeds() {
        let f = Fixture::new();
        f.row(detail("a", 5));
        let a = f.scan();
        f.row(detail("b", 9));
        let mut opt = ScanOptions::new("s", 50);
        opt.checkpoint = Some(&a.next_checkpoint);
        opt.resume_state_json = Some("{}");
        let b = scan_file(&f.roots, &f.file, &opt).unwrap();
        assert_eq!(b.entries.len(), 1);
        assert_eq!(b.entries[0].tokens.input_tokens, 9);
        assert!(b.identity_seeds.iter().any(|s| s.key == "codex:response:a"));
        assert!(!b.full_rebuild_coverage());
    }

    #[test]
    fn fork_inherits_baseline_until_child_turn_or_explicit_child_record() {
        let mut s = CodexAccountingState::new(0);
        decide(
            &mut s,
            json!({"type":"session_meta","payload":{"id":"child","timestamp":"2024-03-01T00:00:02.123Z","forked_from_id":"parent"}}),
        );
        decide(
            &mut s,
            json!({"type":"event_msg","payload":{"type":"task_started","turn_id":"old","started_at":1709251201}}),
        );
        assert!(s.forked && s.inherited);
        assert!(decide(&mut s, notice(100, 100)).is_none());
        assert_eq!(s.total, Some([100, 0, 0, 0]));
        let mut alternate = s.clone();
        decide(
            &mut s,
            json!({"type":"event_msg","payload":{"type":"task_started","turn_id":"child","started_at":1709251202}}),
        );
        let d = decide(&mut s, notice(105, 5)).unwrap();
        assert_eq!(d.usage, [5, 0, 0, 0]);
        assert!(!s.inherited);
        let mut child = detail("r", 7);
        child["timestamp"] = json!("2024-03-01T00:00:02.124Z");
        child["payload"]["thread_id"] = json!("child");
        child["payload"]["thread_token_usage"] = usage(107);
        assert_eq!(decide(&mut alternate, child).unwrap().usage, [7, 0, 0, 0]);
        assert!(!alternate.inherited);
    }
    #[test]
    fn aligned_dual_views_in_both_orders_and_unproven_offset() {
        let mut s = CodexAccountingState::new(0);
        let start = json!({"type":"event_msg","payload":{"type":"task_started","turn_id":"t"}});
        let mut d = detail("reply", 20);
        d["payload"]["turn_id"] = json!("t");
        d["payload"]["thread_token_usage"] = usage(100);
        d["payload"]["turn_token_usage"] = usage(20);
        for detail_first in [false, true] {
            decide(&mut s, start.clone());
            let (a, b) = if detail_first {
                (d.clone(), notice(20, 20))
            } else {
                (notice(20, 20), d.clone())
            };
            assert_eq!(decide(&mut s, a).unwrap().usage, [20, 0, 0, 0]);
            if detail_first {
                assert!(decide(&mut s, notice(19, 19)).is_none());
            }
            assert_eq!(decide(&mut s, b).unwrap().kind, DecisionKind::Alias);
            assert_eq!(s.notification_offset, Some([80, 0, 0, 0]));
            assert_eq!(decide(&mut s, notice(25, 5)).unwrap().usage, [5, 0, 0, 0]);
            s = CodexAccountingState::new(0);
        }
    }
    #[test]
    fn counter_keys_rollover_fresh_output_and_recent_bound() {
        let mut s = CodexAccountingState::new(0);
        let a = decide(&mut s, notice(10, 10)).unwrap();
        assert_eq!(
            a.identity_key,
            "codex:counter:f08325097417a37d46bc236a793060972971d333c1ea43a925c6da6790edca81"
        );
        assert_eq!(a.observation_key.as_deref(),Some("codex:observation:870ff5661a0e5583bad06f41640c7d3071fa2e1552dcd0b16eaaad5c680f235f"));
        decide(
            &mut s,
            json!({"type":"response_item","payload":{"type":"function_call"}}),
        );
        assert!(s.fresh_output);
        let b = decide(&mut s, notice(1, 1)).unwrap();
        assert_eq!(s.segment, 1);
        assert_eq!(b.usage, [1, 0, 0, 0]);
        for n in 2..=140 {
            decide(&mut s, notice(n, 1));
        }
        assert_eq!(s.recent.len(), 128);
        assert!(!s.recent.contains(&a.observation_key.unwrap()));
    }
    #[test]
    fn scanner_aliases_and_verified_long_context_pricing() {
        let f = Fixture::new();
        f.row(json!({"type":"turn_context","payload":{"model":"gpt-5.6-sol"}}));
        f.row(notice(10, 10));
        let mut detail = detail("r", 10);
        detail["timestamp"] = json!("1970-01-01T00:00:01Z");
        detail["payload"]["thread_token_usage"] = usage(10);
        f.row(detail);
        f.row(notice(300010, 3));
        let b = f.scan();
        assert_eq!(b.entries.len(), 2);
        assert!(b.entries[0].single_request_evidence);
        assert!(!b.entries[1].single_request_evidence);
        assert!(b
            .identity_links
            .iter()
            .any(|l| l.alias == "codex:response:r"));
        assert_eq!(
            b.entries[1].pricing_precision,
            PricingPrecision::Unpriced {
                reason: UnpricedReason::MissingSingleRequestEvidenceForLongContext
            }
        );
        assert_eq!(b.entries[1].cost_micro_usd, None);
    }
    #[test]
    fn copied_response_and_duplicate_single_do_not_charge() {
        let mut s = CodexAccountingState::new(0);
        let d = detail("r", 17);
        assert!(decide(&mut s, d.clone()).unwrap().single_request_evidence);
        assert!(decide(&mut s, d.clone()).is_none());
        let mut replica = CodexAccountingState::new(0);
        assert!(
            codex_usage_decision(&mut replica, &d, "copy", "position", |k| (k
                == "codex:response:r")
                .then(|| "original".into()))
            .is_none()
        );
    }

    fn rates(contents: &str, stamp: &str) -> RateLimits {
        let line = format!(r#"{{"timestamp":"{stamp}","payload":{{"rate_limits":{contents}}}}}"#);
        parse_rate_limits(line.as_bytes(), "session", 77).unwrap()
    }
    #[test]
    fn precise_credit_states_and_original_decimal_text() {
        for (fields, expect) in [
            (r#"{}"#, CreditState::Unknown),
            (
                r#"{"credits":{"has_credits":"true","unlimited":false}}"#,
                CreditState::Unknown,
            ),
            (r#"{"credits":{"has_credits":false}}"#, CreditState::None),
            (
                r#"{"credits":{"has_credits":true,"unlimited":true}}"#,
                CreditState::Unlimited,
            ),
            (r#"{"credits":{"has_credits":true}}"#, CreditState::Unknown),
            (
                r#"{"credits":{"has_credits":true,"unlimited":false}}"#,
                CreditState::Limited { balance_text: None },
            ),
            (
                r#"{"credits":{"has_credits":true,"unlimited":false,"balance":"001.2300"}}"#,
                CreditState::Limited {
                    balance_text: Some("001.2300".into()),
                },
            ),
            (
                r#"{"credits":{"has_credits":true,"unlimited":false,"balance":12345678901234567890.0000900E-2}}"#,
                CreditState::Limited {
                    balance_text: Some("12345678901234567890.0000900E-2".into()),
                },
            ),
        ] {
            let r = rates(fields, "bad");
            assert!(r.quota_observations.is_empty());
            let c = &r.credit_observations[0];
            assert_eq!(c.state, expect);
            assert!(!c.account_confirmed);
            assert_eq!(c.event_time_ms, None);
        }
    }
    #[test]
    fn quota_selection_uses_event_time_and_preserves_unknown_period() {
        let late =
            r#"{"primary":{"window_minutes":1440,"resets_at":1700000000},"plan_type":" Pro "}"#;
        let early = r#"{"primary":{"used_percent":10},"secondary":{"window_minutes":10080,"used_percent":100,"resets_at":1700000000000}}"#;
        for rev in [false, true] {
            let a = rates(early, "2025-02-28T00:00:00Z");
            let b = rates(late, "2025-03-01T00:00:00Z");
            let (mut s, t) = if rev { (b, a) } else { (a, b) };
            s.merge(t);
            let q = s
                .quota_observations
                .iter()
                .find(|q| q.target_id == "primary")
                .unwrap();
            assert_eq!(q.period_kind, QuotaPeriodKind::Unknown(Some(1440)));
            assert_eq!(q.used_pct, None);
            assert_eq!(q.resets_at_ms, Some(1700000000000));
            assert!(!q.account_confirmed && q.event_time_trusted);
            assert_eq!(q.source_kind, QuotaSourceKind::LocalLogSession);
            let w = s
                .quota_observations
                .iter()
                .find(|q| q.target_id == "secondary")
                .unwrap();
            assert_eq!(
                (w.period_kind, w.used_pct, w.resets_at_ms),
                (QuotaPeriodKind::Weekly, Some(100.), q.resets_at_ms)
            );
            assert_eq!(s.credit_observations[0].plan_type.as_deref(), Some(" Pro "));
        }
        for v in ["null", r#""10""#, "-1", "100.1", "1e400"] {
            let r = rates(&format!(r#"{{"primary":{{"used_percent":{v}}}}}"#), "");
            assert_eq!(r.quota_observations[0].used_pct, None);
        }
    }
    #[test]
    fn utc_dates_and_input_counters() {
        assert_eq!(
            accounting::timestamp_ms("2024-02-29T23:59:59.15199-02:30"),
            accounting::timestamp_ms("2024-03-01T02:29:59.151Z")
        );
        assert_eq!(accounting::timestamp_ms("2023-02-29T00:00:00Z"), None);
        assert_eq!(
            accounting::usage_vector(
                &json!({"input_tokens":2,"cached_input_tokens":15,"output_tokens":3,"reasoning_output_tokens":4})
            ),
            Some([2, 2, 3, 3])
        );
        for n in [
            json!(-1),
            json!(0.5),
            json!("12"),
            json!(9007199254740992u64),
        ] {
            assert_eq!(accounting::usage_vector(&json!({"input_tokens":n})), None);
        }
    }
    #[test]
    fn forbidden_names_unc_and_escape_are_rejected_before_stat() {
        let f = Fixture::new();
        let root = &f.roots.home;
        for p in [
            root.join("AuTh.JsOn"),
            root.join("AUTH.JSON").join("child"),
            root.join("../outside"),
            PathBuf::from(r"\\server\share\x.jsonl"),
            PathBuf::from(r"\\?\UNC\server\share\x.jsonl"),
        ] {
            assert_eq!(
                validate_safe_local_path(root, &p).unwrap_err().kind(),
                std::io::ErrorKind::InvalidInput
            );
        }
        #[cfg(windows)]
        for name in ["auth.json.", "Auth.json:stream", "nul.jsonl", "LPT1.jsonl"] {
            assert_eq!(
                validate_safe_local_path(root, &root.join(name))
                    .unwrap_err()
                    .kind(),
                std::io::ErrorKind::InvalidInput
            );
        }
    }
    #[test]
    fn discovery_archive_rename_and_global_state_cap() {
        let mut f = Fixture::new();
        f.row(detail("r", 8));
        let old = f.scan();
        let roots = f.roots.scan_roots();
        fs::create_dir(&roots[1]).unwrap();
        fs::rename(&f.file, roots[1].join("z.jsonl")).unwrap();
        f.file = roots[1].join("z.jsonl");
        let resumed = f.resume(&old);
        assert!(!resumed.rebased);
        assert_eq!(
            old.next_checkpoint.file_identity,
            resumed.next_checkpoint.file_identity
        );
        fs::create_dir_all(roots[2].join("nested")).unwrap();
        fs::write(roots[2].join("nested/B.JSONL"), b"{}\n").unwrap();
        fs::write(roots[0].join("ignore.txt"), b"").unwrap();
        let found = discover_jsonl_files(&f.roots).unwrap();
        assert_eq!(found.len(), 2);
        assert!(found[0].path < found[1].path);
        let moved = found.iter().find(|x| x.path == f.file).unwrap();
        assert_eq!(
            Some(&moved.identity),
            old.next_checkpoint.file_identity.as_ref()
        );
        assert_eq!(moved.size, old.next_checkpoint.byte_offset);
        assert_eq!(read_default_service_tier(&f.roots), None);
        let p = f.roots.global_state_path();
        fs::write(
            &p,
            br#"{"electron-persisted-atom-state":{"default-service-tier":"  priority "}}"#,
        )
        .unwrap();
        assert_eq!(
            read_default_service_tier(&f.roots).as_deref(),
            Some("  priority ")
        );
        for bad in [
            &b"{"[..],
            br#"{"electron-persisted-atom-state":{"default-service-tier":1}}"#,
        ] {
            fs::write(&p, bad).unwrap();
            assert_eq!(read_default_service_tier(&f.roots), None);
        }
        File::create(p)
            .unwrap()
            .set_len(global_state::MAX_GLOBAL_STATE_BYTES + 1)
            .unwrap();
        assert_eq!(read_default_service_tier(&f.roots), None);
    }
    #[test]
    fn rejects_link_in_directory_chain() {
        let f = Fixture::new();
        let outside = f.roots.home.join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("a.jsonl"), b"{}\n").unwrap();
        let link = f.roots.scan_roots()[0].join("escape");
        #[cfg(windows)]
        {
            // Junction creation does not require symbolic-link privileges.
            let output = std::process::Command::new("cmd")
                .args(["/d", "/c", "mklink", "/j"])
                .current_dir(&f.roots.home)
                .args([r"sessions\escape", "outside"])
                .output()
                .unwrap();
            assert!(output.status.success());
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        assert!(validate_safe_local_path(&f.roots.home, &link.join(Path::new("a.jsonl"))).is_err());
        assert_eq!(discover_jsonl_files(&f.roots).unwrap().len(), 1);
    }

    #[test]
    fn window_only_event_does_not_advance_credit_timestamp() {
        for reverse in [false, true] {
            let credit = rates(
                r#"{"credits":{"has_credits":false}}"#,
                "2026-01-01T00:00:00Z",
            );
            let window = rates(
                r#"{"primary":{"window_minutes":300,"used_percent":0}}"#,
                "2026-06-01T00:00:00Z",
            );
            let (mut snapshot, next) = if reverse {
                (window, credit)
            } else {
                (credit, window)
            };
            snapshot.merge(next);
            assert_eq!(snapshot.credit_observations[0].state, CreditState::None);
            assert_eq!(
                snapshot.credit_observations[0].event_time_ms,
                accounting::timestamp_ms("2026-01-01T00:00:00Z")
            );
            assert_eq!(
                snapshot.quota_observations[0].period_kind,
                QuotaPeriodKind::FiveHour
            );
        }
    }
    #[test]
    fn idle_worker_checks_middle_overwrite_without_a_foreground_scan() {
        use std::{sync::mpsc, time::Duration};
        let f = Fixture::new();
        f.append(&line(16000));
        let cp = f.scan().next_checkpoint;
        let mut edit = OpenOptions::new().write(true).open(&f.file).unwrap();
        edit.seek(SeekFrom::Start(8000)).unwrap();
        edit.write_all(b"\t").unwrap();
        drop(edit);
        let target = AuditTarget {
            path: f.file.clone(),
            checkpoint: cp,
        };
        let (tx, rx) = mpsc::channel();
        let worker = FullVerifyWorker::start(
            f.roots.clone(),
            Duration::from_secs(60),
            move || vec![target.clone()],
            move |target, now, result| {
                tx.send((target.checkpoint.byte_offset, now, result.unwrap()))
                    .unwrap();
            },
        )
        .unwrap();
        let (offset, verified_at, report) = rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(offset, 16000);
        assert!(verified_at > 42);
        assert_eq!(report.outcome, VerifyOutcome::RebaseRequired);
        assert_eq!(report.validation_bytes_read, offset);
        drop(worker);
    }
}

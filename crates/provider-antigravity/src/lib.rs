//! Read-only Antigravity usage collection. Keep one scanner per source; pass the
//! last transactionally committed checkpoint, not an uncommitted batch. Schedule
//! `VerificationMode::Full` periodically, including for sources with no new data.
pub mod checkpoint;
pub mod models;
pub mod paths;
pub mod scanner;

pub use checkpoint::{
    verify_full_prefix, verify_o1, DualTrackCheckpoint, FileIdentity, Verification, VerifyOutcome,
};
pub use models::normalize_antigravity_model;
pub use paths::{
    discover_transcript_files, validate_safe_local_path, AntigravityRoots, DiscoveredTranscript,
};
pub use scanner::{AntigravityScanBatch, AntigravityScanner, VerificationMode, MAX_LINE_BYTES};

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs::{self, File, OpenOptions},
        io::{Seek, SeekFrom, Write},
        path::{Path, PathBuf},
    };
    use tempfile::TempDir;
    use tokenpulse_core_domain::models::{
        PricingPrecision, ProviderId, TokenVector, UnpricedReason,
    };
    const ROW: &str = r#"{"type":"PLANNER_RESPONSE","status":"DONE","step_index":17,"model":"gemini-4-neon-high","created_at":"1970-01-01T01:00:00.123456+01:00","input_tokens":7997,"cache_read_tokens":16087,"output_tokens":556}"#;

    struct Fixture {
        _dir: TempDir,
        roots: AntigravityRoots,
        path: PathBuf,
        scanner: AntigravityScanner,
    }
    impl Fixture {
        fn new(data: &[u8]) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let roots = AntigravityRoots::new(dir.path().join("brain"));
            let path = roots
                .brain_dir
                .join("conv/.system_generated/logs/transcript.jsonl");
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, data).unwrap();
            Self {
                _dir: dir,
                roots,
                path,
                scanner: AntigravityScanner::new(),
            }
        }
        fn append(&self, data: &[u8]) {
            OpenOptions::new()
                .append(true)
                .open(&self.path)
                .unwrap()
                .write_all(data)
                .unwrap();
        }
        fn scan(
            &mut self,
            cp: Option<&DualTrackCheckpoint>,
            mode: VerificationMode,
        ) -> AntigravityScanBatch {
            self.scanner
                .scan(&self.roots, &self.path, "source", cp, mode, 321)
                .unwrap()
        }
    }
    use VerificationMode::{Full, Tail};

    #[test]
    fn disjoint_counts_pricing_and_identity() {
        let row = format!("{ROW}\r\n");
        let mut fixture = Fixture::new(row.as_bytes());
        let batch = fixture.scan(None, Tail);
        assert!(batch.complete_without_gaps);
        assert_eq!(batch.entries.len(), 1);
        let entry = &batch.entries[0];
        assert_eq!(
            entry.tokens,
            TokenVector {
                input_tokens: 24084,
                non_cached_input_tokens: 7997,
                cache_read_tokens: 16087,
                output_tokens: 556,
                ..TokenVector::default()
            }
        );
        assert!((entry.tokens.cache_hit_ratio().unwrap() * 100.0 - 66.8).abs() < 0.01);
        assert_eq!(entry.request_id.as_deref(), Some("ag:step:conv:17"));
        assert_eq!(entry.identity_key, "ag:step:conv:17");
        assert_eq!(entry.identity_origin, "antigravity:conv:conv");
        assert_eq!(entry.timestamp_ms, 123);
        assert!(entry.single_request_evidence);
        assert_eq!(entry.provider, ProviderId::Antigravity);
        assert_eq!(
            entry.pricing_precision,
            PricingPrecision::Unpriced {
                reason: UnpricedReason::UnknownModel
            }
        );
        assert!(entry.cost_micro_usd.is_none());
        assert_eq!(entry.display_model, "Gemini 4 Neon (High)");
        assert_eq!(
            batch.next_checkpoint.full_prefix_sha256,
            checkpoint::digest(row.as_bytes())
        );
    }

    #[test]
    fn model_names_preserve_unknown_verbatim() {
        for (raw, display) in [
            ("", "Unknown (antigravity)"),
            ("gemini-3.8-flash", "Gemini 3.8 Flash"),
            ("claude-opus-5-5-high", "Claude Opus 5.5 (High)"),
            (" My-opus-lab ", " My-opus-lab "),
        ] {
            assert_eq!(normalize_antigravity_model(raw), display);
        }
    }

    #[test]
    fn selective_extraction_never_emits_bodies() {
        let sentinel = "PRIVATE_BODY_SENTINEL";
        let mut row: serde_json::Value = serde_json::from_str(ROW).unwrap();
        row["content"] = sentinel.repeat(8192).into();
        row["thinking"] = sentinel.repeat(8192).into();
        row["tool_calls"] = serde_json::json!([{"args":sentinel}]);
        let text = format!(
            "{}\n{}\n{}\n{}\n",
            ROW.replace("PLANNER_RESPONSE", "TOOL_CALL"),
            ROW.replace("DONE", "PENDING"),
            r#"{"type":"PLANNER_RESPONSE","status":"DONE","output_tokens":0}"#,
            row
        );
        let mut fixture = Fixture::new(text.as_bytes());
        let batch = fixture.scan(None, Tail);
        assert!(batch.complete_without_gaps);
        assert_eq!(batch.entries.len(), 1);
        assert!(!serde_json::to_string(&batch.entries)
            .unwrap()
            .contains(sentinel));
        assert!(!serde_json::to_string(&batch.next_checkpoint)
            .unwrap()
            .contains(sentinel));
        assert_eq!(batch.resume_state_json, "{}");
        assert_eq!(fs::read_to_string(&fixture.path).unwrap(), text);
        assert_eq!(
            fs::read_dir(fixture.path.parent().unwrap())
                .unwrap()
                .count(),
            1
        );
    }

    #[test]
    fn short_growth_uses_only_old_bytes() {
        let mut fixture = Fixture::new(b"{}\n");
        let old = fixture.scan(None, Tail).next_checkpoint;
        let data = format!("{ROW}\n");
        fixture.append(data.as_bytes());
        assert_eq!(
            verify_o1(&mut File::open(&fixture.path).unwrap(), &old)
                .unwrap()
                .validation_bytes_read,
            3
        );
        let batch = fixture.scan(Some(&old), Tail);
        assert!(!batch.rebased);
        assert_eq!(batch.validation_bytes_read, 3);
        assert_eq!(batch.scanned_from_offset, 3);
        assert_eq!(batch.payload_bytes_read, data.len() as u64);
        assert_eq!(batch.entries.len(), 1);
        assert_eq!(
            batch.next_checkpoint.full_prefix_sha256,
            checkpoint::digest(format!("{{}}\n{data}").as_bytes())
        );
    }

    #[test]
    fn warm_tail_and_cold_audit_preserve_sha256_at_window_edges() {
        for size in [0, 1, 63, 64, 65, 4095, 4096, 4097, 8191, 8192, 8193, 12345] {
            let mut data = vec![b' '; size];
            if size != 0 {
                data[size - 1] = b'\n';
            }
            let mut fixture = Fixture::new(&data);
            let old = fixture.scan(None, Tail).next_checkpoint;
            let row = format!("{ROW}\n");
            fixture.append(row.as_bytes());
            data.extend_from_slice(row.as_bytes());
            let batch = fixture.scan(Some(&old), Tail);
            assert!(!batch.rebased);
            assert_eq!(batch.validation_bytes_read, size.min(8192) as u64);
            assert_eq!(
                batch.next_checkpoint.full_prefix_sha256,
                checkpoint::digest(&data)
            );
            fixture.scanner = AntigravityScanner::new();
            let cold = fixture.scan(Some(&old), Tail);
            assert_eq!(
                cold.next_checkpoint.full_prefix_sha256,
                batch.next_checkpoint.full_prefix_sha256
            );
            assert_eq!(
                cold.validation_bytes_read,
                size.min(8192) as u64 + if size > 8192 { size as u64 } else { 0 }
            );
        }
    }

    #[test]
    fn full_verify_rebases_middle_rewrite_even_at_idle() {
        let mut text = format!("{{\"content\":\"{}\"}}\n{ROW}\n", "a".repeat(16000)).into_bytes();
        let mut fixture = Fixture::new(&text);
        let old = fixture.scan(None, Tail).next_checkpoint;
        let mut writer = OpenOptions::new().write(true).open(&fixture.path).unwrap();
        writer.seek(SeekFrom::Start(6500)).unwrap();
        writer.write_all(b"b").unwrap();
        let mut file = File::open(&fixture.path).unwrap();
        assert_eq!(
            verify_o1(&mut file, &old).unwrap().outcome,
            VerifyOutcome::Match
        );
        assert_eq!(
            verify_full_prefix(&mut file, &old).unwrap().outcome,
            VerifyOutcome::RebaseRequired
        );
        let batch = fixture.scan(Some(&old), Full);
        assert!(batch.rebased);
        assert_eq!(batch.scanned_from_offset, 0);
        assert_eq!(batch.entries.len(), 1);
        assert_eq!(batch.next_checkpoint.generation, old.generation + 1);
        text[6500] = b'b';
        assert_eq!(
            batch.next_checkpoint.full_prefix_sha256,
            checkpoint::digest(&text)
        );
    }

    #[test]
    fn incomplete_line_and_malformed_recovery() {
        let row = ROW.replace("\"step_index\":17,", "");
        let mut fixture = Fixture::new(b"{BAD}\n");
        fixture.append(row.as_bytes());
        let batch = fixture.scan(None, Tail);
        assert_eq!(batch.malformed_line_count, 1);
        assert!(batch.has_coverage_gap);
        assert!(!batch.complete_without_gaps);
        assert_eq!(batch.next_checkpoint.byte_offset, 6);
        assert!(batch.entries.is_empty());
        let partial = fixture.scan(Some(&batch.next_checkpoint), Tail);
        assert!(!partial.complete_without_gaps);
        assert!(!partial.has_coverage_gap);
        assert_eq!(partial.next_checkpoint.byte_offset, 6);
        fixture.append(b"\n");
        let done = fixture.scan(Some(&partial.next_checkpoint), Tail);
        assert!(done.complete_without_gaps);
        assert_eq!(done.entries.len(), 1);
        assert_eq!(
            done.entries[0].identity_key,
            format!("ag:pos:source:0:{}", done.next_checkpoint.byte_offset)
        );
        fs::write(&fixture.path, b"{}\n").unwrap();
        let truncated = fixture.scan(Some(&done.next_checkpoint), Tail);
        assert!(truncated.rebased);
        assert_eq!(truncated.next_checkpoint.byte_offset, 3);
    }

    #[test]
    fn oversized_lines_drain_to_lf_without_swallowing_next_record() {
        let mut fixture = Fixture::new(b"{\"content\":\"");
        fixture.append(&vec![b'a'; MAX_LINE_BYTES]);
        let partial = fixture.scan(None, Tail);
        assert_eq!(partial.next_checkpoint.byte_offset, 0);
        assert!(!partial.complete_without_gaps);
        assert!(partial.has_coverage_gap);
        assert_eq!(partial.oversized_line_count, 1);
        fixture.append(format!("\"}}\n{ROW}\n").as_bytes());
        let batch = fixture.scan(Some(&partial.next_checkpoint), Tail);
        assert_eq!(batch.oversized_line_count, 1);
        assert_eq!(batch.malformed_line_count, 0);
        assert!(batch.has_coverage_gap);
        assert_eq!(batch.entries.len(), 1);
    }

    #[test]
    fn invalid_utf8_inside_ignored_field_is_still_malformed() {
        let mut fixture = Fixture::new(b"{\"content\":\"\xff\"}\n");
        let batch = fixture.scan(None, Tail);
        assert!(batch.has_coverage_gap);
        assert_eq!(batch.malformed_line_count, 1);
    }

    #[test]
    fn only_exact_local_log_layout_is_opened() {
        let fixture = Fixture::new(b"\n");
        let root = &fixture.roots.brain_dir;
        for name in [
            "auth.json",
            "state.vscdb",
            "hub_secret.json",
            "transcript_full.jsonl",
            "other.jsonl",
            "transcript.jsonl:secret",
        ] {
            let path = fixture.path.with_file_name(name);
            assert_eq!(
                validate_safe_local_path(root, &path).unwrap_err().kind(),
                std::io::ErrorKind::InvalidInput
            );
        }
        for remote in [
            r"\\server\share\conv\.system_generated\logs\transcript.jsonl",
            r"\\?\UNC\server\share\logs\transcript.jsonl",
            "//server/share/logs/transcript.jsonl",
        ] {
            assert_eq!(
                validate_safe_local_path(root, Path::new(remote))
                    .unwrap_err()
                    .kind(),
                std::io::ErrorKind::InvalidInput
            );
        }
        fs::write(fixture.path.with_file_name("transcript_full.jsonl"), b"{}").unwrap();
        let other = root.join("other/.system_generated/logs");
        fs::create_dir_all(&other).unwrap();
        fs::write(other.join("transcript_full.jsonl"), b"{}").unwrap();
        let files = discover_transcript_files(&fixture.roots).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].conversation_id, "conv");
        assert_eq!(files[0].size, 1);
        assert!(files[0].identity.file_index.is_some());
    }

    #[test]
    fn hard_link_alias_is_not_ingested() {
        let mut fixture = Fixture::new(b"abc");
        fs::hard_link(&fixture.path, fixture.path.with_file_name("auth.json")).unwrap();
        assert!(discover_transcript_files(&fixture.roots)
            .unwrap()
            .is_empty());
        assert!(fixture
            .scanner
            .scan(&fixture.roots, &fixture.path, "source", None, Tail, 0)
            .is_err());
    }

    #[test]
    fn reject_redirected_directory_and_leaf() {
        let fixture = Fixture::new(b"{}\n");
        #[cfg(unix)]
        use std::os::unix::fs::{symlink as symlink_dir, symlink as symlink_file};
        #[cfg(windows)]
        use std::os::windows::fs::{symlink_dir, symlink_file};
        let alias = fixture.roots.brain_dir.join("alias");
        let result = symlink_dir(fixture.roots.brain_dir.join("conv"), &alias);
        #[cfg(windows)]
        if result
            .as_ref()
            .is_err_and(|err| err.raw_os_error() == Some(1314))
        {
            return;
        }
        result.unwrap();
        assert!(validate_safe_local_path(
            &fixture.roots.brain_dir,
            &alias.join(".system_generated/logs/transcript.jsonl")
        )
        .is_err());
        let target = fixture.path.with_file_name("other.jsonl");
        fs::rename(&fixture.path, &target).unwrap();
        symlink_file(target, &fixture.path).unwrap();
        assert!(validate_safe_local_path(&fixture.roots.brain_dir, &fixture.path).is_err());
    }
}

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config_store;
mod desktop;
mod export_dialog;
mod pipeline;
mod single_instance;

use std::{
    env, fs, io,
    path::{Path, PathBuf},
    process::ExitCode,
};
use tokenpulse_provider_antigravity::AntigravityRoots;
use tokenpulse_provider_codex::CodexRoots;

const USAGE: &str = "tokenpulse [--verify-codex | --verify-all | --bench-desktop-ms <ms> [--bench-tray-only]] [--codex-home <dir>] [--antigravity-brain <dir>] [--db <path>]";

fn db_files(path: &Path) -> [PathBuf; 3] {
    ["", "-wal", "-shm"].map(|suffix| {
        let mut name = path.as_os_str().to_owned();
        name.push(suffix);
        PathBuf::from(name)
    })
}

fn run() -> Result<bool, String> {
    let raw_args: Vec<_> = env::args_os().skip(1).collect();
    if raw_args.is_empty() {
        desktop::run_desktop()?;
        return Ok(true);
    }
    let mut args = raw_args.into_iter();
    let mut verify = false;
    let mut verify_all = false;
    let mut bench_ms: Option<u64> = None;
    let mut bench_tray_only = false;
    let mut home = None;
    let mut ag_brain = None;
    let mut db = None;
    while let Some(flag) = args.next() {
        match flag.to_str() {
            Some("--verify-codex") if !verify && !verify_all && bench_ms.is_none() => verify = true,
            Some("--verify-all") if !verify && !verify_all && bench_ms.is_none() => {
                verify_all = true
            }
            Some("--bench-desktop-ms") if !verify && !verify_all && bench_ms.is_none() => {
                let raw = args
                    .next()
                    .and_then(|s| s.to_str().map(str::to_owned))
                    .ok_or(USAGE)?;
                bench_ms = Some(raw.parse::<u64>().map_err(|_| USAGE)?);
            }
            Some("--bench-tray-only") if !bench_tray_only => bench_tray_only = true,
            Some("--codex-home" | "--antigravity-brain" | "--db") => {
                let slot = if flag == "--db" {
                    &mut db
                } else if flag == "--antigravity-brain" {
                    &mut ag_brain
                } else {
                    &mut home
                };
                let value = args
                    .next()
                    .filter(|s| !s.is_empty() && !s.to_string_lossy().starts_with("--"))
                    .ok_or(USAGE)?;
                if slot.replace(PathBuf::from(value)).is_some() {
                    return Err(USAGE.into());
                }
            }
            _ => return Err(USAGE.into()),
        }
    }
    if let Some(ms) = bench_ms {
        desktop::run_bench(bench_tray_only, ms)?;
        return Ok(true);
    }
    if !verify && !verify_all {
        return Err(USAGE.into());
    }
    let roots = match home {
        Some(dir) => CodexRoots::new(env::current_dir().map_err(|e| e.to_string())?.join(dir)),
        None => CodexRoots::from_env().map_err(|e| e.to_string())?,
    };
    let temporary = db.is_none();
    let path = db.unwrap_or_else(|| {
        env::temp_dir().join(format!("tokenpulse-verify-{}.db", std::process::id()))
    });
    let files = db_files(&path);
    if temporary {
        // Refuse an earlier process's files after PID reuse.
        for file in &files[1..] {
            match fs::symlink_metadata(file) {
                Err(e) if e.kind() == io::ErrorKind::NotFound => (),
                _ => {
                    return Err(
                        "temporary database sidecar already exists or is inaccessible".into(),
                    )
                }
            }
        }
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| e.to_string())?;
    }
    let report = pipeline::run_verification(&roots, &path);
    let ag_section = if verify_all && report.is_ok() {
        let ag_roots = match ag_brain {
            Some(dir) => {
                AntigravityRoots::new(env::current_dir().map_err(|e| e.to_string())?.join(dir))
            }
            None => AntigravityRoots::from_env().map_err(|e| e.to_string())?,
        };
        Some(pipeline::run_antigravity_verification(&ag_roots, &path))
    } else {
        None
    };
    if temporary {
        let mut cleanup_error = None;
        for file in files.iter().rev() {
            if let Err(e) = fs::remove_file(file) {
                if e.kind() != io::ErrorKind::NotFound {
                    cleanup_error = Some(e);
                }
            }
        }
        if let Some(e) = cleanup_error {
            return Err(format!("temporary database cleanup failed: {e}"));
        }
    }
    let report = report?;
    if !report.report_privacy_passed {
        println!("{{\n  \"passed\": false,\n  \"report_privacy_passed\": false\n}}");
        return Ok(false);
    }
    if let Some(ag_res) = ag_section {
        let ag = ag_res?;
        let ag_ok = ag.incremental_validation_within_limit
            && ag.incremental_no_inserts
            && ag.metrics_consistent_across_rounds;
        let combined = serde_json::json!({
            "passed": report.passed && ag_ok,
            "codex": report,
            "antigravity": ag,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&combined).map_err(|e| e.to_string())?
        );
        return Ok(report.passed && ag_ok);
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?
    );
    Ok(report.passed)
}

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

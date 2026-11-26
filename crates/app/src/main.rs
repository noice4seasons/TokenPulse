mod pipeline;

use std::{
    env, fs, io,
    path::{Path, PathBuf},
    process::ExitCode,
};
use tokenpulse_provider_codex::CodexRoots;

const USAGE: &str = "tokenpulse --verify-codex [--codex-home <dir>] [--db <path>]";

fn db_files(path: &Path) -> [PathBuf; 3] {
    ["", "-wal", "-shm"].map(|suffix| {
        let mut name = path.as_os_str().to_owned();
        name.push(suffix);
        PathBuf::from(name)
    })
}

fn run() -> Result<bool, String> {
    let mut args = env::args_os().skip(1);
    let mut verify = false;
    let mut home = None;
    let mut db = None;
    while let Some(flag) = args.next() {
        match flag.to_str() {
            Some("--verify-codex") if !verify => verify = true,
            Some("--codex-home" | "--db") => {
                let slot = if flag == "--db" { &mut db } else { &mut home };
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
    if !verify {
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

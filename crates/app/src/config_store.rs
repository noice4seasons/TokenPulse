use serde::{Deserialize, Serialize};
use std::{
    env, fs,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
use tokenpulse_core_domain::config::AppConfig;

pub struct AppPaths {
    #[allow(dead_code)]
    pub data_dir: PathBuf,
    pub config_path: PathBuf,
    pub db_path: PathBuf,
    pub webview_udf_dir: PathBuf,
}

impl AppPaths {
    pub fn discover() -> Result<Self, String> {
        let base = env::var_os("LOCALAPPDATA")
            .filter(|p| !p.is_empty())
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .ok_or("LOCALAPPDATA must be a nonempty absolute path")?;
        let data_dir = base.join("TokenPulse");
        let webview_udf_dir = data_dir.join("webview2");
        fs::create_dir_all(&webview_udf_dir).map_err(|e| e.to_string())?;
        Ok(Self {
            config_path: data_dir.join("config.json"),
            db_path: data_dir.join("usage.db"),
            data_dir,
            webview_udf_dir,
        })
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PersistedState {
    pub config: AppConfig,
    pub overlay_visible: bool,
    pub overlay_compact: bool,
    pub overlay_pos: Option<(i32, i32)>,
}

impl Default for PersistedState {
    fn default() -> Self {
        Self {
            config: AppConfig::default(),
            overlay_visible: true,
            overlay_compact: true,
            overlay_pos: None,
        }
    }
}

pub fn load_state(path: &Path) -> PersistedState {
    let mut state: PersistedState = fs::read(path)
        .ok()
        .and_then(|data| serde_json::from_slice(&data).ok())
        .unwrap_or_default();
    state.config = state.config.sanitized();
    state
}

// std::fs::rename on Windows uses MoveFileExW with MOVEFILE_REPLACE_EXISTING.
// Keep both names on the same volume, close the flushed file before replacing,
// and never unlink the destination, even when another process has it open.
pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let parent = path.parent().ok_or(io::ErrorKind::InvalidInput)?;
    let suffix = if path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("tmp"))
    {
        "part"
    } else {
        "tmp"
    };
    let (temp, mut file) = loop {
        let seq = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let candidate = parent.join(format!(".tokenpulse-{}-{seq}.{suffix}", std::process::id()));
        match fs::File::create_new(&candidate) {
            Ok(file) => break (candidate, file),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    };
    let flushed = file.write_all(bytes).and_then(|()| file.sync_all());
    drop(file);
    let result = flushed.and_then(|()| fs::rename(&temp, path));
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

pub fn save_state(path: &Path, state: &PersistedState) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(state).map_err(|e| e.to_string())?;
    atomic_write(path, &bytes).map_err(|e| e.to_string())
}

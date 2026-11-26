use crate::paths::{open_file, CodexRoots};
use serde_json::Value;
use std::io::Read;

pub const MAX_GLOBAL_STATE_BYTES: u64 = 8 * 1024 * 1024;

pub fn read_default_service_tier(roots: &CodexRoots) -> Option<String> {
    let file = open_file(&roots.home, &roots.global_state_path()).ok()?;
    if file.metadata().ok()?.len() > MAX_GLOBAL_STATE_BYTES {
        return None;
    }
    let mut data = Vec::new();
    file.take(MAX_GLOBAL_STATE_BYTES + 1)
        .read_to_end(&mut data)
        .ok()?;
    if data.len() as u64 > MAX_GLOBAL_STATE_BYTES {
        return None;
    }
    let json: Value = serde_json::from_slice(&data).ok()?;
    json["electron-persisted-atom-state"]["default-service-tier"]
        .as_str()
        .map(str::to_owned)
}

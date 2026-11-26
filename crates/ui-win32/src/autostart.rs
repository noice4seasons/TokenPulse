use std::path::Path;
use windows::{
    core::w,
    Win32::{Foundation::*, System::Registry::*},
};
const RUN: windows::core::PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const NAME: windows::core::PCWSTR = w!("TokenPulse");

pub fn format_run_value(exe_path: &Path) -> Result<String, String> {
    let path = exe_path.to_str().ok_or("Executable path is not Unicode")?;
    if path.trim().is_empty()
        || path.ends_with(['\\', '/'])
        || path.chars().any(|ch| ch.is_control() || ch == '"')
    {
        return Err("Empty executable path or invalid character".into());
    }
    Ok(format!("\"{path}\""))
}
pub fn is_enabled() -> Result<bool, String> {
    let mut bytes = 0;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            RUN,
            NAME,
            RRF_RT_REG_SZ,
            None,
            None,
            Some(&mut bytes),
        )
    };
    match status {
        ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND | ERROR_UNSUPPORTED_TYPE => Ok(false),
        other => other.ok().map(|_| bytes > 2).map_err(|err| err.to_string()),
    }
}
pub fn set_enabled(enabled: bool, exe_path: &Path) -> Result<(), String> {
    let status = if enabled {
        let wide: Vec<_> = format_run_value(exe_path)?
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let size = u32::try_from(wide.len())
            .ok()
            .and_then(|n| n.checked_mul(2))
            .ok_or("Path too long")?;
        unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                RUN,
                NAME,
                REG_SZ.0,
                Some(wide.as_ptr().cast()),
                size,
            )
        }
    } else {
        unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN, NAME) }
    };
    if !enabled && matches!(status, ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND) {
        return Ok(());
    }
    status.ok().map_err(|err| err.to_string())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quoted_without_rewriting() {
        let path = r"C:\本地 应用\TokenPulse.exe";
        assert_eq!(
            format_run_value(Path::new(path)).unwrap(),
            format!("\"{path}\"")
        );
        for invalid in ["", "  ", "C:\\dir\\", "foo\0bar", "a\nb", "a\tb", "a\"b"] {
            assert!(format_run_value(Path::new(invalid)).is_err());
        }
    }
}

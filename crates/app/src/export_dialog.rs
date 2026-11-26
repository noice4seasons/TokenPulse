use crate::config_store::atomic_write;
use std::{ffi::OsString, mem::size_of, os::windows::ffi::OsStringExt, path::PathBuf};
use tokenpulse_store_sqlite::{export, ExactRange, ExportFormat};
use windows::{
    core::{w, PWSTR},
    Win32::{Foundation::HWND, UI::Controls::Dialogs::*},
};

pub fn save_export(
    owner: HWND,
    range: &ExactRange,
    format: ExportFormat,
) -> Result<Option<String>, String> {
    let (name, filter, extension) = match format {
        ExportFormat::Json => (
            "tokenpulse-usage-summary.json",
            w!("JSON (*.json)\0*.json\0"),
            w!("json"),
        ),
        ExportFormat::Csv => (
            "tokenpulse-usage-summary.csv",
            w!("CSV (*.csv)\0*.csv\0"),
            w!("csv"),
        ),
    };
    let mut buffer = vec![0u16; 32768];
    for (slot, unit) in buffer.iter_mut().zip(name.encode_utf16()) {
        *slot = unit;
    }
    let mut dialog = OPENFILENAMEW {
        lStructSize: size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: owner,
        lpstrFilter: filter,
        nFilterIndex: 1,
        lpstrDefExt: extension,
        lpstrFile: PWSTR(buffer.as_mut_ptr()),
        nMaxFile: buffer.len() as u32,
        Flags: OFN_EXPLORER | OFN_PATHMUSTEXIST | OFN_OVERWRITEPROMPT | OFN_NOCHANGEDIR,
        ..Default::default()
    };
    if !unsafe { GetSaveFileNameW(&mut dialog) }.as_bool() {
        return match unsafe { CommDlgExtendedError() }.0 {
            0 => Ok(None),
            code => Err(format!("Save dialog failed (0x{code:04x})")),
        };
    }
    let end = buffer
        .iter()
        .position(|&unit| unit == 0)
        .ok_or("Invalid file name")?;
    let path = PathBuf::from(OsString::from_wide(&buffer[..end]));
    let filename = path
        .file_name()
        .ok_or("Missing file name")?
        .to_string_lossy()
        .into_owned();
    let text = export(range, format).map_err(|_| "Could not prepare usage export")?;
    atomic_write(&path, text.as_bytes()).map_err(|_| "Could not save usage export")?;
    Ok(Some(filename))
}

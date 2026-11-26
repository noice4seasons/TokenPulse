use crate::checkpoint::FileIdentity;
use std::{
    env,
    fs::{self, File, OpenOptions},
    io,
    path::{Component, Path, PathBuf},
};

#[derive(Debug, Clone)]
pub struct AntigravityRoots {
    pub brain_dir: PathBuf,
}

impl AntigravityRoots {
    pub fn new(brain_dir: impl Into<PathBuf>) -> Self {
        Self {
            brain_dir: brain_dir.into(),
        }
    }

    pub fn from_env() -> io::Result<Self> {
        let home = env::var_os("ANTIGRAVITY_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                env::var_os("USERPROFILE")
                    .map(|profile| PathBuf::from(profile).join(".gemini/antigravity"))
            })
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "missing local home"))?;
        Ok(Self::new(normalize(&home.join("brain"))?))
    }
}

#[cfg(windows)]
fn open_local(path: &Path, directory: bool) -> io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows::Win32::Storage::FileSystem::*;
    check_drive(path)?;
    let root = path.ancestors().last().ok_or_else(unsafe_path)?;
    let relative = path.strip_prefix(root).map_err(|_| unsafe_path())?;
    let anchor = OpenOptions::new()
        .access_mode(if relative.as_os_str().is_empty() {
            FILE_GENERIC_READ.0
        } else {
            FILE_READ_ATTRIBUTES.0
        })
        .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0 | FILE_SHARE_DELETE.0)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0 | FILE_FLAG_OPEN_REPARSE_POINT.0)
        .open(root)?;
    check_link(&anchor.metadata()?)?;
    let file = if relative.as_os_str().is_empty() {
        anchor
    } else {
        open_below(&anchor, relative)?
    };
    let meta = file.metadata()?;
    if (directory && !meta.is_dir()) || (!directory && !meta.is_file()) {
        return Err(unsafe_path());
    }
    Ok(file)
}

#[cfg(all(target_os = "linux", target_pointer_width = "64"))]
fn open_local(path: &Path, directory: bool) -> io::Result<File> {
    use std::ffi::{c_int, c_long, c_void};
    use std::os::{fd::AsRawFd, unix::fs::OpenOptionsExt};
    extern "C" {
        fn fstatfs(fd: c_int, buf: *mut c_void) -> c_int;
    }
    fn local_fs(file: &File) -> io::Result<()> {
        let mut stat = [0 as c_long; 32];
        if unsafe { fstatfs(file.as_raw_fd(), stat.as_mut_ptr().cast()) } != 0 {
            return Err(io::Error::last_os_error());
        }
        match stat[0] {
            0xef53 | 0x58465342 | 0x9123683e | 0x01021994 | 0x2fc12fc1 | 0xf2f52010 => Ok(()),
            _ => Err(unsafe_path()),
        }
    }
    let pin = |path: &Path| {
        OpenOptions::new()
            .read(true)
            .custom_flags(0o10000000 | 0o400000)
            .open(path)
    };
    let mut file = pin(Path::new("/"))?;
    local_fs(&file)?;
    let mut names = path.components().skip(1).peekable();
    while let Some(Component::Normal(name)) = names.next() {
        let child = PathBuf::from(format!("/proc/self/fd/{}", file.as_raw_fd())).join(name);
        let want_dir = directory || names.peek().is_some();
        file = pin(&child)?;
        local_fs(&file)?;
        let metadata = file.metadata()?;
        check_link(&metadata)?;
        if (want_dir && !metadata.is_dir()) || (!want_dir && !metadata.is_file()) {
            return Err(unsafe_path());
        }
    }
    if !directory && !file.metadata()?.is_file() {
        return Err(unsafe_path());
    }
    File::open(format!("/proc/self/fd/{}", file.as_raw_fd()))
}

#[cfg(not(any(windows, all(target_os = "linux", target_pointer_width = "64"))))]
fn open_local(_: &Path, _: bool) -> io::Result<File> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "no guarded local reader",
    ))
}

#[cfg(not(windows))]
fn children(file: &File) -> io::Result<Vec<(std::ffi::OsString, bool)>> {
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsRawFd;
        fs::read_dir(format!("/proc/self/fd/{}", file.as_raw_fd()))?
            .map(|entry| {
                let entry = entry?;
                Ok((entry.file_name(), entry.file_type()?.is_dir()))
            })
            .collect()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = file;
        Err(unsafe_path())
    }
}

pub(crate) fn open_log(root: &Path, path: &Path) -> io::Result<(File, DiscoveredTranscript)> {
    let root = normalize(root)?;
    let path = normalize(path)?;
    let relative = path.strip_prefix(&root).map_err(|_| unsafe_path())?;
    let mut parts = relative.iter();
    let id = parts
        .next()
        .and_then(|id| id.to_str())
        .ok_or_else(unsafe_path)?
        .to_owned();
    if !parts.eq([".system_generated", "logs", "transcript.jsonl"].map(std::ffi::OsStr::new)) {
        return Err(unsafe_path());
    }
    let file = open_local(&path, false)?;
    let found = describe(&file, path, id)?;
    Ok((file, found))
}

pub fn validate_safe_local_path(root: &Path, candidate: &Path) -> io::Result<PathBuf> {
    Ok(open_log(root, candidate)?.1.path)
}

pub fn discover_transcript_files(
    roots: &AntigravityRoots,
) -> io::Result<Vec<DiscoveredTranscript>> {
    let root = normalize(&roots.brain_dir)?;
    let brain = match open_local(&root, true) {
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(vec![]),
        other => other?,
    };
    let mut found = Vec::new();
    for (name, is_dir) in children(&brain)? {
        if !is_dir || !admissible_name(&name) {
            continue;
        }
        let candidate = root
            .join(name)
            .join(".system_generated/logs/transcript.jsonl");
        match open_log(&root, &candidate) {
            Ok((_, entry)) => found.push(entry),
            Err(err)
                if matches!(
                    err.kind(),
                    io::ErrorKind::InvalidInput
                        | io::ErrorKind::NotFound
                        | io::ErrorKind::NotADirectory
                ) =>
            {
                continue;
            }
            Err(err) => return Err(err),
        }
    }
    found.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(found)
}

fn unsafe_path() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "unsafe local path")
}
fn admissible_name(name: &std::ffi::OsStr) -> bool {
    let s = name.to_string_lossy();
    let lower = s.to_ascii_lowercase();
    if lower == "auth.json"
        || lower == "state.vscdb"
        || (lower.starts_with("hub_") && lower.ends_with(".json"))
    {
        return false;
    }
    #[cfg(windows)]
    {
        // Exclude DOS devices, alternate streams and Win32 trailing-dot/space aliases.
        let stem = s
            .split('.')
            .next()
            .unwrap_or("")
            .trim_end()
            .to_ascii_uppercase();
        let short_alias = stem
            .rsplit_once('~')
            .is_some_and(|(_, n)| !n.is_empty() && n.bytes().all(|c| c.is_ascii_digit()));
        if (short_alias
            && s.rsplit_once('.')
                .is_some_and(|(_, ext)| ext.eq_ignore_ascii_case("jso")))
            || s.ends_with([' ', '.'])
            || s.contains([':', '\0', '/', '\\'])
            || matches!(
                stem.as_str(),
                "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
            )
            || (stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem[3..].chars().count() == 1
                && "0123456789¹²³".contains(&stem[3..])
        {
            return false;
        }
    }
    true
}

// Validate *all* components before doing any filesystem I/O, including for the root.
fn normalize(path: &Path) -> io::Result<PathBuf> {
    let spelling = path.as_os_str().to_string_lossy();
    if !path.is_absolute() || spelling.starts_with("//") {
        return Err(unsafe_path());
    }
    #[cfg(not(windows))]
    if spelling.starts_with(r"\\") {
        return Err(unsafe_path());
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            #[cfg(windows)]
            Component::Prefix(p) => match p.kind() {
                std::path::Prefix::Disk(letter) | std::path::Prefix::VerbatimDisk(letter)
                    if letter.is_ascii_alphabetic() =>
                {
                    normalized.push(format!("{}:", letter as char))
                }
                _ => return Err(unsafe_path()),
            },
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => (),
            Component::Normal(name) if admissible_name(name) => normalized.push(name),
            _ => return Err(unsafe_path()),
        }
    }
    Ok(normalized)
}

#[cfg(windows)]
fn check_drive(path: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{core::PCWSTR, Win32::Storage::FileSystem::GetDriveTypeW};
    let root = path.ancestors().last().ok_or_else(unsafe_path)?;
    let name: Vec<u16> = root.as_os_str().encode_wide().chain(Some(0)).collect();
    // Only removable, fixed, optical and RAM disks. This also rejects DRIVE_REMOTE (4).
    if matches!(
        unsafe { GetDriveTypeW(PCWSTR(name.as_ptr())) },
        2 | 3 | 5 | 6
    ) {
        Ok(())
    } else {
        Err(unsafe_path())
    }
}

#[cfg(windows)]
fn check_link(m: &fs::Metadata) -> io::Result<()> {
    use std::os::windows::fs::MetadataExt;
    use windows::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_OFFLINE, FILE_ATTRIBUTE_REPARSE_POINT,
    };
    if m.file_attributes() & (FILE_ATTRIBUTE_REPARSE_POINT.0 | FILE_ATTRIBUTE_OFFLINE.0) != 0 {
        Err(unsafe_path())
    } else {
        Ok(())
    }
}
#[cfg(not(windows))]
fn check_link(m: &fs::Metadata) -> io::Result<()> {
    if m.file_type().is_symlink() {
        Err(unsafe_path())
    } else {
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct DiscoveredTranscript {
    pub conversation_id: String,
    pub path: PathBuf,
    pub identity: FileIdentity,
    pub size: u64,
    pub mtime_ms: i64,
}

pub(crate) fn describe(
    file: &File,
    path: PathBuf,
    conversation_id: String,
) -> io::Result<DiscoveredTranscript> {
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::{
            Foundation::HANDLE,
            Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION},
        };
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        // file owns this handle for the entire call.
        unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }
            .map_err(io::Error::other)?;
        if info.nNumberOfLinks != 1 {
            return Err(unsafe_path());
        }
        let join = |hi: u32, lo: u32| (u64::from(hi) << 32) | u64::from(lo);
        let ticks = join(
            info.ftLastWriteTime.dwHighDateTime,
            info.ftLastWriteTime.dwLowDateTime,
        );
        Ok(DiscoveredTranscript {
            conversation_id,
            path,
            identity: FileIdentity {
                volume_serial: Some(info.dwVolumeSerialNumber),
                file_index: Some(join(info.nFileIndexHigh, info.nFileIndexLow)),
                creation_time_100ns: Some(join(
                    info.ftCreationTime.dwHighDateTime,
                    info.ftCreationTime.dwLowDateTime,
                )),
            },
            size: join(info.nFileSizeHigh, info.nFileSizeLow),
            mtime_ms: (ticks / 10_000) as i64 - 11_644_473_600_000,
        })
    }
    #[cfg(not(windows))]
    {
        use std::os::unix::fs::MetadataExt;
        let m = file.metadata()?;
        if m.nlink() != 1 {
            return Err(unsafe_path());
        }
        let identity = FileIdentity {
            volume_serial: u32::try_from(m.dev()).ok(),
            file_index: Some(m.ino()),
            creation_time_100ns: None,
        };
        Ok(DiscoveredTranscript {
            conversation_id,
            path,
            identity,
            size: m.len(),
            mtime_ms: m
                .mtime()
                .saturating_mul(1000)
                .saturating_add(m.mtime_nsec() / 1_000_000),
        })
    }
}

#[cfg(windows)]
fn open_below(anchor: &File, relative: &Path) -> io::Result<File> {
    use std::{
        ffi::c_void,
        mem::size_of,
        os::windows::{
            ffi::OsStrExt,
            io::{AsRawHandle, FromRawHandle},
        },
        ptr,
    };
    use windows::Win32::Storage::FileSystem::*;
    #[repr(C)]
    struct UnicodeString {
        len: u16,
        capacity: u16,
        buffer: *const u16,
    }
    #[repr(C)]
    struct Attributes {
        size: u32,
        root: *mut c_void,
        name: *const UnicodeString,
        flags: u32,
        security: *mut c_void,
        qos: *mut c_void,
    }
    #[repr(C)]
    struct IoStatus {
        status: usize,
        information: usize,
    }
    #[link(name = "ntdll")]
    extern "system" {
        fn NtOpenFile(
            file: *mut *mut c_void,
            access: u32,
            attributes: *const Attributes,
            status: *mut IoStatus,
            share: u32,
            options: u32,
        ) -> i32;
        fn RtlNtStatusToDosError(status: i32) -> u32;
    }
    let wide: Vec<u16> = relative.as_os_str().encode_wide().collect();
    let len = u16::try_from(wide.len() * 2).map_err(|_| unsafe_path())?;
    let name = UnicodeString {
        len,
        capacity: len,
        buffer: wide.as_ptr(),
    };
    let attr = Attributes {
        size: size_of::<Attributes>() as u32,
        root: anchor.as_raw_handle(),
        name: &name,
        flags: 0x40 | 0x1000,
        security: ptr::null_mut(),
        qos: ptr::null_mut(),
    }; // CASE_INSENSITIVE | DONT_REPARSE
    let mut out = ptr::null_mut();
    let mut ios = IoStatus {
        status: 0,
        information: 0,
    };
    // FILE_OPEN_REPARSE_POINT | FILE_SYNCHRONOUS_IO_NONALERT. All referenced
    // buffers and the anchor remain alive until this synchronous call returns.
    let status = unsafe {
        NtOpenFile(
            &mut out,
            FILE_GENERIC_READ.0,
            &attr,
            &mut ios,
            FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0 | FILE_SHARE_DELETE.0,
            0x00200000 | 0x00400000 | 0x20,
        )
    };
    if status < 0 {
        if matches!(status as u32, 0xc000050b | 0xc0000280) {
            return Err(unsafe_path());
        }
        return Err(io::Error::from_raw_os_error(
            unsafe { RtlNtStatusToDosError(status) } as i32,
        ));
    }
    let f = unsafe { File::from_raw_handle(out) };
    check_link(&f.metadata()?)?;
    Ok(f)
}

// Enumerate a previously opened directory without resolving its pathname again.
#[cfg(windows)]
fn children(file: &File) -> io::Result<Vec<(std::ffi::OsString, bool)>> {
    use std::{
        mem::{offset_of, size_of_val},
        os::windows::{ffi::OsStringExt, io::AsRawHandle},
    };
    use windows::Win32::{Foundation::HANDLE, Storage::FileSystem::*};
    let mut out = Vec::new();
    let mut storage = [0u64; 4096]; // alignment for FILE_FULL_DIR_INFO
    let capacity = size_of_val(&storage);
    loop {
        let result = unsafe {
            GetFileInformationByHandleEx(
                HANDLE(file.as_raw_handle()),
                FileFullDirectoryInfo,
                storage.as_mut_ptr().cast(),
                capacity as u32,
            )
        };
        if let Err(e) = result {
            if e.code().0 as u32 == 0x80070012 {
                break;
            } // ERROR_NO_MORE_FILES
            return Err(io::Error::other(e));
        }
        let mut at = 0;
        loop {
            let header = offset_of!(FILE_FULL_DIR_INFO, FileName);
            if at % 8 != 0 || at + std::mem::size_of::<FILE_FULL_DIR_INFO>() > capacity {
                return Err(unsafe_path());
            }
            let ptr = unsafe {
                storage
                    .as_ptr()
                    .cast::<u8>()
                    .add(at)
                    .cast::<FILE_FULL_DIR_INFO>()
            };
            let record = unsafe { &*ptr };
            let len = record.FileNameLength as usize;
            if len & 1 != 0 || len > capacity - at - header {
                return Err(unsafe_path());
            }
            let chars = unsafe {
                std::slice::from_raw_parts(
                    storage.as_ptr().cast::<u8>().add(at + header).cast::<u16>(),
                    len / 2,
                )
            };
            let name = std::ffi::OsString::from_wide(chars);
            if name != "."
                && name != ".."
                && admissible_name(&name)
                && record.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT.0 == 0
            {
                out.push((
                    name,
                    record.FileAttributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0,
                ));
            }
            if record.NextEntryOffset == 0 {
                break;
            }
            at += record.NextEntryOffset as usize;
        }
    }
    Ok(out)
}

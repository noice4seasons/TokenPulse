use crate::checkpoint::FileIdentity;
use std::{
    env,
    fs::{self, File, OpenOptions},
    io,
    path::{Component, Path, PathBuf},
};

#[derive(Clone, Debug)]
pub struct CodexRoots {
    pub home: PathBuf,
}

impl CodexRoots {
    pub fn new(home: impl Into<PathBuf>) -> Self {
        Self { home: home.into() }
    }
    pub fn from_env() -> io::Result<Self> {
        let home = env::var_os("CODEX_HOME")
            .map(PathBuf::from)
            .or_else(|| env::var_os("USERPROFILE").map(|s| PathBuf::from(s).join(".codex")))
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "missing local home"))?;
        Ok(Self::new(normalize(&home)?))
    }
    pub fn scan_roots(&self) -> [PathBuf; 3] {
        ["sessions", "archived_sessions", "session-cleanup-archive"]
            .map(|name| self.home.join(name))
    }
    pub fn global_state_path(&self) -> PathBuf {
        self.home.join(".codex-global-state.json")
    }
}

fn unsafe_path() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "unsafe local path")
}
fn admissible_name(name: &std::ffi::OsStr) -> bool {
    let s = name.to_string_lossy();
    if s.eq_ignore_ascii_case("auth.json") {
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
    use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
    if m.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 {
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

struct GuardedPath {
    path: PathBuf,
    _parents: Vec<File>,
    #[cfg(windows)]
    target: Option<File>,
}

fn guard(root: &Path, candidate: &Path) -> io::Result<GuardedPath> {
    let root = normalize(root)?;
    let path = normalize(candidate)?;
    if !path.starts_with(&root) {
        return Err(unsafe_path());
    }
    #[cfg(windows)]
    check_drive(&path)?;
    let mut ancestors: Vec<_> = path.ancestors().skip(1).collect();
    ancestors.reverse();
    #[allow(unused_mut)]
    let mut parents = Vec::new();
    for ancestor in ancestors {
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            use windows::Win32::Storage::FileSystem::*;
            // Pin each ancestor against rename and reparse-point modification until
            // the child handle is open. Never follow a junction to inspect its target.
            let opened = OpenOptions::new()
                .access_mode(FILE_READ_ATTRIBUTES.0)
                .share_mode(FILE_SHARE_READ.0)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0 | FILE_FLAG_OPEN_REPARSE_POINT.0)
                .open(ancestor);
            let dir = match opened {
                Ok(file) => file,
                Err(e) if e.kind() == io::ErrorKind::PermissionDenied && !parents.is_empty() => {
                    let parent = ancestor.parent().ok_or_else(unsafe_path)?;
                    let relative = path.strip_prefix(parent).map_err(|_| unsafe_path())?;
                    let f = open_below(parents.last().unwrap(), relative)?;
                    return Ok(GuardedPath {
                        path,
                        _parents: parents,
                        target: Some(f),
                    });
                }
                Err(e) => return Err(e),
            };
            let m = dir.metadata()?;
            check_link(&m)?;
            if !m.is_dir() {
                return Err(unsafe_path());
            }
            parents.push(dir);
        }
        #[cfg(not(windows))]
        {
            let m = fs::symlink_metadata(ancestor)?;
            check_link(&m)?;
            if !m.is_dir() {
                return Err(unsafe_path());
            }
        }
    }
    let m = fs::symlink_metadata(&path)?;
    check_link(&m)?;
    if !m.is_file() && !m.is_dir() {
        return Err(unsafe_path());
    }
    Ok(GuardedPath {
        path,
        _parents: parents,
        #[cfg(windows)]
        target: None,
    })
}

/// Return an existing, absolute local path. Opens revalidate at point of use.
pub fn validate_safe_local_path(root: &Path, candidate: &Path) -> io::Result<PathBuf> {
    Ok(guard(root, candidate)?.path)
}
pub(crate) fn open_file(root: &Path, candidate: &Path) -> io::Result<File> {
    let checked = guard(root, candidate)?;
    #[cfg(windows)]
    if let Some(file) = checked.target {
        if !file.metadata()?.is_file() {
            return Err(unsafe_path());
        }
        return Ok(file);
    }
    let mut opts = OpenOptions::new();
    opts.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows::Win32::Storage::FileSystem::*;
        opts.share_mode(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0 | FILE_SHARE_DELETE.0)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0);
    }
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.custom_flags(0o400000 | 0o4000);
    } // O_NOFOLLOW | O_NONBLOCK
    let file = opts.open(&checked.path)?;
    let m = file.metadata()?;
    check_link(&m)?;
    if !m.is_file() {
        return Err(unsafe_path());
    }
    Ok(file)
}

#[derive(Debug, Clone)]
pub struct DiscoveredFile {
    pub path: PathBuf,
    pub identity: FileIdentity,
    pub size: u64,
    pub mtime_ms: i64,
}

pub(crate) fn describe(file: &File, path: PathBuf) -> io::Result<DiscoveredFile> {
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
        let join = |hi: u32, lo: u32| (u64::from(hi) << 32) | u64::from(lo);
        let ticks = join(
            info.ftLastWriteTime.dwHighDateTime,
            info.ftLastWriteTime.dwLowDateTime,
        );
        Ok(DiscoveredFile {
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
        let identity = FileIdentity {
            volume_serial: u32::try_from(m.dev()).ok(),
            file_index: Some(m.ino()),
            creation_time_100ns: None,
        };
        Ok(DiscoveredFile {
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

pub(crate) fn jsonl(path: &Path) -> bool {
    path.extension()
        .is_some_and(|s| s.eq_ignore_ascii_case("jsonl"))
}
fn optional<T>(result: io::Result<T>) -> io::Result<Option<T>> {
    match result {
        Ok(v) => Ok(Some(v)),
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::InvalidInput
            ) =>
        {
            Ok(None)
        }
        Err(e) => Err(e),
    }
}
pub fn discover_jsonl_files(roots: &CodexRoots) -> io::Result<Vec<DiscoveredFile>> {
    let home = normalize(&roots.home)?;
    #[cfg(windows)]
    check_drive(&home)?;
    let roots = CodexRoots::new(home);
    let mut result = Vec::new();
    for root in roots.scan_roots() {
        let mut pending = vec![root.clone()];
        while let Some(dir) = pending.pop() {
            let Some(checked) = optional(guard(&root, &dir))? else {
                continue;
            };
            #[cfg(windows)]
            let entries = {
                use std::os::windows::fs::OpenOptionsExt;
                use windows::Win32::Storage::FileSystem::*;
                let directory = match checked.target {
                    Some(f) => f,
                    None => OpenOptions::new()
                        .read(true)
                        .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0 | FILE_SHARE_DELETE.0)
                        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0 | FILE_FLAG_OPEN_REPARSE_POINT.0)
                        .open(&checked.path)?,
                };
                if optional(check_link(&directory.metadata()?))?.is_none() {
                    continue;
                }
                children(&directory)?
            };
            #[cfg(not(windows))]
            let entries = {
                let mut entries = Vec::new();
                for entry in fs::read_dir(&checked.path)? {
                    let entry = entry?;
                    if !admissible_name(&entry.file_name()) {
                        continue;
                    }
                    let ty = entry.file_type()?;
                    if ty.is_dir() || ty.is_file() {
                        entries.push((entry.file_name(), ty.is_dir()));
                    }
                }
                entries
            };
            for (name, is_dir) in entries {
                let child = dir.join(name);
                if is_dir {
                    pending.push(child);
                } else if jsonl(&child) {
                    if let Some(f) = optional(open_file(&root, &child))? {
                        result.push(describe(&f, child)?);
                    }
                }
            }
        }
    }
    result.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(result)
}

// Some ACLs permit traversal but deny opening an intermediate directory. In
// that case use a relative NT open anchored at the last pinned directory.
// OBJ_DONT_REPARSE makes the kernel stop on a reparse anywhere in the suffix.
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
            0x00200000 | 0x20,
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
            if len % 2 != 0 || len > capacity - at - header {
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

pub(crate) fn open_log(roots: &CodexRoots, path: &Path) -> io::Result<File> {
    if !jsonl(path) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "expected session log",
        ));
    }
    let scan_roots = roots.scan_roots();
    let root = scan_roots
        .iter()
        .find(|r| path.starts_with(r))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "outside scan roots"))?;
    open_file(root, path)
}

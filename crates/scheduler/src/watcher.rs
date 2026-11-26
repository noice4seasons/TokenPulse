#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(usize)]
pub enum WatchRoot {
    Sessions,
    ArchivedSessions,
    SessionCleanupArchive,
}

impl WatchRoot {
    pub const ALL: [Self; 3] = [
        Self::Sessions,
        Self::ArchivedSessions,
        Self::SessionCleanupArchive,
    ];
    pub fn index(self) -> usize {
        self as usize
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Sessions => "sessions",
            Self::ArchivedSessions => "archived_sessions",
            Self::SessionCleanupArchive => "session-cleanup-archive",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchEvent {
    Dirty(WatchRoot),
    OverflowReconcile,
}

#[cfg(windows)]
pub use platform::DirectoryWatcher;

#[cfg(windows)]
mod platform {
    use super::{WatchEvent, WatchRoot};
    use std::cell::UnsafeCell;
    use std::ffi::c_void;
    use std::fs::{File, OpenOptions};
    use std::io;
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use std::path::{Component, Path, PathBuf, Prefix};
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Sender},
        Arc,
    };
    use std::thread::{self, JoinHandle};
    use windows::core::{Error, HRESULT, PCWSTR};
    use windows::Win32::Foundation::{
        ERROR_IO_PENDING, ERROR_NOTIFY_ENUM_DIR, HANDLE, WAIT_FAILED, WAIT_OBJECT_0,
    };
    use windows::Win32::Storage::FileSystem::{
        GetDriveTypeW, GetFileInformationByHandle, ReadDirectoryChangesW,
        BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_FLAG_OVERLAPPED, FILE_LIST_DIRECTORY,
        FILE_NOTIFY_CHANGE, FILE_NOTIFY_CHANGE_DIR_NAME, FILE_NOTIFY_CHANGE_FILE_NAME,
        FILE_NOTIFY_CHANGE_LAST_WRITE, FILE_NOTIFY_CHANGE_SIZE, FILE_SHARE_DELETE, FILE_SHARE_READ,
        FILE_SHARE_WRITE,
    };
    use windows::Win32::System::Threading::{
        ResetEvent, SetEvent, WaitForMultipleObjects, INFINITE,
    };
    use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};

    // The 0.58 projection of CreateEventW requires Win32_Security even for
    // NULL security attributes. Keep the requested four-feature manifest.
    #[link(name = "kernel32")]
    extern "system" {
        fn CreateEventW(
            attributes: *const c_void,
            manual_reset: i32,
            initial: i32,
            name: *const u16,
        ) -> *mut c_void;
    }

    fn event() -> io::Result<OwnedHandle> {
        // SAFETY: unnamed manual-reset event, default security descriptor.
        let handle = unsafe { CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
    }

    fn raw(owner: &impl AsRawHandle) -> HANDLE {
        HANDLE(owner.as_raw_handle())
    }

    #[derive(Debug, thiserror::Error)]
    enum WatchPathError {
        #[error("expected an absolute path on a local drive")]
        NotLocal,
        #[error("watch path is not a plain directory")]
        NotDirectory,
    }

    fn local_home(home: &Path) -> io::Result<()> {
        let invalid = || io::Error::new(io::ErrorKind::InvalidInput, WatchPathError::NotLocal);
        let mut parts = home.components();
        let Some(Component::Prefix(prefix)) = parts.next() else {
            return Err(invalid());
        };
        let (Prefix::Disk(letter) | Prefix::VerbatimDisk(letter)) = prefix.kind() else {
            return Err(invalid());
        };
        if parts.next() != Some(Component::RootDir)
            || !parts.all(|p| matches!(p, Component::Normal(_)))
        {
            return Err(invalid());
        }
        let drive = [u16::from(letter), 58, 92, 0];
        // DRIVE_REMOVABLE, DRIVE_FIXED, DRIVE_RAMDISK. Reject mapped/unknown drives.
        if !matches!(unsafe { GetDriveTypeW(PCWSTR(drive.as_ptr())) }, 2 | 3 | 6) {
            return Err(invalid());
        }
        Ok(())
    }

    // Pin accessible ancestors during setup. Windows can grant traverse
    // permission without granting an open handle on a parent profile.
    fn open_ancestors(home: &Path) -> io::Result<Vec<File>> {
        let mut held = Vec::new();
        let mut chain: Vec<_> = home.ancestors().collect();
        chain.reverse();
        for path in chain {
            let opened = OpenOptions::new()
                .access_mode(0)
                .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0 | FILE_FLAG_OPEN_REPARSE_POINT.0)
                .open(path);
            match opened {
                Ok(file) => {
                    verify_directory(&file.metadata()?)?;
                    held.push(file);
                }
                Err(e) if e.kind() == io::ErrorKind::PermissionDenied && path != home => {
                    verify_directory(&std::fs::symlink_metadata(path)?)?;
                }
                Err(e) => return Err(e),
            }
        }
        Ok(held)
    }

    fn verify_directory(meta: &std::fs::Metadata) -> io::Result<()> {
        if !meta.is_dir() || meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                WatchPathError::NotDirectory,
            ));
        }
        Ok(())
    }

    fn open_directory(path: &Path) -> io::Result<File> {
        let file = OpenOptions::new()
            .access_mode(FILE_LIST_DIRECTORY.0)
            .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0 | FILE_SHARE_DELETE.0)
            .custom_flags(
                FILE_FLAG_BACKUP_SEMANTICS.0
                    | FILE_FLAG_OPEN_REPARSE_POINT.0
                    | FILE_FLAG_OVERLAPPED.0,
            )
            .open(path)?;
        verify_directory(&file.metadata()?)?;
        Ok(file)
    }

    fn identity(file: &File) -> io::Result<(u32, u32, u32)> {
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        unsafe { GetFileInformationByHandle(raw(file), &mut info) }?;
        Ok((
            info.dwVolumeSerialNumber,
            info.nFileIndexHigh,
            info.nFileIndexLow,
        ))
    }

    // Each stream needs a distinct file object: Windows associates its change
    // buffer and filter with the original directory handle.
    type Buffer = [u32; 4096];

    struct Stream {
        directory: File,
        event: OwnedHandle,
        overlapped: Box<UnsafeCell<OVERLAPPED>>,
        // DWORD-aligned, fixed-size backing allocation retained through completion.
        buffer: Box<UnsafeCell<Buffer>>,
        directories: bool,
        pending: bool,
    }

    impl Stream {
        fn new(directory: File, directories: bool) -> io::Result<Self> {
            let event = event()?;
            Ok(Self {
                directory,
                event,
                overlapped: Box::new(UnsafeCell::new(OVERLAPPED::default())),
                buffer: Box::new(UnsafeCell::new([0; 4096])),
                directories,
                pending: false,
            })
        }

        fn filter(&self) -> FILE_NOTIFY_CHANGE {
            if self.directories {
                FILE_NOTIFY_CHANGE_DIR_NAME
            } else {
                FILE_NOTIFY_CHANGE_FILE_NAME
                    | FILE_NOTIFY_CHANGE_SIZE
                    | FILE_NOTIFY_CHANGE_LAST_WRITE
            }
        }

        fn arm(&mut self) -> windows::core::Result<()> {
            debug_assert!(!self.pending);
            let filter = self.filter();
            // SAFETY: both allocations have stable addresses until result() or Drop
            // observes completion. The event is private to this one operation.
            unsafe {
                ResetEvent(raw(&self.event))?;
                let overlapped = self.overlapped.get();
                overlapped.write(OVERLAPPED {
                    hEvent: raw(&self.event),
                    ..Default::default()
                });
                let read = ReadDirectoryChangesW(
                    raw(&self.directory),
                    self.buffer.get().cast(),
                    std::mem::size_of::<Buffer>() as u32,
                    true,
                    filter,
                    None,
                    Some(overlapped),
                    None,
                );
                match read {
                    Err(e) if e.code() != HRESULT::from_win32(ERROR_IO_PENDING.0) => return Err(e),
                    _ => self.pending = true,
                }
            }
            Ok(())
        }

        fn result(&mut self) -> windows::core::Result<u32> {
            let mut bytes = 0;
            let result = unsafe {
                GetOverlappedResult(
                    raw(&self.directory),
                    self.overlapped.get(),
                    &mut bytes,
                    true,
                )
            };
            self.pending = false;
            result.map(|()| bytes)
        }

        fn classify(&self, count: u32, root: WatchRoot) -> Option<WatchEvent> {
            // SAFETY: completion returned; no read is reissued until parsing ends.
            let all = unsafe {
                std::slice::from_raw_parts(
                    self.buffer.get().cast::<u8>(),
                    std::mem::size_of::<Buffer>(),
                )
            };
            match all
                .get(..count as usize)
                .and_then(|bytes| relevant(bytes, self.directories))
            {
                Some(true) => Some(WatchEvent::Dirty(root)),
                Some(false) => None,
                None => Some(WatchEvent::OverflowReconcile),
            }
        }
    }

    impl Drop for Stream {
        fn drop(&mut self) {
            if self.pending {
                // ERROR_NOT_FOUND means completion raced with cancellation.
                // In both cases drain the operation before freeing its storage.
                let _ = unsafe { CancelIoEx(raw(&self.directory), Some(self.overlapped.get())) };
                let _ = self.result();
            }
        }
    }

    fn matches_ascii(name: &[u8], ascii: &[u8]) -> bool {
        name.len() == ascii.len() * 2
            && name
                .chunks_exact(2)
                .zip(ascii)
                .all(|(unit, b)| unit[1] == 0 && unit[0].eq_ignore_ascii_case(b))
    }

    // FILE_NOTIFY_INFORMATION has three DWORDs followed by a non-NUL-terminated
    // UTF-16 name. Use byte offsets to avoid alignment and flexible-array casts.
    fn relevant(mut bytes: &[u8], directories: bool) -> Option<bool> {
        let mut dirty = false;
        loop {
            let next = u32::from_le_bytes(bytes.get(0..4)?.try_into().ok()?) as usize;
            let len = u32::from_le_bytes(bytes.get(8..12)?.try_into().ok()?) as usize;
            if len == 0 || len % 2 != 0 {
                return None;
            }
            let end = 12usize.checked_add(len)?;
            let name = bytes.get(12..end)?;
            let mut start = 0;
            let mut credential = false;
            for (i, unit) in name.chunks_exact(2).enumerate() {
                if unit == [92, 0] || unit == [47, 0] {
                    credential |= matches_ascii(&name[start..i * 2], b"auth.json");
                    start = (i + 1) * 2;
                }
            }
            let leaf = &name[start..];
            credential |= matches_ascii(leaf, b"auth.json");
            dirty |= !credential
                && (directories
                    || leaf.len() >= 12 && matches_ascii(&leaf[leaf.len() - 12..], b".jsonl"));
            if next == 0 {
                return Some(dirty);
            }
            if next < end || next % 4 != 0 {
                return None;
            }
            bytes = bytes.get(next..)?;
        }
    }

    fn overflow(error: &Error) -> bool {
        error.code() == HRESULT::from_win32(ERROR_NOTIFY_ENUM_DIR.0)
    }

    fn arm<T: From<WatchEvent>>(stream: &mut Stream, tx: &Sender<T>) -> io::Result<()> {
        if let Err(error) = stream.arm() {
            if !overflow(&error) {
                return Err(error.into());
            }
            let _ = tx.send(WatchEvent::OverflowReconcile.into());
            stream.arm()?;
        }
        Ok(())
    }

    fn listen<T: From<WatchEvent>>(
        files: [File; 2],
        stop: &OwnedHandle,
        root: WatchRoot,
        tx: &Sender<T>,
        ready: mpsc::SyncSender<io::Result<()>>,
    ) -> io::Result<()> {
        let [files, dirs] = files;
        let init = (|| -> io::Result<_> {
            let mut streams = [Stream::new(files, false)?, Stream::new(dirs, true)?];
            for stream in &mut streams {
                arm(stream, tx)?;
            }
            Ok(streams)
        })();
        let mut streams = match init {
            Ok(streams) => streams,
            Err(error) => {
                let _ = ready.send(Err(error));
                return Ok(());
            }
        };
        if ready.send(Ok(())).is_err() {
            return Ok(());
        }
        let mut order = [0, 1];
        loop {
            // Stop always wins. Rotate the other two slots so a flood of
            // unrelated writes cannot starve directory rename notifications.
            let handles = [
                raw(stop),
                raw(&streams[order[0]].event),
                raw(&streams[order[1]].event),
            ];
            let wait = unsafe { WaitForMultipleObjects(&handles, false, INFINITE) };
            if wait == WAIT_FAILED {
                return Err(io::Error::last_os_error());
            }
            let index = wait.0.wrapping_sub(WAIT_OBJECT_0.0);
            if index == 0 {
                return Ok(());
            }
            let selected = order
                .get((index - 1) as usize)
                .ok_or_else(|| io::Error::other("unexpected directory wait result"))?;
            let stream = &mut streams[*selected];
            order.swap(0, 1);
            let hint = match stream.result() {
                Ok(bytes) => stream.classify(bytes, root),
                Err(error) if overflow(&error) => Some(WatchEvent::OverflowReconcile),
                Err(error) => return Err(error.into()),
            };
            if let Some(event) = hint {
                if tx.send(event.into()).is_err() {
                    return Ok(());
                }
            }
            arm(stream, tx)?;
        }
    }

    struct Worker {
        identity: (u32, u32, u32),
        stop: Arc<OwnedHandle>,
        retired: Arc<AtomicBool>,
        join: JoinHandle<()>,
    }

    impl Worker {
        fn signal(&self) -> io::Result<()> {
            unsafe { SetEvent(raw(&*self.stop)) }?;
            Ok(())
        }
        fn join(self) -> io::Result<()> {
            self.join
                .join()
                .map_err(|_| io::Error::other("directory worker panicked"))
        }
    }

    pub struct DirectoryWatcher<T = WatchEvent> {
        home: PathBuf,
        sender: Sender<T>,
        workers: [Option<Worker>; 3],
    }

    impl<T: From<WatchEvent> + Send + 'static> DirectoryWatcher<T> {
        pub fn start(home: &Path, sender: Sender<T>) -> io::Result<Self> {
            local_home(home)?;
            let mut watcher = Self {
                home: home.to_owned(),
                sender,
                workers: [None, None, None],
            };
            watcher.refresh()?;
            Ok(watcher)
        }

        /// Call before each full reconciliation so newly present roots are
        /// armed before the scan takes its snapshot.
        pub fn refresh(&mut self) -> io::Result<()> {
            for root in WatchRoot::ALL {
                let slot = &mut self.workers[root.index()];
                let path = self.home.join(root.name());
                let opened = (|| -> io::Result<_> {
                    let ancestors = open_ancestors(&self.home)?;
                    Ok((ancestors, open_directory(&path)?))
                })();
                let (ancestors, first) = match opened {
                    Ok(opened) => opened,
                    Err(e) if e.kind() == io::ErrorKind::NotFound => {
                        if let Some(old) = slot.take() {
                            old.signal()?;
                            old.join()?;
                        }
                        continue;
                    }
                    Err(e) => return Err(e),
                };
                let id = identity(&first)?;
                if slot
                    .as_ref()
                    .is_some_and(|w| !w.retired.load(Ordering::Acquire) && w.identity == id)
                {
                    continue;
                }
                if let Some(old) = slot.take() {
                    old.signal()?;
                    old.join()?;
                }
                let second = match open_directory(&path) {
                    Ok(file) => file,
                    Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                    Err(e) => return Err(e),
                };
                if identity(&second)? != id {
                    let _ = self.sender.send(WatchEvent::OverflowReconcile.into());
                    continue;
                }
                let files = [first, second];
                drop(ancestors);
                let stop = Arc::new(event()?);
                let retired = Arc::new(AtomicBool::new(false));
                let signal = stop.clone();
                let done = retired.clone();
                let tx = self.sender.clone();
                let (ready, armed) = mpsc::sync_channel(1);
                let join = thread::Builder::new()
                    .name(format!("watch-{}", root.name()))
                    .spawn(move || {
                        let result = listen(files, &signal, root, &tx, ready);
                        done.store(true, Ordering::Release);
                        if result.is_err() {
                            let _ = tx.send(WatchEvent::OverflowReconcile.into());
                        }
                    })?;
                *slot = Some(Worker {
                    identity: id,
                    stop,
                    retired,
                    join,
                });
                armed
                    .recv()
                    .map_err(|_| io::Error::other("directory worker exited during startup"))??;
            }
            Ok(())
        }
    }

    impl<T> DirectoryWatcher<T> {
        pub fn stop(&mut self) -> io::Result<()> {
            for worker in self.workers.iter().flatten() {
                worker.signal()?;
            }
            for slot in &mut self.workers {
                if let Some(worker) = slot.take() {
                    worker.join()?;
                }
            }
            Ok(())
        }
    }

    impl<T> Drop for DirectoryWatcher<T> {
        fn drop(&mut self) {
            let _ = self.stop();
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn lost_and_truncated_notifications_request_reconciliation() {
            assert!(overflow(&Error::from_hresult(HRESULT::from_win32(
                ERROR_NOTIFY_ENUM_DIR.0
            ))));
            let tmp = tempfile::tempdir().unwrap();
            let stream = Stream::new(open_directory(tmp.path()).unwrap(), false).unwrap();
            for count in [0, 1, 8, u32::MAX] {
                assert_eq!(
                    stream.classify(count, WatchRoot::Sessions),
                    Some(WatchEvent::OverflowReconcile)
                );
            }
        }
    }
}

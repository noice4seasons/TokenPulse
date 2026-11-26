use serde::{Deserialize, Serialize};
use sha2::{compress256, digest::generic_array::GenericArray, Digest, Sha256};
use std::{
    fs::File,
    io::{self, Read, Seek, SeekFrom},
};

pub(crate) const WINDOW: usize = 4096;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileIdentity {
    pub volume_serial: Option<u32>,
    pub file_index: Option<u64>,
    pub creation_time_100ns: Option<u64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DualTrackCheckpoint {
    pub byte_offset: u64,
    pub generation: u64,
    pub head_len: u32,
    pub head_sha256: String,
    pub tail_offset: u64,
    pub tail_len: u32,
    pub tail_sha256: String,
    pub full_prefix_sha256: String,
    pub file_identity: Option<FileIdentity>,
    pub fallback_timestamp_ms: i64,
    pub last_full_verify_ms: Option<i64>,
}
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum VerifyOutcome {
    Match,
    RebaseRequired,
}
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct Verification {
    pub outcome: VerifyOutcome,
    pub validation_bytes_read: u64,
}

pub(crate) fn digest(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}
fn verification(ok: bool, count: u64) -> Verification {
    Verification {
        outcome: if ok {
            VerifyOutcome::Match
        } else {
            VerifyOutcome::RebaseRequired
        },
        validation_bytes_read: count,
    }
}
fn read_window(file: &mut File, offset: u64, len: usize) -> io::Result<Vec<u8>> {
    file.seek(SeekFrom::Start(offset))?;
    let mut bytes = Vec::with_capacity(len);
    file.take(len as u64).read_to_end(&mut bytes)?;
    Ok(bytes)
}
pub(crate) fn windows(
    file: &mut File,
    cp: &DualTrackCheckpoint,
) -> io::Result<(Verification, Vec<u8>, Vec<u8>)> {
    let len = cp.byte_offset.min(WINDOW as u64) as usize;
    let tail_at = cp.byte_offset - len as u64;
    if file.metadata()?.len() < cp.byte_offset
        || cp.head_len != len as u32
        || cp.tail_len != cp.head_len
        || cp.tail_offset != tail_at
    {
        return Ok((verification(false, 0), vec![], vec![]));
    }
    let head = read_window(file, 0, len)?;
    // Only read the part of the tail not already covered by the head.
    let overlap = (len as u64).saturating_sub(tail_at) as usize;
    let suffix = read_window(file, tail_at + overlap as u64, len - overlap)?;
    let count = (head.len() + suffix.len()) as u64;
    let mut tail = head.get(len - overlap..).unwrap_or_default().to_vec();
    tail.extend(suffix);
    let valid = head.len() == len
        && tail.len() == len
        && (len == 0 || tail.last() == Some(&b'\n'))
        && digest(&head) == cp.head_sha256
        && digest(&tail) == cp.tail_sha256;
    Ok((verification(valid, count), head, tail))
}
pub fn verify_o1(file: &mut File, cp: &DualTrackCheckpoint) -> io::Result<Verification> {
    Ok(windows(file, cp)?.0)
}
pub fn verify_full_prefix(file: &mut File, cp: &DualTrackCheckpoint) -> io::Result<Verification> {
    verify_full_while(file, cp, &|| false)
}
fn verify_full_while(
    file: &mut File,
    cp: &DualTrackCheckpoint,
    cancelled: &dyn Fn() -> bool,
) -> io::Result<Verification> {
    if file.metadata()?.len() < cp.byte_offset {
        return Ok(verification(false, 0));
    }
    file.seek(SeekFrom::Start(0))?;
    let mut hash = Sha256::new();
    let mut reader = file.take(cp.byte_offset);
    let mut buffer = [0u8; 32 * 1024];
    let mut count = 0u64;
    loop {
        if cancelled() {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "prefix audit cancelled",
            ));
        }
        let n = reader.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
        count += n as u64;
    }
    Ok(verification(
        count == cp.byte_offset && hex::encode(hash.finalize()) == cp.full_prefix_sha256,
        count,
    ))
}

// Persist just the SHA-256 chaining words at the preceding 64-byte boundary.
// The uncompressed suffix is recovered from the validated tail. No log plaintext
// or library-specific hasher memory layout is serialized in resume_state_json.
const IV: [u32; 8] = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];
#[derive(Clone)]
pub(crate) struct Prefix {
    pub(crate) len: u64,
    pub(crate) words: [u32; 8],
    pending: Vec<u8>,
    head: Vec<u8>,
    tail: Vec<u8>,
}
impl Default for Prefix {
    fn default() -> Self {
        Self {
            len: 0,
            words: IV,
            pending: vec![],
            head: vec![],
            tail: vec![],
        }
    }
}
impl Prefix {
    pub(crate) fn restore(
        cp: &DualTrackCheckpoint,
        words: [u32; 8],
        head: Vec<u8>,
        tail: Vec<u8>,
    ) -> Option<Self> {
        let remainder = (cp.byte_offset % 64) as usize;
        let pending = tail.get(tail.len().checked_sub(remainder)?..)?.to_vec();
        let value = Self {
            len: cp.byte_offset,
            words,
            pending,
            head,
            tail,
        };
        (value.full_digest() == cp.full_prefix_sha256).then_some(value)
    }
    fn compress(&mut self, mut data: &[u8]) {
        if !self.pending.is_empty() {
            let take = (64 - self.pending.len()).min(data.len());
            self.pending.extend_from_slice(&data[..take]);
            if self.pending.len() < 64 {
                return;
            }
            compress256(
                &mut self.words,
                std::slice::from_ref(GenericArray::from_slice(&self.pending)),
            );
            self.pending.clear();
            data = &data[take..];
        }
        let mut blocks = data.chunks_exact(64);
        for block in &mut blocks {
            compress256(
                &mut self.words,
                std::slice::from_ref(GenericArray::from_slice(block)),
            );
        }
        self.pending.extend_from_slice(blocks.remainder());
    }
    pub(crate) fn update(&mut self, data: &[u8]) {
        self.len += data.len() as u64;
        self.compress(data);
        self.head
            .extend_from_slice(&data[..data.len().min(WINDOW - self.head.len())]);
        if data.len() >= WINDOW {
            self.tail.clear();
            self.tail.extend_from_slice(&data[data.len() - WINDOW..]);
        } else {
            let discard = (self.tail.len() + data.len()).saturating_sub(WINDOW);
            self.tail.drain(..discard);
            self.tail.extend_from_slice(data);
        }
    }
    fn full_digest(&self) -> String {
        let mut words = self.words;
        let pad_len = if self.pending.len() < 56 { 64 } else { 128 };
        let mut end = [0u8; 128];
        end[..self.pending.len()].copy_from_slice(&self.pending);
        end[self.pending.len()] = 0x80;
        end[pad_len - 8..pad_len].copy_from_slice(&self.len.wrapping_mul(8).to_be_bytes());
        for block in end[..pad_len].chunks_exact(64) {
            compress256(
                &mut words,
                std::slice::from_ref(GenericArray::from_slice(block)),
            );
        }
        hex::encode(words.map(u32::to_be_bytes).concat())
    }
    pub(crate) fn checkpoint(
        &self,
        generation: u64,
        identity: FileIdentity,
        fallback: i64,
        last_full: Option<i64>,
    ) -> DualTrackCheckpoint {
        DualTrackCheckpoint {
            byte_offset: self.len,
            generation,
            head_len: self.head.len() as u32,
            head_sha256: digest(&self.head),
            tail_offset: self.len - self.tail.len() as u64,
            tail_len: self.tail.len() as u32,
            tail_sha256: digest(&self.tail),
            full_prefix_sha256: self.full_digest(),
            file_identity: Some(identity),
            fallback_timestamp_ms: fallback,
            last_full_verify_ms: last_full,
        }
    }
}

/// A background verification result names the exact checkpoint that was audited.
/// The store must compare its generation, offset and digest before applying it.
#[derive(Clone, Debug)]
pub struct AuditTarget {
    pub path: std::path::PathBuf,
    pub checkpoint: DualTrackCheckpoint,
}
pub struct FullVerifyWorker {
    stop: std::sync::mpsc::Sender<()>,
    stopping: std::sync::Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl FullVerifyWorker {
    /// Check even idle sources. The first sweep starts immediately. The callbacks
    /// supply committed snapshots and receive audit results on the worker thread.
    /// On Match the caller may persist verified_at_ms as last_full_verify_ms.
    pub fn start(
        roots: crate::paths::CodexRoots,
        poll_interval: std::time::Duration,
        mut snapshots: impl FnMut() -> Vec<AuditTarget> + Send + 'static,
        mut report: impl FnMut(AuditTarget, i64, io::Result<Verification>) + Send + 'static,
    ) -> io::Result<Self> {
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            mpsc, Arc,
        };
        use std::time::{SystemTime, UNIX_EPOCH};
        if poll_interval.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "zero polling interval",
            ));
        }
        let (stop, rx) = mpsc::channel();
        let stopping = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stopping);
        let thread = std::thread::Builder::new()
            .name("codex-prefix-audit".into())
            .spawn(move || {
                let cancelled = || flag.load(Ordering::Acquire);
                while !cancelled() {
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map(|t| i64::try_from(t.as_millis()).unwrap_or(i64::MAX))
                        .unwrap_or(0);
                    for target in snapshots() {
                        if cancelled() {
                            return;
                        }
                        if !crate::scanner::full_verify_due(&target.checkpoint, now) {
                            continue;
                        }
                        let result =
                            crate::paths::open_log(&roots, &target.path).and_then(|mut file| {
                                verify_full_while(&mut file, &target.checkpoint, &cancelled)
                            });
                        if cancelled() {
                            return;
                        }
                        report(target, now, result);
                    }
                    if rx.recv_timeout(poll_interval) != Err(mpsc::RecvTimeoutError::Timeout) {
                        break;
                    }
                }
            })?;
        Ok(Self {
            stop,
            stopping,
            thread: Some(thread),
        })
    }
}
impl Drop for FullVerifyWorker {
    fn drop(&mut self) {
        self.stopping
            .store(true, std::sync::atomic::Ordering::Release);
        let _ = self.stop.send(());
        if let Some(handle) = self.thread.take() {
            if handle.thread().id() != std::thread::current().id() {
                let _ = handle.join();
            }
        }
    }
}

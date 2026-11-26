use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{self, Read, Seek, SeekFrom},
};

pub(crate) const WINDOW: usize = 4096;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileIdentity {
    pub volume_serial: Option<u32>,
    pub file_index: Option<u64>,
    pub creation_time_100ns: Option<u64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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

#[derive(Default, Clone)]
pub(crate) struct Prefix {
    pub len: u64,
    pub aligned: Sha256,
    pending: Vec<u8>,
    head: Vec<u8>,
    tail: Vec<u8>,
}

impl Prefix {
    pub fn update(&mut self, mut bytes: &[u8]) {
        self.len += bytes.len() as u64;
        self.head
            .extend_from_slice(&bytes[..bytes.len().min(WINDOW - self.head.len())]);
        if bytes.len() >= WINDOW {
            self.tail.clear();
            self.tail.extend_from_slice(&bytes[bytes.len() - WINDOW..]);
        } else {
            self.tail
                .drain(..(self.tail.len() + bytes.len()).saturating_sub(WINDOW));
            self.tail.extend_from_slice(bytes);
        }
        if !self.pending.is_empty() {
            let take = (64 - self.pending.len()).min(bytes.len());
            self.pending.extend_from_slice(&bytes[..take]);
            if self.pending.len() < 64 {
                return;
            }
            self.aligned.update(&self.pending);
            self.pending.clear();
            bytes = &bytes[take..];
        }
        let whole = bytes.len() / 64 * 64;
        self.aligned.update(&bytes[..whole]);
        self.pending.extend_from_slice(&bytes[whole..]);
    }

    pub fn restore(
        cp: &DualTrackCheckpoint,
        aligned: Sha256,
        head: Vec<u8>,
        tail: Vec<u8>,
    ) -> Option<Self> {
        let remaining = (cp.byte_offset % 64) as usize;
        let pending = tail.get(tail.len().checked_sub(remaining)?..)?.to_vec();
        let prefix = Self {
            len: cp.byte_offset,
            aligned,
            pending,
            head,
            tail,
        };
        (prefix.hash() == cp.full_prefix_sha256).then_some(prefix)
    }

    pub(crate) fn hash(&self) -> String {
        hex::encode(self.aligned.clone().chain_update(&self.pending).finalize())
    }

    pub fn checkpoint(
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
            full_prefix_sha256: self.hash(),
            file_identity: Some(identity),
            fallback_timestamp_ms: fallback,
            last_full_verify_ms: last_full,
        }
    }
}

pub(crate) fn audit(
    file: &mut File,
    cp: &DualTrackCheckpoint,
) -> io::Result<(Verification, Prefix)> {
    file.seek(SeekFrom::Start(0))?;
    let mut reader = file.take(cp.byte_offset);
    let mut prefix = Prefix::default();
    let mut buffer = [0; 32 * 1024];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        prefix.update(&buffer[..count]);
    }
    let valid = prefix.len == cp.byte_offset && prefix.hash() == cp.full_prefix_sha256;
    Ok((verification(valid, prefix.len), prefix))
}

pub fn verify_full_prefix(file: &mut File, cp: &DualTrackCheckpoint) -> io::Result<Verification> {
    Ok(audit(file, cp)?.0)
}

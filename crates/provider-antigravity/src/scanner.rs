use crate::{
    checkpoint::{audit, windows, DualTrackCheckpoint, Prefix, VerifyOutcome, WINDOW},
    models::normalize_antigravity_model,
    paths::{open_log, AntigravityRoots},
};
use serde::Deserialize;
use sha2::Sha256;
use std::{
    io::{self, BufRead, BufReader, Read, Seek, SeekFrom},
    path::Path,
};
use tokenpulse_core_domain::models::{
    PricingPrecision, ProviderId, TokenVector, UnpricedReason, UsageEntry,
};

pub const MAX_LINE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Deserialize)]
struct TranscriptLineMeta {
    step_index: Option<u64>,
    #[serde(rename = "type")]
    kind: Option<String>,
    status: Option<String>,
    created_at: Option<String>,
    model: Option<String>,
    input_tokens: Option<u64>,
    cache_read_tokens: Option<u64>,
    output_tokens: Option<u64>,
}

fn timestamp(raw: &str) -> Option<i64> {
    let bytes = raw.as_bytes();
    let number = |start: usize, length: usize| -> Option<i64> {
        bytes
            .get(start..start + length)?
            .iter()
            .try_fold(0i64, |value, digit| {
                digit
                    .is_ascii_digit()
                    .then(|| value * 10 + i64::from(*digit - b'0'))
            })
    };
    if bytes.get(4) != Some(&b'-')
        || bytes.get(7) != Some(&b'-')
        || !matches!(bytes.get(10), Some(b'T' | b't'))
        || bytes.get(13) != Some(&b':')
        || bytes.get(16) != Some(&b':')
    {
        return None;
    }
    let (year, month, day) = (number(0, 4)?, number(5, 2)?, number(8, 2)?);
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let limit = match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        _ => return None,
    };
    let (hour, minute, second) = (number(11, 2)?, number(14, 2)?, number(17, 2)?);
    if !(1..=limit).contains(&day) || hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    let mut at = 19;
    let mut millis = 0;
    if bytes.get(at) == Some(&b'.') {
        at += 1;
        let start = at;
        while bytes.get(at).is_some_and(u8::is_ascii_digit) {
            if at - start < 3 {
                millis += i64::from(bytes[at] - b'0') * [100, 10, 1][at - start];
            }
            at += 1;
        }
        if at == start {
            return None;
        }
    }
    let zone = match bytes.get(at..)? {
        b"Z" | b"z" => 0,
        [sign @ (b'+' | b'-'), _, _, b':', _, _] => {
            let (hours, minutes) = (number(at + 1, 2)?, number(at + 4, 2)?);
            if hours > 23 || minutes > 59 {
                return None;
            }
            (hours * 60 + minutes) * if *sign == b'+' { 1 } else { -1 }
        }
        _ => return None,
    };
    let adjusted = year - i64::from(month <= 2);
    let era = adjusted.div_euclid(400);
    let within = adjusted.rem_euclid(400);
    let march = if month > 2 { month - 3 } else { month + 9 };
    let days =
        era * 146097 + within * 365 + within / 4 - within / 100 + (153 * march + 2) / 5 + day
            - 1
            - 719468;
    Some((((days * 24 + hour) * 60 + minute - zone) * 60 + second) * 1000 + millis)
}

impl TranscriptLineMeta {
    fn entry(
        self,
        conv: &str,
        source: &str,
        generation: u64,
        offset: u64,
        fallback: i64,
    ) -> Option<UsageEntry> {
        if self.kind.as_deref() != Some("PLANNER_RESPONSE")
            || self.status.as_deref() != Some("DONE")
        {
            return None;
        }
        let non_cached = self.input_tokens.unwrap_or(0);
        let cached = self.cache_read_tokens.unwrap_or(0);
        let output = self.output_tokens.unwrap_or(0);
        if non_cached == 0 && cached == 0 && output == 0 {
            return None;
        }
        let key = self
            .step_index
            .map(|step| format!("ag:step:{conv}:{step}"))
            .unwrap_or_else(|| format!("ag:pos:{source}:{generation}:{offset}"));
        let raw_model = self.model.unwrap_or_default();
        let mut entry = UsageEntry {
            display_model: normalize_antigravity_model(&raw_model),
            raw_model,
            request_id: Some(key.clone()),
            identity_key: key,
            identity_origin: format!("antigravity:conv:{conv}"),
            identity_aliases: vec![],
            timestamp_ms: self
                .created_at
                .as_deref()
                .and_then(timestamp)
                .unwrap_or(fallback),
            provider: ProviderId::Antigravity,
            tokens: TokenVector {
                input_tokens: non_cached.saturating_add(cached),
                non_cached_input_tokens: non_cached,
                cache_read_tokens: cached,
                output_tokens: output,
                cache_creation_tokens: 0,
                reasoning_output_tokens: 0,
            },
            cost_micro_usd: None,
            cache_savings_micro_usd: None,
            price_rule_id: None,
            pricing_precision: PricingPrecision::Unpriced {
                reason: UnpricedReason::UnknownModel,
            },
            single_request_evidence: true,
        };
        entry.reprice();
        Some(entry)
    }
}

struct Frame<'a, R> {
    input: &'a mut R,
    prefix: &'a mut Prefix,
    ended: bool,
    utf8: Utf8,
}

#[derive(Default)]
struct Utf8 {
    partial: [u8; 4],
    used: usize,
    invalid: bool,
}

impl Utf8 {
    fn update(&mut self, mut chunk: &[u8]) {
        if self.invalid {
            return;
        }
        while self.used != 0 && !chunk.is_empty() {
            self.partial[self.used] = chunk[0];
            self.used += 1;
            chunk = &chunk[1..];
            match std::str::from_utf8(&self.partial[..self.used]) {
                Ok(_) => self.used = 0,
                Err(err) if err.error_len().is_some() => {
                    self.invalid = true;
                    return;
                }
                Err(_) => (),
            }
        }
        if let Err(err) = std::str::from_utf8(chunk) {
            self.invalid = err.error_len().is_some();
            if !self.invalid {
                let tail = &chunk[err.valid_up_to()..];
                self.used = tail.len();
                self.partial[..tail.len()].copy_from_slice(tail);
            }
        }
    }
}

impl<R: BufRead> Read for Frame<'_, R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if self.ended || out.is_empty() {
            return Ok(0);
        }
        let bytes = self.input.fill_buf()?;
        let chunk = &bytes[..bytes.len().min(out.len())];
        let lf = chunk.iter().position(|byte| *byte == b'\n');
        let count = lf.unwrap_or(chunk.len());
        let consumed = count + usize::from(lf.is_some());
        self.utf8.update(&chunk[..consumed]);
        out[..count].copy_from_slice(&chunk[..count]);
        self.prefix.update(&chunk[..consumed]);
        self.input.consume(consumed);
        self.ended = lf.is_some();
        Ok(count)
    }
}

fn parse_object(mut reader: impl BufRead) -> io::Result<Option<TranscriptLineMeta>> {
    loop {
        let bytes = reader.fill_buf()?;
        if bytes.is_empty() {
            return Ok(None);
        }
        let skip = bytes
            .iter()
            .take_while(|byte| matches!(**byte, b' ' | b'\t' | b'\r'))
            .count();
        let first = bytes.get(skip).copied();
        reader.consume(skip);
        if let Some(first) = first {
            if first != b'{' {
                return Ok(None);
            }
            return match serde_json::from_reader(reader) {
                Ok(meta) => Ok(Some(meta)),
                Err(err) if err.is_io() => Err(err.into()),
                Err(_) => Ok(None),
            };
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VerificationMode {
    Tail,
    Full,
}

#[derive(Debug)]
pub struct AntigravityScanBatch {
    pub entries: Vec<UsageEntry>,
    pub next_checkpoint: DualTrackCheckpoint,
    pub resume_state_json: String,
    pub rebased: bool,
    pub complete_without_gaps: bool,
    pub has_coverage_gap: bool,
    pub malformed_line_count: u64,
    pub oversized_line_count: u64,
    pub validation_bytes_read: u64,
    pub payload_bytes_read: u64,
    pub scanned_from_offset: u64,
}

/// Caches only the hash state at a block boundary. A cold long-prefix resume
/// audits the prefix once: the finalized SHA-256 in SQLite is not resumable.
#[derive(Default)]
pub struct AntigravityScanner {
    aligned: Option<Sha256>,
}

impl AntigravityScanner {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn scan(
        &mut self,
        roots: &AntigravityRoots,
        path: &Path,
        source_id: &str,
        previous: Option<&DualTrackCheckpoint>,
        mode: VerificationMode,
        now_ms: i64,
    ) -> io::Result<AntigravityScanBatch> {
        let (mut file, source) = open_log(&roots.brain_dir, path)?;
        let frozen_end_offset = source.size;
        let mut validation = 0;
        let mut generation = 0;
        let mut fallback = source.mtime_ms;
        let mut last_full = Some(now_ms);
        let mut restored = None;
        if let Some(cp) = previous {
            if cp.file_identity.as_ref() == Some(&source.identity)
                && cp.byte_offset <= frozen_end_offset
            {
                let (check, head, tail) = windows(&mut file, cp)?;
                validation += check.validation_bytes_read;
                if check.outcome == VerifyOutcome::Match {
                    last_full = cp.last_full_verify_ms;
                    if mode == VerificationMode::Tail {
                        if cp.byte_offset <= 2 * WINDOW as u64 {
                            let mut prefix = Prefix::default();
                            prefix.update(&head);
                            let overlap = head.len().saturating_sub(cp.tail_offset as usize);
                            prefix.update(&tail[overlap..]);
                            if prefix.hash() == cp.full_prefix_sha256 {
                                restored = Some(prefix);
                            }
                        } else if let Some(aligned) = self.aligned.clone() {
                            restored = Prefix::restore(cp, aligned, head, tail);
                        }
                    }
                    if restored.is_none()
                        && (mode == VerificationMode::Full || cp.byte_offset > 2 * WINDOW as u64)
                    {
                        let (check, prefix) = audit(&mut file, cp)?;
                        validation += check.validation_bytes_read;
                        if check.outcome == VerifyOutcome::Match {
                            restored = Some(prefix);
                            last_full = Some(now_ms);
                        }
                    }
                }
            }
            if restored.is_some() {
                generation = cp.generation;
                fallback = cp.fallback_timestamp_ms;
            } else {
                generation = cp
                    .generation
                    .checked_add(1)
                    .ok_or_else(|| io::Error::other("generation overflow"))?;
                last_full = Some(now_ms);
            }
        }
        let rebased = previous.is_some() && restored.is_none();
        let mut prefix = restored.unwrap_or_default();
        let start = prefix.len;
        file.seek(SeekFrom::Start(start))?;
        let mut stream = BufReader::new(file.take(frozen_end_offset - start));
        let mut entries = Vec::new();
        let (mut malformed, mut oversized) = (0, 0);
        let mut position = start;
        while position < frozen_end_offset {
            let mut tentative = prefix.clone();
            let mut frame = Frame {
                input: &mut stream,
                prefix: &mut tentative,
                ended: false,
                utf8: Utf8::default(),
            };
            let meta = parse_object(BufReader::new((&mut frame).take(MAX_LINE_BYTES as u64)))?;
            io::copy(&mut frame, &mut io::sink())?;
            let ended = frame.ended;
            let valid_utf8 = !frame.utf8.invalid && frame.utf8.used == 0;
            position = tentative.len;
            if !ended {
                if position != frozen_end_offset {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "source shortened during scan",
                    ));
                }
                if position - prefix.len > MAX_LINE_BYTES as u64 {
                    oversized += 1;
                }
                break;
            }
            if position - prefix.len - 1 > MAX_LINE_BYTES as u64 {
                oversized += 1;
            } else if let Some(meta) = meta.filter(|_| valid_utf8) {
                if let Some(entry) = meta.entry(
                    &source.conversation_id,
                    source_id,
                    generation,
                    position,
                    fallback,
                ) {
                    entries.push(entry);
                }
            } else {
                malformed += 1;
            }
            prefix = tentative;
        }
        let has_coverage_gap = malformed != 0 || oversized != 0;
        let next_checkpoint = prefix.checkpoint(generation, source.identity, fallback, last_full);
        self.aligned = Some(prefix.aligned);
        Ok(AntigravityScanBatch {
            entries,
            next_checkpoint,
            resume_state_json: "{}".into(),
            rebased,
            complete_without_gaps: !has_coverage_gap && prefix.len == frozen_end_offset,
            has_coverage_gap,
            malformed_line_count: malformed,
            oversized_line_count: oversized,
            validation_bytes_read: validation,
            payload_bytes_read: position - start,
            scanned_from_offset: start,
        })
    }
}

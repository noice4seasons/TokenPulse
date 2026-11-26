use crate::{
    accounting::{
        codex_usage_decision, timestamp_ms, CodexAccountingState, CodexUsageDecision, DecisionKind,
    },
    checkpoint::{verify_full_prefix, windows, DualTrackCheckpoint, Prefix, VerifyOutcome},
    paths::{describe, open_log, CodexRoots},
    rate_limits::{from_event, RateLimits},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    io::{self, Read, Seek, SeekFrom},
    path::Path,
};
use tokenpulse_core_domain::{
    models::{
        normalize_codex_model, PricingPrecision, ProviderId, TokenVector, UnpricedReason,
        UsageEntry,
    },
    quota::{CreditObservation, QuotaObservation},
};

pub const MAX_LINE_BYTES: usize = 16 * 1024 * 1024;
pub const FULL_VERIFY_INTERVAL_MS: i64 = 24 * 60 * 60 * 1000;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IdentitySeed {
    pub key: String,
    pub request_id: String,
    pub timestamp_ms: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IdentityLink {
    pub alias: String,
    pub target: String,
}
#[derive(Debug)]
pub struct ScanBatchOutput {
    pub entries: Vec<UsageEntry>,
    pub identity_seeds: Vec<IdentitySeed>,
    pub identity_links: Vec<IdentityLink>,
    pub quota_observations: Vec<QuotaObservation>,
    pub credit_observations: Vec<CreditObservation>,
    pub next_checkpoint: DualTrackCheckpoint,
    pub resume_state_json: String,
    pub rebased: bool,
    pub complete_without_gaps: bool,
    pub has_coverage_gap: bool,
    pub malformed_line_count: u64,
    pub oversized_line_count: u64,
    pub validation_bytes_read: u64,
    pub payload_bytes_read: u64,
    pub frozen_end_offset: u64,
    pub scanned_from_offset: u64,
    pub protected_prefix: u64,
    pub cancelled: bool,
}
impl ScanBatchOutput {
    /// Only full replays supply the entire execution set for destructive reconciliation.
    pub fn full_rebuild_coverage(&self) -> bool {
        self.complete_without_gaps && self.scanned_from_offset == 0 && self.protected_prefix == 0
    }
}
pub struct ScanOptions<'a> {
    /// Stable store identity (retained on a verified archive move).
    pub source_id: &'a str,
    pub now_ms: i64,
    pub checkpoint: Option<&'a DualTrackCheckpoint>,
    pub resume_state_json: Option<&'a str>,
    pub identity_source: Option<&'a dyn Fn(&str) -> Option<String>>,
    pub is_cancelled: Option<&'a dyn Fn() -> bool>,
    pub payload_budget: Option<u64>,
    /// Worker may explicitly request an audit; ordinary scans also audit when due.
    pub force_full_verify: bool,
    /// Emit a complete execution set even for an unchanged file.
    pub rebuild: bool,
}
impl<'a> ScanOptions<'a> {
    pub fn new(source_id: &'a str, now_ms: i64) -> Self {
        Self {
            source_id,
            now_ms,
            checkpoint: None,
            resume_state_json: None,
            identity_source: None,
            is_cancelled: None,
            payload_budget: None,
            force_full_verify: false,
            rebuild: false,
        }
    }
}
pub fn full_verify_due(checkpoint: &DualTrackCheckpoint, now_ms: i64) -> bool {
    checkpoint.last_full_verify_ms.map_or(true, |last| {
        now_ms < last || now_ms.saturating_sub(last) >= FULL_VERIFY_INTERVAL_MS
    })
}

#[derive(Serialize, Deserialize)]
struct Resume {
    accounting: CodexAccountingState,
    raw_model: String,
    last_time: Option<i64>,
    offset: u64,
    words: [u32; 8],
}
struct Batch<'a> {
    options: &'a ScanOptions<'a>,
    accounting: CodexAccountingState,
    model: String,
    last_time: Option<i64>,
    fallback: i64,
    protected: u64,
    replay: bool,
    local: HashSet<String>,
    indices: HashMap<String, usize>,
    entries: Vec<UsageEntry>,
    seeds: Vec<IdentitySeed>,
    links: Vec<IdentityLink>,
    rates: RateLimits,
    malformed: u64,
    oversized: u64,
}
impl Batch<'_> {
    fn line(&mut self, bytes: &[u8], end: u64) {
        let Ok(event) = serde_json::from_slice::<Value>(bytes) else {
            self.malformed += 1;
            return;
        };
        let p = &event["payload"];
        if !p.is_object() {
            return;
        }
        let time = event["timestamp"].as_str().and_then(timestamp_ms);
        if time.is_some() {
            self.last_time = time;
        }
        let source = self.options.source_id;
        let protected = end <= self.protected;
        let position = format!(
            "codex:position:{source}:{}:{end}",
            self.accounting.generation
        );
        let decision =
            codex_usage_decision(&mut self.accounting, &event, source, &position, |key| {
                if self.local.contains(key) {
                    return Some(source.into());
                }
                if protected {
                    return None;
                }
                self.options
                    .identity_source
                    .and_then(|lookup| lookup(key))
                    .filter(|s| !self.replay || s != source)
            });
        let kind = event["type"].as_str();
        if matches!(
            kind,
            Some("session_meta" | "turn_context" | "token_usage_record")
        ) || kind == Some("event_msg")
            && matches!(p["type"].as_str(), Some("task_started" | "token_count"))
        {
            if let Some(m) = p["model"]
                .as_str()
                .or_else(|| p["info"]["model"].as_str())
                .or_else(|| p["model_slug"].as_str())
                .or_else(|| p["settings"]["model"].as_str())
                .or_else(|| p["collaboration_mode"]["settings"]["model"].as_str())
            {
                self.model = m.into();
            }
        }
        let scope = if self.accounting.session_id.is_empty() {
            source
        } else {
            &self.accounting.session_id
        };
        self.rates
            .merge(from_event(&event, bytes, scope, self.options.now_ms));
        if let Some(d) = decision {
            let timestamp = self
                .last_time
                .or((self.accounting.created_ms != 0).then_some(self.accounting.created_ms))
                .unwrap_or(self.fallback);
            self.record(d, timestamp, protected);
        }
    }
    fn record(&mut self, d: CodexUsageDecision, timestamp: i64, protected: bool) {
        let aliases: Vec<String> = d
            .observation_key
            .into_iter()
            .chain(d.response_key)
            .collect();
        if d.kind == DecisionKind::Alias {
            for alias in aliases.into_iter().filter(|a| a != &d.identity_key) {
                self.links.push(IdentityLink {
                    alias: alias.clone(),
                    target: d.identity_key.clone(),
                });
                self.local.insert(alias);
            }
            return;
        }
        let [input, cache, output, reasoning] = d.usage;
        for key in std::iter::once(&d.identity_key).chain(aliases.iter()) {
            self.local.insert(key.clone());
            if protected {
                self.seeds.push(IdentitySeed {
                    key: key.clone(),
                    request_id: d.request_id.clone(),
                    timestamp_ms: timestamp,
                });
            }
        }
        if protected {
            return;
        }
        let tokens = TokenVector {
            input_tokens: input,
            non_cached_input_tokens: input - cache,
            cache_read_tokens: cache,
            cache_creation_tokens: 0,
            output_tokens: output,
            reasoning_output_tokens: reasoning,
        };
        let mut entry = UsageEntry {
            raw_model: self.model.clone(),
            display_model: normalize_codex_model(&self.model),
            request_id: Some(d.request_id),
            identity_key: d.identity_key,
            identity_origin: d.identity_origin,
            identity_aliases: aliases,
            timestamp_ms: timestamp,
            provider: ProviderId::Codex,
            tokens,
            cost_micro_usd: None,
            cache_savings_micro_usd: None,
            price_rule_id: None,
            pricing_precision: PricingPrecision::Unpriced {
                reason: UnpricedReason::UnknownModel,
            },
            single_request_evidence: d.single_request_evidence,
        };
        if let Some(&i) = self.indices.get(&entry.identity_key) {
            let old = &self.entries[i];
            if old.identity_origin != entry.identity_origin
                || old.tokens.input_tokens > input
                || old.tokens.output_tokens > output
            {
                return;
            }
            entry.timestamp_ms = old.timestamp_ms;
            for alias in &old.identity_aliases {
                if !entry.identity_aliases.contains(alias) {
                    entry.identity_aliases.push(alias.clone());
                }
            }
            entry.reprice();
            self.entries[i] = entry;
        } else {
            entry.reprice();
            self.indices
                .insert(entry.identity_key.clone(), self.entries.len());
            self.entries.push(entry);
        }
    }
}

/// Read exactly one discovered file. Persist checkpoint, resume state, identities
/// and entries together. Schedule idle sources using full_verify_due on a worker.
pub fn scan_file(
    roots: &CodexRoots,
    path: &Path,
    options: &ScanOptions<'_>,
) -> io::Result<ScanBatchOutput> {
    let mut file = open_log(roots, path)?;
    let meta = describe(&file, path.into())?;
    let frozen = meta.size;
    let mut generation = 0;
    let mut fallback = meta.mtime_ms;
    let mut validated = 0;
    let mut rebased = false;
    let mut protected = 0;
    let mut last_full = Some(options.now_ms);
    let mut prefix = Prefix::default();
    let mut state = None;
    if let Some(cp) = options.checkpoint {
        let (result, head, tail) = windows(&mut file, cp)?;
        validated += result.validation_bytes_read;
        let mut matches = result.outcome == VerifyOutcome::Match && cp.byte_offset <= frozen;
        let recovered = options
            .resume_state_json
            .and_then(|s| serde_json::from_str::<Resume>(s).ok())
            .filter(|r| {
                r.accounting.version == 4
                    && r.accounting.generation == cp.generation
                    && r.offset == cp.byte_offset
                    && r.accounting.recent.len() <= 128
            })
            .and_then(|r| Prefix::restore(cp, r.words, head, tail).map(|p| (r, p)));
        let audit = options.force_full_verify
            || options.rebuild
            || full_verify_due(cp, options.now_ms)
            || cp.file_identity.as_ref() != Some(&meta.identity)
            || recovered.is_none();
        if matches && audit {
            let result = verify_full_prefix(&mut file, cp)?;
            validated += result.validation_bytes_read;
            matches = result.outcome == VerifyOutcome::Match;
        }
        if matches {
            generation = cp.generation;
            fallback = cp.fallback_timestamp_ms;
            if !audit {
                last_full = cp.last_full_verify_ms;
            }
            if !options.rebuild {
                if let Some((r, p)) = recovered {
                    state = Some(r);
                    prefix = p;
                } else {
                    protected = cp.byte_offset;
                }
            }
        } else {
            generation = cp
                .generation
                .checked_add(1)
                .ok_or_else(|| io::Error::other("generation overflow"))?;
            rebased = true;
        }
    }
    let start = prefix.len;
    let (accounting, model, last_time) = state
        .map(|r| (r.accounting, r.raw_model, r.last_time))
        .unwrap_or_else(|| (CodexAccountingState::new(generation), String::new(), None));
    let mut batch = Batch {
        options,
        accounting,
        model,
        last_time,
        fallback,
        protected,
        replay: rebased || options.rebuild,
        local: HashSet::new(),
        indices: HashMap::new(),
        entries: vec![],
        seeds: vec![],
        links: vec![],
        rates: RateLimits::default(),
        malformed: 0,
        oversized: 0,
    };
    file.seek(SeekFrom::Start(start))?;
    let limit = options
        .payload_budget
        .unwrap_or(u64::MAX)
        .min(frozen - start);
    let mut reader = file.take(limit);
    let mut buffer = [0u8; 32 * 1024];
    let mut line = Vec::new();
    let mut oversized = false;
    let mut payload = 0;
    let mut committed = prefix.clone();
    let cancelled = loop {
        if options.is_cancelled.is_some_and(|f| f()) {
            break true;
        }
        let n = reader.read(&mut buffer)?;
        if n == 0 {
            break false;
        }
        let bytes = &buffer[..n];
        let mut last_lf = 0;
        for (i, &ch) in bytes.iter().enumerate() {
            if ch != b'\n' {
                continue;
            }
            let part = &bytes[last_lf..i];
            if oversized || part.len() > MAX_LINE_BYTES - line.len() {
                batch.oversized += 1;
            } else {
                line.extend_from_slice(part);
                batch.line(&line, start + payload + i as u64 + 1);
            }
            line.clear();
            oversized = false;
            last_lf = i + 1;
        }
        if last_lf > 0 {
            prefix.update(&bytes[..last_lf]);
            committed.clone_from(&prefix);
        }
        let rest = &bytes[last_lf..];
        prefix.update(rest);
        if !oversized {
            if rest.len() > MAX_LINE_BYTES - line.len() {
                oversized = true;
                line.clear();
            } else {
                line.extend_from_slice(rest);
            }
        }
        payload += n as u64;
    };
    if committed.len < protected {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "protected prefix replay incomplete",
        ));
    }
    let gap = batch.malformed != 0 || batch.oversized != 0;
    let complete = !cancelled && !gap && committed.len == frozen;
    let checkpoint = committed.checkpoint(generation, meta.identity, fallback, last_full);
    let resume = Resume {
        accounting: batch.accounting,
        raw_model: batch.model,
        last_time: batch.last_time,
        offset: committed.len,
        words: committed.words,
    };
    Ok(ScanBatchOutput {
        entries: batch.entries,
        identity_seeds: batch.seeds,
        identity_links: batch.links,
        quota_observations: batch.rates.quota_observations,
        credit_observations: batch.rates.credit_observations,
        next_checkpoint: checkpoint,
        resume_state_json: serde_json::to_string(&resume)?,
        rebased,
        complete_without_gaps: complete,
        has_coverage_gap: gap,
        malformed_line_count: batch.malformed,
        oversized_line_count: batch.oversized,
        validation_bytes_read: validated,
        payload_bytes_read: payload,
        frozen_end_offset: frozen,
        scanned_from_offset: start,
        protected_prefix: protected,
        cancelled,
    })
}

use crate::{provider, schema, timezone::*, Result, StoreError};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{mpsc, Arc},
    thread,
};
use tokenpulse_core_domain::{models::*, quota::*};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExpectedVersion {
    pub expected_generation: Option<u64>,
    pub expected_offset: Option<u64>,
    pub expected_version_token: Option<String>,
}
/// Byte windows describe only their location, length and SHA-256 digest.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DigestWindow {
    pub offset: u64,
    pub length: u64,
    pub sha256: [u8; 32],
}

#[derive(Default, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Checkpoint {
    pub version: u32,
    pub head: Option<DigestWindow>,
    pub tail: Option<DigestWindow>,
    pub blocks: Vec<DigestWindow>,
    pub next_verification_block: u64,
}

#[derive(Default, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ResumeState {
    pub version: u32,
    pub segment: u64,
    #[serde(deserialize_with = "counter_state")]
    pub token_usage_record: Option<TokenVector>,
    #[serde(deserialize_with = "counter_state")]
    pub event_msg_token_count: Option<TokenVector>,
    #[serde(deserialize_with = "counter_state")]
    pub alignment_offset: Option<TokenVector>,
    pub recent_observations: Vec<[u8; 32]>,
    pub skipping_overlong_line: bool,
}

// The domain type tolerates extra fields on the wire. At this persistence
// boundary every field in the original (verbatim) JSON must be accounted for.
fn counter_state<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<TokenVector>, D::Error> {
    use serde::{de::Error, Deserialize};
    let Some(value) = Option::<serde_json::Value>::deserialize(d)? else {
        return Ok(None);
    };
    let counters: TokenVector = serde_json::from_value(value.clone()).map_err(D::Error::custom)?;
    if serde_json::to_value(counters).map_err(D::Error::custom)? != value {
        return Err(D::Error::custom("unexpected counter fields"));
    }
    Ok(Some(counters))
}

#[allow(dead_code)]
#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CodexFileIdentity {
    volume_serial: Option<u32>,
    file_index: Option<u64>,
    creation_time_100ns: Option<u64>,
}

#[allow(dead_code)]
#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CodexDualTrackCheckpoint {
    byte_offset: u64,
    generation: u64,
    head_len: u32,
    head_sha256: String,
    tail_offset: u64,
    tail_len: u32,
    tail_sha256: String,
    full_prefix_sha256: String,
    file_identity: Option<CodexFileIdentity>,
    fallback_timestamp_ms: i64,
    last_full_verify_ms: Option<i64>,
}

#[allow(dead_code)]
#[derive(Debug, serde::Deserialize)]
#[serde(untagged)]
enum ValidatedCheckpoint {
    Generic(Checkpoint),
    CodexDualTrack(CodexDualTrackCheckpoint),
}

#[allow(dead_code)]
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CodexAccountingStateSchema {
    version: u32,
    notification_offset: Option<[u64; 4]>,
    notification_raw_total: Option<[u64; 4]>,
    notification_turn_id: String,
    detailed_total: Option<[u64; 4]>,
    detailed_turn_total: Option<[u64; 4]>,
    detailed_turn_id: String,
    generation: u64,
    last_entry_id: Option<String>,
    notification_total: Option<[u64; 4]>,
    session_id: String,
    created_ms: i64,
    forked: bool,
    turn_id: String,
    inherited: bool,
    mode: String,
    total: Option<[u64; 4]>,
    segment: u64,
    fresh_output: bool,
    recent: Vec<String>,
}

#[allow(dead_code)]
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CodexResumeStateSchema {
    accounting: CodexAccountingStateSchema,
    raw_model: String,
    last_time: Option<i64>,
    offset: u64,
    words: [u32; 8],
}

#[allow(dead_code)]
#[derive(Debug, serde::Deserialize)]
#[serde(untagged)]
enum ValidatedResumeState {
    Generic(ResumeState),
    Codex(CodexResumeStateSchema),
}

/// JSON fields retain the scanner's exact encoding of Checkpoint and ResumeState.
#[derive(Clone, Debug)]
pub struct SourceState {
    pub source_id: String,
    pub provider: ProviderId,
    pub current_path_hash: String,
    pub volume_serial: String,
    pub file_index: String,
    pub creation_time_100ns: Option<i64>,
    pub generation: u64,
    pub committed_offset: u64,
    pub version_token: String,
    pub checkpoint_json: String,
    pub resume_state_json: String,
    pub sealed_before_ms: Option<i64>,
    pub updated_at_ms: i64,
}
impl SourceState {
    pub fn expected_version(&self) -> ExpectedVersion {
        ExpectedVersion {
            expected_generation: Some(self.generation),
            expected_offset: Some(self.committed_offset),
            expected_version_token: Some(self.version_token.clone()),
        }
    }
}
#[derive(Clone, Debug)]
pub enum CommitMode {
    Tail,
    Rebuild {
        complete_without_gaps: bool,
        seen_request_ids: HashSet<String>,
    },
}
#[derive(Clone, Debug)]
pub struct CommitBatch {
    pub expected: ExpectedVersion,
    pub source: SourceState,
    pub mode: CommitMode,
    pub entries: Vec<UsageEntry>,
    pub identity_links: Vec<(String, String)>,
    pub identity_seeds: Vec<(String, String, i64)>,
    pub quotas: Vec<QuotaObservation>,
    pub credits: Vec<CreditObservation>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommitCounts {
    pub unchanged: usize,
    pub inserted: usize,
    pub updated: usize,
    pub removed: usize,
    pub skipped: usize,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommitOutcome {
    Committed(CommitCounts),
    StaleConflict,
}

type Job = Box<dyn FnOnce(&mut Connection) + Send>;
struct Worker {
    queue: Option<mpsc::Sender<Job>>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Drop for Worker {
    fn drop(&mut self) {
        // Release the connection and WAL handles before the last owner returns.
        drop(self.queue.take());
        if let Some(worker) = self.thread.take() {
            let _ = worker.join();
        }
    }
}
#[derive(Clone)]
pub struct StoreHandle {
    worker: Arc<Worker>,
    pub(crate) path: Option<PathBuf>,
    pub(crate) cancel: Option<crate::CancelToken>,
}
impl StoreHandle {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        if path.as_ref() == Path::new(":memory:") {
            return Self::open_in_memory();
        }
        Self::start(Some(std::env::current_dir()?.join(path)))
    }
    pub fn open_in_memory() -> Result<Self> {
        Self::start(None)
    }
    fn start(path: Option<PathBuf>) -> Result<Self> {
        let (queue, jobs) = mpsc::channel::<Job>();
        let (started, ready) = mpsc::sync_channel(1);
        let file = path.clone();
        let thread = thread::Builder::new()
            .name("sqlite-writer".into())
            .spawn(move || {
                let opened = (|| -> Result<Connection> {
                    let mut db = match &file {
                        Some(p) => Connection::open(p)?,
                        None => Connection::open_in_memory()?,
                    };
                    schema::initialize(&mut db, file.is_some())?;
                    Ok(db)
                })();
                match opened {
                    Ok(mut db) => {
                        if started.send(Ok(())).is_ok() {
                            for job in jobs {
                                job(&mut db)
                            }
                        }
                    }
                    Err(e) => {
                        let _ = started.send(Err(e));
                    }
                }
            })?;
        ready.recv().map_err(|_| StoreError::Stopped)??;
        Ok(Self {
            worker: Arc::new(Worker {
                queue: Some(queue),
                thread: Some(thread),
            }),
            path,
            cancel: None,
        })
    }
    pub(crate) fn queued<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let (send, recv) = mpsc::sync_channel(1);
        self.worker
            .queue
            .as_ref()
            .ok_or(StoreError::Stopped)?
            .send(Box::new(move |db| {
                let _ = send.send(f(db));
            }))
            .map_err(|_| StoreError::Stopped)?;
        recv.recv().map_err(|_| StoreError::Stopped)?
    }
    pub(crate) fn read<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        if let Some(p) = &self.path {
            let mut db = Connection::open_with_flags(p, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
            db.execute_batch("PRAGMA busy_timeout=5000; PRAGMA foreign_keys=ON")?;
            f(&mut db)
        } else {
            self.queued(f)
        }
    }
    pub fn commit(&self, batch: CommitBatch) -> Result<CommitOutcome> {
        self.queued(move |db| commit(db, batch))
    }
    pub fn source_state(&self, id: &str) -> Result<Option<SourceState>> {
        let id = id.to_owned();
        self.read(move |db| load_source(db, &id))
    }
    pub fn identity_source(&self, p: ProviderId, key: &str) -> Result<Option<String>> {
        let key = key.to_owned();
        self.read(move |db| {
            Ok(db
                .query_row(
                    "SELECT source_id FROM usage_identity WHERE provider=? AND identity_key=?",
                    params![provider(p), key],
                    |r| r.get(0),
                )
                .optional()?)
        })
    }
    pub fn update_source_path_alias(
        &self,
        id: &str,
        volume: &str,
        index: &str,
        new_hash: &str,
    ) -> Result<bool> {
        if volume.is_empty() || index.is_empty() {
            return Ok(false);
        }
        let args = [new_hash, id, volume, index].map(str::to_owned);
        self.queued(move|db|Ok(db.execute("UPDATE usage_source SET current_path_hash=?1 WHERE source_id=?2 AND volume_serial=?3 AND file_index=?4",rusqlite::params_from_iter(&args))?==1))
    }
    pub fn compact(&self, now_ms: i64, detail_retention_days: u32) -> Result<usize> {
        self.queued(move |db| {
            let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let cutoff = now_ms
                .saturating_sub(i64::from(detail_retention_days) * DAY_MS)
                .max(schema::retained(&tx)?);
            let n = tx.execute("DELETE FROM usage_entry WHERE timestamp_ms<?", [cutoff])?;
            tx.execute(
                "UPDATE schema_meta SET value=? WHERE key='detail_retained_from_ms'",
                [cutoff.to_string()],
            )?;
            tx.commit()?;
            Ok(n)
        })
    }
}
fn load_source(db: &Connection, id: &str) -> Result<Option<SourceState>> {
    let sql="SELECT source_id,provider,current_path_hash,volume_serial,file_index,creation_time_100ns,generation,committed_offset,version_token,checkpoint_json,resume_state_json,sealed_before_ms,updated_at_ms FROM usage_source WHERE source_id=?";
    Ok(db
        .query_row(sql, [id], |r| {
            Ok(SourceState {
                source_id: r.get(0)?,
                provider: crate::parse_provider(&r.get::<_, String>(1)?)?,
                current_path_hash: r.get(2)?,
                volume_serial: r.get(3)?,
                file_index: r.get(4)?,
                creation_time_100ns: r.get(5)?,
                generation: r.get(6)?,
                committed_offset: r.get(7)?,
                version_token: r.get(8)?,
                checkpoint_json: r.get(9)?,
                resume_state_json: r.get(10)?,
                sealed_before_ms: r.get(11)?,
                updated_at_ms: r.get(12)?,
            })
        })
        .optional()?)
}
fn commit(db: &mut Connection, b: CommitBatch) -> Result<CommitOutcome> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let s = &b.source;
    let before = load_source(&tx, &s.source_id)?;
    if before
        .as_ref()
        .map(SourceState::expected_version)
        .unwrap_or_default()
        != b.expected
        || before.as_ref().is_some_and(|v| {
            s.generation < v.generation
                || (s.generation == v.generation && s.committed_offset < v.committed_offset)
        })
    {
        return Ok(CommitOutcome::StaleConflict);
    }
    if before.as_ref().is_some_and(|v| v.provider != s.provider) {
        return Err(StoreError::Invalid("source provider changed"));
    }
    // A rescan of the same generation and offset must issue a fresh revision
    // token. Otherwise a second writer with the old snapshot could still win.
    if before.is_some() && s.expected_version() == b.expected {
        return Ok(CommitOutcome::StaleConflict);
    }
    let _ = serde_json::from_str::<ValidatedCheckpoint>(&s.checkpoint_json)?;
    if let ValidatedResumeState::Codex(ref rs) =
        serde_json::from_str::<ValidatedResumeState>(&s.resume_state_json)?
    {
        if rs.accounting.recent.len() > 128 {
            return Err(StoreError::Invalid("codex recent observations exceed bound"));
        }
    }
    tx.execute("INSERT INTO usage_source(source_id,provider,current_path_hash,volume_serial,file_index,creation_time_100ns,generation,committed_offset,version_token,checkpoint_json,resume_state_json,sealed_before_ms,updated_at_ms) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT(source_id) DO UPDATE SET
      current_path_hash=CASE WHEN usage_source.volume_serial=excluded.volume_serial AND usage_source.file_index=excluded.file_index THEN usage_source.current_path_hash ELSE excluded.current_path_hash END,
      volume_serial=excluded.volume_serial,file_index=excluded.file_index,creation_time_100ns=excluded.creation_time_100ns,
      generation=excluded.generation,committed_offset=excluded.committed_offset,version_token=excluded.version_token,
      checkpoint_json=excluded.checkpoint_json,resume_state_json=excluded.resume_state_json,sealed_before_ms=excluded.sealed_before_ms,updated_at_ms=excluded.updated_at_ms",
      params![s.source_id,provider(s.provider),s.current_path_hash,s.volume_serial,s.file_index,s.creation_time_100ns,s.generation,s.committed_offset,s.version_token,s.checkpoint_json,s.resume_state_json,s.sealed_before_ms,s.updated_at_ms])?;
    let cutoff = schema::retained(&tx)?;
    let mut counts = CommitCounts::default();
    let tail = matches!(&b.mode, CommitMode::Tail);
    if let CommitMode::Rebuild {
        complete_without_gaps,
        ..
    } = &b.mode
    {
        tx.execute(
            "UPDATE usage_source SET scan_incomplete=? WHERE source_id=?",
            params![!*complete_without_gaps, s.source_id],
        )?;
    }
    let mut seen = match b.mode {
        CommitMode::Rebuild {
            complete_without_gaps: true,
            seen_request_ids,
        } => Some(seen_request_ids),
        _ => None,
    };
    for e in b.entries {
        let incoming = e.request_id.as_deref().unwrap_or(&e.identity_key);
        if let Some(proof) = &seen {
            if !proof.contains(incoming) {
                return Err(StoreError::Invalid("entry missing from rebuild proof"));
            }
        }
        if let Some(id) = apply(&tx, s, e, cutoff, tail, &mut counts)? {
            if let Some(proof) = &mut seen {
                proof.insert(id);
            }
        }
    }
    if let Some(proof) = seen {
        let mut q = tx.prepare("SELECT request_id FROM usage_entry WHERE source_id=?")?;
        let ids = q
            .query_map([&s.source_id], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for id in ids.into_iter().filter(|id| !proof.contains(id)) {
            let old = load_entry(&tx, s, &id)?.ok_or(StoreError::Invalid("missing detail"))?;
            adjust(&tx, s, &old, false)?;
            tx.execute(
                "DELETE FROM usage_entry WHERE source_id=? AND request_id=?",
                params![s.source_id, id],
            )?;
            tx.execute(
                "DELETE FROM usage_identity WHERE provider=? AND source_id=? AND request_id=?",
                params![provider(s.provider), s.source_id, id],
            )?;
            counts.removed += 1;
        }
    }
    for (key, request_id, timestamp_ms) in b.identity_seeds {
        tx.execute(
            "INSERT OR IGNORE INTO usage_identity(provider,identity_key,source_id,request_id,origin_id,timestamp_ms) VALUES(?,?,?,?,?,?)",
            params![provider(s.provider), key, s.source_id, request_id, "", timestamp_ms],
        )?;
    }
    for (alias, target) in b.identity_links {
        if let Some((owner_source, request_id, origin_id, ts)) = tx
            .query_row(
                "SELECT source_id,request_id,origin_id,timestamp_ms FROM usage_identity WHERE provider=? AND identity_key=?",
                params![provider(s.provider), target],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, i64>(3)?,
                    ))
                },
            )
            .optional()?
        {
            if owner_source == s.source_id {
                tx.execute(
                    "INSERT OR IGNORE INTO usage_identity(provider,identity_key,source_id,request_id,origin_id,timestamp_ms) VALUES(?,?,?,?,?,?)",
                    params![provider(s.provider), alias, owner_source, request_id, origin_id, ts],
                )?;
            }
        }
    }
    for q in b.quotas {
        let effective = q
            .event_time_ms
            .filter(|_| q.event_time_trusted)
            .unwrap_or(q.ingested_at_ms);
        tx.execute(
            "INSERT INTO quota_observation VALUES(?,?,?,?,?,?,?,?) ON CONFLICT DO NOTHING",
            params![
                provider(q.provider),
                q.scope_id,
                q.target_id,
                serde_json::to_string(&q.period_kind)?,
                effective,
                q.event_time_ms,
                q.ingested_at_ms,
                serde_json::to_string(&q)?
            ],
        )?;
    }
    for c in b.credits {
        tx.execute(
            "INSERT INTO credit_observation VALUES(?,?,?,?,?,?,?,?,?) ON CONFLICT DO NOTHING",
            params![
                provider(c.provider),
                c.scope_id,
                c.event_time_ms.unwrap_or(c.ingested_at_ms),
                c.event_time_ms,
                c.ingested_at_ms,
                c.plan_type,
                c.service_tier,
                serde_json::to_string(&c.state)?,
                c.account_confirmed
            ],
        )?;
    }
    tx.commit()?;
    Ok(CommitOutcome::Committed(counts))
}

fn load_entry(db: &Connection, s: &SourceState, id: &str) -> Result<Option<UsageEntry>> {
    let sql="SELECT identity_key,identity_origin,timestamp_ms,raw_model,display_model,input_tokens,non_cached_input_tokens,cache_read_tokens,cache_creation_tokens,output_tokens,reasoning_output_tokens,cost_micro_usd,cache_savings_micro_usd,price_rule_id,pricing_precision_json,single_request_evidence FROM usage_entry WHERE source_id=? AND request_id=?";
    let row = db
        .query_row(sql, params![s.source_id, id], |r| {
            let json: String = r.get(14)?;
            let precision = serde_json::from_str(&json).map_err(crate::json_column)?;
            Ok(UsageEntry {
                request_id: Some(id.into()),
                identity_aliases: vec![],
                provider: s.provider,
                identity_key: r.get(0)?,
                identity_origin: r.get(1)?,
                timestamp_ms: r.get(2)?,
                raw_model: r.get(3)?,
                display_model: r.get(4)?,
                tokens: TokenVector {
                    input_tokens: r.get(5)?,
                    non_cached_input_tokens: r.get(6)?,
                    cache_read_tokens: r.get(7)?,
                    cache_creation_tokens: r.get(8)?,
                    output_tokens: r.get(9)?,
                    reasoning_output_tokens: r.get(10)?,
                },
                cost_micro_usd: r.get(11)?,
                cache_savings_micro_usd: r.get(12)?,
                price_rule_id: r.get(13)?,
                pricing_precision: precision,
                single_request_evidence: r.get(15)?,
            })
        })
        .optional()?;
    Ok(row)
}
fn apply(
    db: &Connection,
    s: &SourceState,
    mut e: UsageEntry,
    cutoff: i64,
    tail: bool,
    counts: &mut CommitCounts,
) -> Result<Option<String>> {
    if s.provider != e.provider
        || e.identity_key.is_empty()
        || e.request_id.as_deref() == Some("")
        || e.identity_aliases.iter().any(String::is_empty)
    {
        return Err(StoreError::Invalid("invalid entry identity"));
    }
    let p = provider(e.provider);
    let aliases = std::mem::take(&mut e.identity_aliases);
    let keys = || std::iter::once(&e.identity_key).chain(aliases.iter());
    let mut owner = None;
    for key in keys() {
        let known:Option<(String,String)>=db.query_row("SELECT source_id,request_id FROM usage_identity WHERE provider=? AND identity_key=?",params![p,key],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        if let Some((source, id)) = known {
            if source != s.source_id {
                counts.skipped += 1;
                return Ok(None);
            }
            if owner.as_ref().is_some_and(|v| v != &id) {
                return Err(StoreError::Invalid("ambiguous identity aliases"));
            }
            owner = Some(id);
        }
    }
    let id = owner
        .clone()
        .or_else(|| e.request_id.clone())
        .unwrap_or_else(|| e.identity_key.clone());
    let old = load_entry(db, s, &id)?;
    // Compaction deliberately leaves the identity index intact. Check the
    // request owner too, in case a later parser introduces a new canonical key.
    let compacted = old.is_none() && (owner.is_some() || db.query_row(
        "SELECT EXISTS(SELECT 1 FROM usage_identity WHERE provider=? AND source_id=? AND request_id=?)",
        params![p,s.source_id,id], |r| r.get::<_, bool>(0)
    )?);
    if compacted {
        if owner.is_some() {
            for key in keys() {
                db.execute(
                    "INSERT INTO usage_identity VALUES(?,?,?,?,?,?) ON CONFLICT DO NOTHING",
                    params![p, key, s.source_id, id, e.identity_origin, e.timestamp_ms],
                )?;
            }
        }
        counts.skipped += 1;
        return Ok(Some(id));
    }
    if let Some(v) = &old {
        if v.identity_origin != e.identity_origin
            || (owner.is_none() && v.identity_key != e.identity_key)
        {
            return Err(StoreError::Invalid("request identity collision"));
        }
    }
    for key in keys() {
        db.execute(
            "INSERT INTO usage_identity VALUES(?,?,?,?,?,?) ON CONFLICT DO NOTHING",
            params![p, key, s.source_id, id, e.identity_origin, e.timestamp_ms],
        )?;
    }
    e.request_id = Some(id.clone());
    if old.as_ref() == Some(&e) {
        counts.unchanged += 1;
        return Ok(Some(id));
    }
    if let Some(v) = &old {
        if tail
            && ((v.single_request_evidence && !e.single_request_evidence)
                || (v.cost_micro_usd.is_some() && e.cost_micro_usd.is_none())
                || e.tokens.total_tokens() < v.tokens.total_tokens())
        {
            counts.skipped += 1;
            return Ok(Some(id));
        }
        adjust(db, s, v, false)?;
        counts.updated += 1;
    } else {
        counts.inserted += 1;
    }
    let t = e.tokens;
    db.execute(
        "INSERT OR REPLACE INTO usage_entry VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        params![
            s.source_id,
            id,
            e.identity_key,
            e.identity_origin,
            e.timestamp_ms,
            p,
            e.raw_model,
            e.display_model,
            t.input_tokens,
            t.non_cached_input_tokens,
            t.cache_read_tokens,
            t.cache_creation_tokens,
            t.output_tokens,
            t.reasoning_output_tokens,
            e.cost_micro_usd,
            e.cache_savings_micro_usd,
            e.price_rule_id,
            serde_json::to_string(&e.pricing_precision)?,
            e.single_request_evidence
        ],
    )?;
    adjust(db, s, &e, true)?;
    if e.timestamp_ms < cutoff {
        db.execute(
            "DELETE FROM usage_entry WHERE source_id=? AND request_id=?",
            params![s.source_id, id],
        )?;
    }
    Ok(Some(id))
}
fn adjust(db: &Connection, s: &SourceState, e: &UsageEntry, add: bool) -> Result<()> {
    let m = UsageMetrics::from_entry(e).ok_or(StoreError::Invalid(
        "inconsistent entry pricing or token overflow",
    ))?;
    let ts = e.timestamp_ms;
    let floor = |width| {
        ts.checked_sub(ts.rem_euclid(width))
            .ok_or(StoreError::Invalid("bucket start overflow"))
    };
    for (kind, start) in [
        ("hour", floor(HOUR_MS)?),
        ("day", floor(DAY_MS)?),
        ("month", month_start(ts)?),
    ] {
        let args = params![
            s.source_id,
            provider(e.provider),
            e.display_model,
            kind,
            start,
            m.request_count,
            m.input_tokens,
            m.non_cached_input_tokens,
            m.cache_read_tokens,
            m.cache_creation_tokens,
            m.output_tokens,
            m.reasoning_output_tokens,
            m.total_tokens,
            m.priced_request_count,
            m.unpriced_request_count,
            m.priced_tokens,
            m.cost_micro_usd,
            m.cache_savings_micro_usd
        ];
        if add {
            db.execute("INSERT INTO usage_bucket VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT DO UPDATE SET
                request_count=request_count+excluded.request_count,
                input_tokens=input_tokens+excluded.input_tokens,
                non_cached_input_tokens=non_cached_input_tokens+excluded.non_cached_input_tokens,
                cache_read_tokens=cache_read_tokens+excluded.cache_read_tokens,
                cache_creation_tokens=cache_creation_tokens+excluded.cache_creation_tokens,
                output_tokens=output_tokens+excluded.output_tokens,
                reasoning_output_tokens=reasoning_output_tokens+excluded.reasoning_output_tokens,
                total_tokens=total_tokens+excluded.total_tokens,
                priced_request_count=priced_request_count+excluded.priced_request_count,
                unpriced_request_count=unpriced_request_count+excluded.unpriced_request_count,
                priced_tokens=priced_tokens+excluded.priced_tokens,
                cost_micro_usd=cost_micro_usd+excluded.cost_micro_usd,
                cache_savings_micro_usd=cache_savings_micro_usd+excluded.cache_savings_micro_usd",args)?;
        } else {
            let changed=db.execute("UPDATE usage_bucket SET
                request_count=request_count-?6,
                input_tokens=input_tokens-?7,
                non_cached_input_tokens=non_cached_input_tokens-?8,
                cache_read_tokens=cache_read_tokens-?9,
                cache_creation_tokens=cache_creation_tokens-?10,
                output_tokens=output_tokens-?11,
                reasoning_output_tokens=reasoning_output_tokens-?12,
                total_tokens=total_tokens-?13,
                priced_request_count=priced_request_count-?14,
                unpriced_request_count=unpriced_request_count-?15,
                priced_tokens=priced_tokens-?16,
                cost_micro_usd=cost_micro_usd-?17,
                cache_savings_micro_usd=cache_savings_micro_usd-?18
                WHERE source_id=?1 AND provider=?2 AND display_model=?3 AND bucket_kind=?4 AND bucket_start_ms=?5",args)?;
            if changed != 1 {
                return Err(StoreError::Invalid("missing bucket"));
            }
            db.execute("DELETE FROM usage_bucket WHERE source_id=? AND provider=? AND display_model=? AND bucket_kind=? AND bucket_start_ms=? AND request_count=0",params![s.source_id,provider(e.provider),e.display_model,kind,start])?;
        }
    }
    Ok(())
}

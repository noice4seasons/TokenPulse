use crate::{Result, StoreError};
use rusqlite::Connection;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS schema_meta(key TEXT PRIMARY KEY,value TEXT NOT NULL) STRICT;
INSERT OR IGNORE INTO schema_meta VALUES ('schema_version','1'),('detail_retained_from_ms','-9223372036854775808');
CREATE TABLE IF NOT EXISTS usage_source(
 source_id TEXT PRIMARY KEY, provider TEXT NOT NULL, current_path_hash TEXT NOT NULL,
 volume_serial TEXT NOT NULL, file_index TEXT NOT NULL, creation_time_100ns INTEGER,
 generation INTEGER NOT NULL, committed_offset INTEGER NOT NULL, version_token TEXT NOT NULL,
 checkpoint_json TEXT NOT NULL, resume_state_json TEXT NOT NULL, sealed_before_ms INTEGER,
 updated_at_ms INTEGER NOT NULL,
 scan_incomplete INTEGER NOT NULL DEFAULT 0 CHECK(scan_incomplete IN (0,1))
) STRICT;
CREATE TABLE IF NOT EXISTS usage_entry(
 source_id TEXT NOT NULL REFERENCES usage_source, request_id TEXT NOT NULL,
 identity_key TEXT NOT NULL, identity_origin TEXT NOT NULL, timestamp_ms INTEGER NOT NULL,
 provider TEXT NOT NULL, raw_model TEXT NOT NULL, display_model TEXT NOT NULL,
 input_tokens INTEGER NOT NULL, non_cached_input_tokens INTEGER NOT NULL,
 cache_read_tokens INTEGER NOT NULL, cache_creation_tokens INTEGER NOT NULL,
 output_tokens INTEGER NOT NULL, reasoning_output_tokens INTEGER NOT NULL,
 cost_micro_usd INTEGER, cache_savings_micro_usd INTEGER, price_rule_id TEXT,
 pricing_precision_json TEXT NOT NULL, single_request_evidence INTEGER NOT NULL,
 PRIMARY KEY(source_id,request_id)
) STRICT;
CREATE INDEX IF NOT EXISTS entry_time ON usage_entry(timestamp_ms,provider);
CREATE TABLE IF NOT EXISTS usage_identity(
 provider TEXT NOT NULL,identity_key TEXT NOT NULL,source_id TEXT NOT NULL REFERENCES usage_source,
 request_id TEXT NOT NULL,origin_id TEXT NOT NULL,timestamp_ms INTEGER NOT NULL,
 PRIMARY KEY(provider,identity_key)
) STRICT;
CREATE INDEX IF NOT EXISTS identity_owner ON usage_identity(provider,source_id,request_id);
CREATE TABLE IF NOT EXISTS usage_bucket(
 source_id TEXT NOT NULL REFERENCES usage_source,provider TEXT NOT NULL,display_model TEXT NOT NULL,
 bucket_kind TEXT NOT NULL CHECK(bucket_kind IN ('hour','day','month')),bucket_start_ms INTEGER NOT NULL,
 request_count INTEGER NOT NULL CHECK(request_count>=0),
 input_tokens INTEGER NOT NULL CHECK(input_tokens>=0),
 non_cached_input_tokens INTEGER NOT NULL CHECK(non_cached_input_tokens>=0),
 cache_read_tokens INTEGER NOT NULL CHECK(cache_read_tokens>=0),
 cache_creation_tokens INTEGER NOT NULL CHECK(cache_creation_tokens>=0),
 output_tokens INTEGER NOT NULL CHECK(output_tokens>=0),
 reasoning_output_tokens INTEGER NOT NULL CHECK(reasoning_output_tokens>=0),
 total_tokens INTEGER NOT NULL CHECK(total_tokens>=0),
 priced_request_count INTEGER NOT NULL CHECK(priced_request_count>=0),
 unpriced_request_count INTEGER NOT NULL CHECK(unpriced_request_count>=0),
 priced_tokens INTEGER NOT NULL CHECK(priced_tokens>=0),
 cost_micro_usd INTEGER NOT NULL CHECK(cost_micro_usd>=0),
 cache_savings_micro_usd INTEGER NOT NULL CHECK(cache_savings_micro_usd>=0),
 PRIMARY KEY(source_id,provider,display_model,bucket_kind,bucket_start_ms)
) STRICT;
CREATE INDEX IF NOT EXISTS bucket_time ON usage_bucket(bucket_kind,bucket_start_ms,provider);
CREATE VIEW IF NOT EXISTS entry_metrics AS
 SELECT source_id,request_id,timestamp_ms,provider,display_model,1 AS request_count,
 input_tokens,non_cached_input_tokens,cache_read_tokens,cache_creation_tokens,
 output_tokens,reasoning_output_tokens,
 non_cached_input_tokens+cache_read_tokens+cache_creation_tokens+output_tokens AS total_tokens,
 cost_micro_usd IS NOT NULL AS priced_request_count,cost_micro_usd IS NULL AS unpriced_request_count,
 CASE WHEN cost_micro_usd IS NULL THEN 0 ELSE non_cached_input_tokens+cache_read_tokens+cache_creation_tokens+output_tokens END AS priced_tokens,
 coalesce(cost_micro_usd,0) AS cost_micro_usd, coalesce(cache_savings_micro_usd,0) AS cache_savings_micro_usd FROM usage_entry;
CREATE TABLE IF NOT EXISTS quota_observation(
 provider TEXT NOT NULL, scope_id TEXT NOT NULL,target_id TEXT NOT NULL,period_kind TEXT NOT NULL,
 effective_ms INTEGER NOT NULL,event_time_ms INTEGER,ingested_at_ms INTEGER NOT NULL,observation_json TEXT NOT NULL,
 PRIMARY KEY(provider,scope_id,target_id,period_kind,effective_ms)
) STRICT;
CREATE TABLE IF NOT EXISTS credit_observation(
 provider TEXT NOT NULL,scope_id TEXT NOT NULL,effective_ms INTEGER NOT NULL,event_time_ms INTEGER,
 ingested_at_ms INTEGER NOT NULL,plan_type TEXT,service_tier TEXT,state_json TEXT NOT NULL,
 account_confirmed INTEGER NOT NULL,PRIMARY KEY(provider,scope_id,effective_ms)
) STRICT;
"#;

pub(crate) fn initialize(db: &mut Connection, file: bool) -> Result<()> {
    db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;")?;
    if file {
        let mode: String = db.query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))?;
        if !mode.eq_ignore_ascii_case("wal") {
            return Err(StoreError::Invalid("WAL unavailable"));
        }
        db.execute_batch("PRAGMA synchronous=NORMAL;")?;
    }
    let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    tx.execute_batch(SCHEMA)?;
    let version: String = tx.query_row(
        "SELECT value FROM schema_meta WHERE key='schema_version'",
        [],
        |r| r.get(0),
    )?;
    if version != "1" {
        return Err(StoreError::Invalid("unsupported schema version"));
    }
    tx.commit()?;
    Ok(())
}
pub(crate) fn retained(db: &Connection) -> Result<i64> {
    let s: String = db.query_row(
        "SELECT value FROM schema_meta WHERE key='detail_retained_from_ms'",
        [],
        |r| r.get(0),
    )?;
    s.parse()
        .map_err(|_| StoreError::Invalid("invalid retention watermark"))
}

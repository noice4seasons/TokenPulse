use crate::assets::APP_ORIGIN;
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};
use tokenpulse_core_domain::{config::AppConfig, dto::PROTOCOL_VERSION};

pub fn is_allowed_top_level_navigation(url: &str) -> bool {
    matches!(url.strip_prefix(APP_ORIGIN), Some("/" | "/index.html"))
}

#[derive(Debug, PartialEq, Eq)]
pub enum HostCommand {
    Ready,
    Refresh,
    Filter { provider: String, range: String },
    UpdateConfig(AppConfig),
    ExportJson,
    ExportCsv,
    ToggleOverlay,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SecurityRejectReason {
    UntrustedSource,
    PayloadTooLarge,
    RateLimited,
    InvalidEnvelope,
    UnsupportedVersion,
    UnknownCommand,
    InvalidPayload,
    InvalidProvider,
    InvalidRange,
}

#[derive(Default, Debug)]
pub struct MessageRateLimiter {
    arrivals: VecDeque<Instant>,
}

impl MessageRateLimiter {
    pub fn new() -> Self {
        Self::default()
    }

    fn admit(&mut self, now: Instant) -> bool {
        while self
            .arrivals
            .front()
            .is_some_and(|first| now.saturating_duration_since(*first) >= Duration::from_secs(1))
        {
            self.arrivals.pop_front();
        }
        if self.arrivals.len() >= 30 {
            return false;
        }
        self.arrivals.push_back(now);
        true
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope<T> {
    protocol_version: u32,
    command: String,
    payload: Option<T>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Filter {
    provider: String,
    range: String,
}

pub fn validate_incoming_message(
    source_uri: &str,
    raw_json: &str,
    limiter: &mut MessageRateLimiter,
    now: Instant,
) -> Result<HostCommand, SecurityRejectReason> {
    use SecurityRejectReason::*;
    if !is_allowed_top_level_navigation(source_uri) {
        return Err(UntrustedSource);
    }
    if raw_json.len() > 16 * 1024 {
        return Err(PayloadTooLarge);
    }
    // Charge malformed attempts too, before allocating or decoding their payload.
    if !limiter.admit(now) {
        return Err(RateLimited);
    }
    if !raw_json.trim_start().starts_with('{') {
        return Err(InvalidEnvelope);
    }
    let msg: Envelope<Value> = serde_json::from_str(raw_json).map_err(|_| InvalidEnvelope)?;
    if msg.protocol_version != PROTOCOL_VERSION {
        return Err(UnsupportedVersion);
    }

    // Deserialize typed payloads directly from the original bytes. Going through
    // a JSON map would silently overwrite duplicate config and filter fields.
    match msg.command.as_str() {
        "filter" => {
            let Filter { provider, range } = payload::<Filter>(raw_json, msg.payload.as_ref())?;
            if !matches!(provider.as_str(), "all" | "codex" | "antigravity") {
                return Err(InvalidProvider);
            }
            if !matches!(range.as_str(), "today" | "7d" | "30d") {
                return Err(InvalidRange);
            }
            Ok(HostCommand::Filter { provider, range })
        }
        "update_config" => Ok(HostCommand::UpdateConfig(
            payload::<AppConfig>(raw_json, msg.payload.as_ref())?.sanitized(),
        )),
        name => {
            let command = match name {
                "ready" => HostCommand::Ready,
                "refresh" => HostCommand::Refresh,
                "export_json" => HostCommand::ExportJson,
                "export_csv" => HostCommand::ExportCsv,
                "toggle_overlay" => HostCommand::ToggleOverlay,
                _ => return Err(UnknownCommand),
            };
            if msg.payload.is_some() {
                return Err(InvalidPayload);
            }
            Ok(command)
        }
    }
}

fn payload<T: serde::de::DeserializeOwned>(
    raw: &str,
    value: Option<&Value>,
) -> Result<T, SecurityRejectReason> {
    // Serde structs can also accept positional arrays. The JS contract uses
    // named properties, including both nested provider configurations.
    let fields = value
        .and_then(Value::as_object)
        .ok_or(SecurityRejectReason::InvalidPayload)?;
    if fields.values().any(Value::is_array) {
        return Err(SecurityRejectReason::InvalidPayload);
    }
    serde_json::from_str::<Envelope<T>>(raw)
        .ok()
        .and_then(|m| m.payload)
        .ok_or(SecurityRejectReason::InvalidPayload)
}

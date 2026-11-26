use crate::checkpoint::digest;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub type Counters = [u64; 4];
pub const CODEX_RECENT_OBSERVATIONS: usize = 128;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Unset,
    Single,
    Cumulative,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexAccountingState {
    pub version: u32,
    pub notification_offset: Option<Counters>,
    pub notification_raw_total: Option<Counters>,
    pub notification_turn_id: String,
    pub detailed_total: Option<Counters>,
    pub detailed_turn_total: Option<Counters>,
    pub detailed_turn_id: String,
    pub generation: u64,
    pub last_entry_id: Option<String>,
    pub notification_total: Option<Counters>,
    pub session_id: String,
    pub created_ms: i64,
    pub forked: bool,
    pub turn_id: String,
    pub inherited: bool,
    pub mode: Mode,
    pub total: Option<Counters>,
    pub segment: u64,
    pub fresh_output: bool,
    pub recent: Vec<String>,
}
impl CodexAccountingState {
    pub fn new(generation: u64) -> Self {
        Self {
            version: 4,
            generation,
            notification_offset: None,
            notification_raw_total: None,
            notification_turn_id: String::new(),
            detailed_total: None,
            detailed_turn_total: None,
            detailed_turn_id: String::new(),
            last_entry_id: None,
            notification_total: None,
            session_id: String::new(),
            created_ms: 0,
            forked: false,
            turn_id: String::new(),
            inherited: false,
            mode: Mode::Unset,
            total: None,
            segment: 0,
            fresh_output: false,
            recent: Vec::new(),
        }
    }
    fn remember(&mut self, key: Option<&str>) {
        if let Some(k) = key {
            if !self.recent.iter().any(|x| x == k) {
                self.recent.push(k.into());
                if self.recent.len() > CODEX_RECENT_OBSERVATIONS {
                    self.recent.remove(0);
                }
            }
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecisionKind {
    Usage,
    Alias,
}
#[derive(Clone, Debug)]
pub struct CodexUsageDecision {
    pub kind: DecisionKind,
    pub request_id: String,
    pub identity_key: String,
    pub identity_origin: String,
    pub usage: Counters,
    pub observation_key: Option<String>,
    pub response_key: Option<String>,
    pub single_request_evidence: bool,
}
pub fn usage_vector(value: &Value) -> Option<Counters> {
    value.as_object()?;
    let mut v = [0; 4];
    for (slot, name) in v.iter_mut().zip([
        "input_tokens",
        "cached_input_tokens",
        "output_tokens",
        "reasoning_output_tokens",
    ]) {
        let n = &value[name];
        *slot = if n.is_null() {
            0
        } else {
            let f = n.as_f64()?;
            if !(0.0..=9_007_199_254_740_991.0).contains(&f) || f.fract() != 0.0 {
                return None;
            }
            f as u64
        };
    }
    v[1] = v[1].min(v[0]);
    v[3] = v[3].min(v[2]);
    Some(v)
}
fn covered(a: Counters, b: Counters) -> bool {
    a[0] <= b[0] && a[2] <= b[2]
}
fn nonzero(v: Option<Counters>) -> bool {
    v.is_some_and(|v| v[0] != 0 || v[2] != 0)
}
fn offset(a: Counters, b: Counters) -> Option<Counters> {
    let mut d = [0; 4];
    for i in 0..4 {
        d[i] = a[i].checked_sub(b[i])?;
    }
    Some(d)
}
fn key(label: &str, tuple: impl Serialize) -> String {
    let bytes =
        serde_json::to_vec(&tuple).expect("identity tuples contain only strings and integers");
    format!("codex:{label}:{}", digest(&bytes))
}
fn string<'a>(p: &'a Value, key: &str) -> &'a str {
    p[key].as_str().unwrap_or("")
}

// The original timestamp spelling participates in the observation hash. Only the
// separately parsed UTC milliseconds are used for fork boundaries and pricing.
pub(crate) fn timestamp_ms(s: &str) -> Option<i64> {
    if !s.is_ascii() {
        return None;
    }
    let b = s.as_bytes();
    let num = |at: usize, len: usize| -> Option<i64> {
        let digits = s.get(at..at + len)?;
        digits
            .bytes()
            .all(|c| c.is_ascii_digit())
            .then(|| digits.parse().ok())
            .flatten()
    };
    let (mut y, m, d) = (num(0, 4)?, num(5, 2)?, num(8, 2)?);
    if b.get(4) != Some(&b'-') || b.get(7) != Some(&b'-') || !(1..=12).contains(&m) {
        return None;
    }
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let days_in_month = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if !(1..=days_in_month[(m - 1) as usize]).contains(&d) {
        return None;
    }
    y -= i64::from(m <= 2);
    let era = y.div_euclid(400);
    let year = y - era * 400;
    let day = era * 146097 + year * 365 + year / 4 - year / 100
        + (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5
        + d
        - 1
        - 719468;
    if b.len() == 10 {
        return Some(day * 86_400_000);
    }
    if !matches!(b.get(10), Some(b'T' | b't' | b' '))
        || b.get(13) != Some(&b':')
        || b.get(16) != Some(&b':')
    {
        return None;
    }
    let (h, min, sec) = (num(11, 2)?, num(14, 2)?, num(17, 2)?);
    if h > 23 || min > 59 || sec > 59 {
        return None;
    }
    let mut pos = 19;
    let mut ms = 0;
    if b.get(pos) == Some(&b'.') {
        pos += 1;
        let start = pos;
        while let Some(ch) = b.get(pos).filter(|ch| ch.is_ascii_digit()) {
            if pos - start < 3 {
                ms += i64::from(*ch - b'0') * [100, 10, 1][pos - start];
            }
            pos += 1;
        }
        if pos == start {
            return None;
        }
    }
    let zone = match b.get(pos) {
        Some(b'Z' | b'z') if pos + 1 == b.len() => 0,
        Some(sign @ (b'+' | b'-')) if pos + 6 == b.len() && b[pos + 3] == b':' => {
            let (hh, mm) = (num(pos + 1, 2)?, num(pos + 4, 2)?);
            if hh > 23 || mm > 59 {
                return None;
            }
            (if *sign == b'+' { 1 } else { -1 }) * (hh * 60 + mm)
        }
        _ => return None,
    };
    Some(((day * 1440 + h * 60 + min - zone) * 60 + sec) * 1000 + ms)
}

fn align(
    s: &mut CodexAccountingState,
    p: &Value,
    structured: bool,
    raw: Counters,
    already: bool,
) -> Option<Counters> {
    let turn = usage_vector(&p["turn_token_usage"]);
    if structured {
        let replay = already && s.total.is_some_and(|t| covered(raw, t));
        if let Some(turn) =
            turn.filter(|_| !replay && !s.detailed_total.is_some_and(|t| covered(raw, t)))
        {
            let id = string(p, "turn_id");
            if let (Some(previous), Some(notification)) =
                (s.notification_raw_total, s.notification_total)
            {
                if !id.is_empty()
                    && id == s.notification_turn_id
                    && (previous == turn || previous == raw)
                {
                    if let Some(d) = offset(raw, previous) {
                        if s.total == Some(notification) {
                            s.total = Some(raw);
                        }
                        s.notification_total = Some(raw);
                        s.notification_offset = Some(d);
                    }
                }
            }
            s.detailed_total = Some(raw);
            s.detailed_turn_total = Some(turn);
            s.detailed_turn_id = id.into();
        }
    } else {
        if let (Some(thread), Some(turn)) = (s.detailed_total, s.detailed_turn_total) {
            if raw == thread
                || !s.detailed_turn_id.is_empty() && s.turn_id == s.detailed_turn_id && raw == turn
            {
                if let Some(d) = offset(thread, raw) {
                    s.notification_offset = Some(d);
                }
            }
            s.notification_offset?;
        }
        s.notification_raw_total = Some(raw);
        s.notification_turn_id.clone_from(&s.turn_id);
        if let Some(d) = s.notification_offset {
            let mut sum = raw;
            for i in 0..4 {
                sum[i] = raw[i].checked_add(d[i])?;
            }
            return Some(sum);
        }
    }
    Some(raw)
}

/// identity_source returns the stable source owning a previously committed identity.
pub fn codex_usage_decision(
    s: &mut CodexAccountingState,
    obj: &Value,
    source: &str,
    physical: &str,
    identity_source: impl Fn(&str) -> Option<String>,
) -> Option<CodexUsageDecision> {
    let p = obj.get("payload")?;
    let kind = string(obj, "type");
    let event = string(p, "type");
    let time = obj["timestamp"].as_str().and_then(timestamp_ms);
    if kind == "session_meta" {
        if s.session_id.is_empty() {
            s.session_id = p["id"]
                .as_str()
                .filter(|x| !x.is_empty())
                .unwrap_or(source)
                .into();
            s.created_ms = timestamp_ms(string(p, "timestamp")).unwrap_or(0);
            s.forked = !string(p, "forked_from_id").is_empty();
            s.inherited = s.forked;
        }
        return None;
    }
    if kind == "event_msg" && event == "task_started" {
        s.turn_id = string(p, "turn_id").into();
        let start = p["started_at"]
            .as_f64()
            .map(|x| x * 1000.)
            .or(time.map(|t| t as f64));
        if s.forked {
            s.inherited = s.created_ms > 0
                && start.is_some_and(|t| t < ((s.created_ms / 1000) * 1000) as f64);
        }
        return None;
    }
    if kind == "response_item"
        && !s.inherited
        && (string(p, "role") == "assistant"
            || matches!(event, "function_call" | "custom_tool_call"))
    {
        s.fresh_output = true;
        return None;
    }
    let structured = kind == "token_usage_record";
    if !structured && !(kind == "event_msg" && event == "token_count") {
        return None;
    }
    let info = &p["info"];
    let last = usage_vector(if structured {
        &p["usage"]
    } else {
        &info["last_token_usage"]
    });
    let raw = usage_vector(if structured {
        &p["thread_token_usage"]
    } else {
        &info["total_token_usage"]
    });
    let response = p["response_id"]
        .as_str()
        .filter(|x| structured && !x.is_empty())
        .map(|x| format!("codex:response:{x}"));
    let owner = response.as_deref().and_then(&identity_source);
    let recent_response = response.as_ref().is_some_and(|r| s.recent.contains(r));
    let already = recent_response || owner.is_some();
    let total = match raw {
        Some(v) => Some(align(s, p, structured, v, already)?),
        None => None,
    };
    let session = if s.session_id.is_empty() {
        source.to_owned()
    } else {
        s.session_id.clone()
    };
    let origin = format!("codex:session:{session}");
    let observation = total
        .zip(time)
        .map(|(v, _)| key("observation", (&session, string(obj, "timestamp"), v)));
    let obs_owner = observation.as_deref().and_then(&identity_source);
    let repeated =
        obs_owner.is_some() || observation.as_ref().is_some_and(|k| s.recent.contains(k));
    if s.inherited
        && structured
        && p["thread_id"].as_str() == Some(s.session_id.as_str())
        && s.created_ms > 0
        && time.is_some_and(|t| t >= s.created_ms)
    {
        s.inherited = false;
        s.fresh_output = true;
    }
    if s.inherited {
        if total.is_some() {
            s.total = total;
            s.mode = Mode::Cumulative;
        }
        s.remember(observation.as_deref());
        return None;
    }
    let link = |s: &mut CodexAccountingState, target: String| {
        s.remember(observation.as_deref());
        s.remember(response.as_deref());
        Some(CodexUsageDecision {
            kind: DecisionKind::Alias,
            request_id: target.clone(),
            identity_key: target,
            identity_origin: origin.clone(),
            usage: [0; 4],
            observation_key: observation.clone(),
            response_key: response.clone(),
            single_request_evidence: false,
        })
    };
    let forward = |old: Option<Counters>, v| old.map_or(true, |t| covered(t, v));
    if let Some(t) = total {
        if !structured
            && repeated
            && s.total.is_some_and(|old| covered(t, old))
            && forward(s.notification_total, t)
        {
            s.notification_total = total;
        }
    }
    if obs_owner.is_some() {
        if let Some(t) = total {
            if forward(s.total, t)
                && (s.total.is_none()
                    || identity_source(s.last_entry_id.as_deref().unwrap_or("")).as_deref()
                        != Some(source))
            {
                s.total = total;
                s.mode = Mode::Cumulative;
                s.last_entry_id = observation.clone();
            }
        }
        return if response.is_some() {
            link(s, observation.clone()?)
        } else {
            None
        };
    }
    if response.is_none() && repeated {
        return None;
    }
    if owner.as_deref().is_some_and(|o| o != source) {
        if total.is_some_and(|t| forward(s.total, t)) {
            s.total = total;
            s.mode = Mode::Cumulative;
            s.last_entry_id = response.clone();
        }
        s.remember(observation.as_deref());
        s.remember(response.as_deref());
        return None;
    }
    if already
        && total
            .zip(s.total)
            .is_some_and(|(t, old)| !covered(old, t) || owner.is_some() && !recent_response)
    {
        s.remember(observation.as_deref());
        return None;
    }
    let mut usage = None;
    let mut evidence = false;
    let mut id = response.clone().unwrap_or_else(|| physical.into());
    if let Some(t) = total {
        let previous = s.total;
        let previous_mode = s.mode;
        let notification = s.notification_total;
        if let (Some(old), Some(target)) = (previous, s.last_entry_id.clone()) {
            let covered_by_thread = !structured
                && s.notification_offset.is_some()
                && s.detailed_total.is_some_and(|v| covered(t, v));
            if covered(t, old)
                && (covered_by_thread
                    || structured && notification.is_some()
                    || !structured
                        && forward(notification, t)
                        && (!s.fresh_output || notification == total))
            {
                if !structured {
                    s.notification_total = total;
                }
                return link(s, target);
            }
        }
        if !structured {
            s.notification_total = total;
        }
        if previous == total {
            if response.is_some() {
                if let Some(target) = s.last_entry_id.clone() {
                    return link(s, target);
                }
            }
            if response.is_some() && !already && nonzero(last) {
                usage = last;
                evidence = true;
            } else {
                s.remember(observation.as_deref());
                return None;
            }
        } else {
            s.mode = Mode::Cumulative;
            s.total = total;
            match previous.filter(|_| previous_mode == Mode::Cumulative) {
                None => {
                    s.last_entry_id = None;
                    if previous_mode == Mode::Unset
                        && nonzero(last)
                        && !already
                        && (total == last || response.is_some() || s.fresh_output)
                    {
                        usage = last;
                        evidence = true; // last_token_usage is explicitly per request
                    }
                }
                Some(old) if !covered(old, t) => {
                    s.segment += 1;
                    s.last_entry_id = None;
                    s.recent.clear();
                    if nonzero(last) && !already && (response.is_some() || s.fresh_output) {
                        usage = last;
                        evidence = true;
                    }
                }
                Some(old) => {
                    let mut delta = std::array::from_fn(|i| t[i].saturating_sub(old[i]));
                    delta[1] = delta[1].min(delta[0]);
                    delta[3] = delta[3].min(delta[2]);
                    usage = Some(delta);
                    // A detailed record can cover omitted requests. The per-response
                    // vector must actually account for the charged interval.
                    evidence = usage == last && (structured || s.fresh_output);
                }
            }
        }
        id = key("counter", (&session, s.generation, s.segment, t));
    } else if s.mode != Mode::Cumulative && nonzero(last) && !already {
        s.mode = Mode::Single;
        usage = last;
        evidence = true;
    }
    s.remember(observation.as_deref());
    s.remember(response.as_deref());
    s.fresh_output = false;
    if !nonzero(usage) {
        return None;
    }
    s.last_entry_id = Some(id.clone());
    Some(CodexUsageDecision {
        kind: DecisionKind::Usage,
        request_id: id.clone(),
        identity_key: id,
        identity_origin: origin,
        usage: usage?,
        observation_key: observation,
        response_key: response.filter(|_| !already),
        single_request_evidence: evidence,
    })
}

impl Default for CodexAccountingState {
    fn default() -> Self {
        Self::new(0)
    }
}

use crate::{provider, ExactRange, Result};
use serde::Serialize;
use tokenpulse_core_domain::{
    dto::{Coverage, QueryGranularity},
    models::UsageMetrics,
    pricing::RULES,
};

#[derive(Clone, Copy, Debug)]
pub enum ExportFormat {
    Json,
    Csv,
}

#[derive(Serialize)]
struct Row<'a> {
    kind: &'static str,
    provider: Option<&'static str>,
    model: Option<String>,
    from_ms: i64,
    to_ms: i64,
    coverage: Coverage,
    available_granularity: QueryGranularity,
    #[serde(flatten)]
    metrics: &'a UsageMetrics,
}
/// Model labels originate in logs. Only public catalog names can pass through.
/// Other models keep separate rows under opaque, export-local labels.
fn label(value: &str, index: usize) -> String {
    if RULES.iter().any(|r| r.model.eq_ignore_ascii_case(value))
        || [
            "Opus",
            "Sonnet",
            "Haiku",
            "GPT-4o",
            "GPT-4",
            "GLM",
            "Unknown (codex)",
        ]
        .contains(&value)
    {
        value.to_owned()
    } else {
        format!("model-{}", index + 1)
    }
}
pub fn export(range: &ExactRange, format: ExportFormat) -> Result<String> {
    let a = &range.aggregate;
    let row = |kind, provider, model, metrics| Row {
        kind,
        provider,
        model,
        metrics,
        from_ms: a.from_ms,
        to_ms: a.to_ms,
        coverage: a.coverage,
        available_granularity: range.available_granularity,
    };
    let mut rows = vec![row("aggregate", None, None, &a.metrics)];
    rows.extend(range.models.iter().enumerate().map(|(i, m)| {
        row(
            "model",
            Some(provider(m.provider)),
            Some(label(&m.display_model, i)),
            &m.metrics,
        )
    }));
    match format {
        ExportFormat::Json => Ok(serde_json::to_string(&rows)?),
        ExportFormat::Csv => {
            let mut csv=String::from("kind,provider,model,from_ms,to_ms,coverage,available_granularity,request_count,input_tokens,non_cached_input_tokens,cache_read_tokens,cache_creation_tokens,output_tokens,reasoning_output_tokens,total_tokens,priced_request_count,unpriced_request_count,priced_tokens,cost_micro_usd,cache_savings_micro_usd\r\n");
            for r in rows {
                let m = r.metrics;
                let fields = vec![
                    r.kind.to_owned(),
                    r.provider.unwrap_or("").to_owned(),
                    r.model.unwrap_or_default(),
                    r.from_ms.to_string(),
                    r.to_ms.to_string(),
                    format!("{:?}", r.coverage),
                    format!("{:?}", r.available_granularity),
                    m.request_count.to_string(),
                    m.input_tokens.to_string(),
                    m.non_cached_input_tokens.to_string(),
                    m.cache_read_tokens.to_string(),
                    m.cache_creation_tokens.to_string(),
                    m.output_tokens.to_string(),
                    m.reasoning_output_tokens.to_string(),
                    m.total_tokens.to_string(),
                    m.priced_request_count.to_string(),
                    m.unpriced_request_count.to_string(),
                    m.priced_tokens.to_string(),
                    m.cost_micro_usd.to_string(),
                    m.cache_savings_micro_usd.to_string(),
                ];
                csv.push_str(&fields.join(","));
                csv.push_str("\r\n");
            }
            Ok(csv)
        }
    }
}

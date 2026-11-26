pub fn normalize_antigravity_model(raw: &str) -> String {
    match raw {
        "" => "Unknown (antigravity)",
        "gemini-4-neon-high" => "Gemini 4 Neon (High)",
        "gemini-3.8-flash" => "Gemini 3.8 Flash",
        "claude-opus-5-5-high" => "Claude Opus 5.5 (High)",
        other => other,
    }
    .to_owned()
}

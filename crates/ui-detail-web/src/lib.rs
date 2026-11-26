pub mod assets;
mod host;
pub mod security;

pub use assets::{resolve_asset, APP_ORIGIN, STRICT_CSP};
pub use host::{DetailHostError, DetailWindowHost};
pub use security::*;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use std::time::{Duration, Instant};

    const PAGE: &str = "https://app.tokenpulse.local/index.html";
    const READY: &str = r#"{"protocol_version":1,"command":"ready","payload":null}"#;

    fn check(raw: &str) -> Result<HostCommand, SecurityRejectReason> {
        validate_incoming_message(PAGE, raw, &mut MessageRateLimiter::new(), Instant::now())
    }
    fn message(command: &str, payload: Value) -> String {
        json!({"protocol_version":1,"command":command,"payload":payload}).to_string()
    }

    #[test]
    fn reject_foreign_or_ambiguous_urls_at_every_boundary() {
        for url in [
            "https://evil.com/",
            "http://app.tokenpulse.local/",
            "https://app.tokenpulse.local.evil.com/",
            "file:///C:/Windows/win.ini",
            "javascript:alert(1)",
            "data:text/plain,x",
            "about:blank",
            "https://sub.app.tokenpulse.local/",
            "https://app.tokenpulse.local@evil.com/",
            "https://user@app.tokenpulse.local/",
            "https://app.tokenpulse.local:8443/",
            "https://app.tokenpulse.local:443/",
            "HTTPS://app.tokenpulse.local/",
            "https://APP.tokenpulse.local/",
            APP_ORIGIN,
        ] {
            reject_url(url);
        }
        for path in [
            "/../Cargo.toml",
            "/%2e%2e/secret",
            "/\\index.html",
            "/%2E/index.html",
            "//index.html",
            "/index.html?x",
            "/index.html#",
            "/index.html/",
            "/x/../app.js",
            "/%69ndex.html",
            "/app.js\0",
            "/unknown",
        ] {
            reject_url(&format!("{APP_ORIGIN}{path}"));
        }
    }
    fn reject_url(url: &str) {
        assert!(resolve_asset(url).is_none(), "{url:?}");
        assert!(!is_allowed_top_level_navigation(url), "{url:?}");
        assert_eq!(
            validate_incoming_message(url, READY, &mut MessageRateLimiter::new(), Instant::now()),
            Err(SecurityRejectReason::UntrustedSource),
            "{url:?}"
        );
    }

    #[test]
    fn resources_and_documents_have_distinct_allowlists() {
        let mut unique_bytes = 0;
        for (path, mime, document) in [
            ("/", "text/html; charset=utf-8", true),
            ("/index.html", "text/html; charset=utf-8", true),
            ("/app.css", "text/css; charset=utf-8", false),
            ("/app.js", "application/javascript; charset=utf-8", false),
            ("/logo.svg", "image/svg+xml", false),
        ] {
            let url = format!("{APP_ORIGIN}{path}");
            let asset = resolve_asset(&url).unwrap();
            assert_eq!(asset.content_type, mime);
            assert!(!asset.bytes.is_empty());
            if path != "/" {
                unique_bytes += asset.bytes.len();
            }
            assert_eq!(is_allowed_top_level_navigation(&url), document);
            assert_eq!(
                validate_incoming_message(
                    &url,
                    READY,
                    &mut MessageRateLimiter::new(),
                    Instant::now()
                )
                .is_ok(),
                document
            );
        }
        assert_eq!(
            resolve_asset(PAGE).unwrap().bytes,
            resolve_asset(&format!("{APP_ORIGIN}/")).unwrap().bytes
        );
        assert!(unique_bytes < 150 * 1024);
        let policies: Vec<_> = STRICT_CSP.split("; ").collect();
        assert!(policies.contains(&"connect-src 'none'"));
        assert!(policies.contains(&"frame-ancestors 'none'"));
    }

    #[test]
    fn message_size_is_a_utf8_byte_limit_including_whitespace() {
        let exact = format!("{READY}{}", " ".repeat(16 * 1024 - READY.len()));
        assert_eq!(check(&exact), Ok(HostCommand::Ready));
        assert_eq!(
            check(&(exact + " ")),
            Err(SecurityRejectReason::PayloadTooLarge)
        );
        assert_eq!(
            check(&message("ready", json!("字".repeat(6000)))),
            Err(SecurityRejectReason::PayloadTooLarge)
        );
    }

    #[test]
    fn strict_envelopes_and_command_payloads() {
        use SecurityRejectReason::*;
        for (raw, reason) in [
            (
                r#"{"protocol_version":2,"command":"ready"}"#,
                UnsupportedVersion,
            ),
            (r#"{"command":"ready"}"#, InvalidEnvelope),
            (
                r#"{"protocol_version":1,"command":"query"}"#,
                UnknownCommand,
            ),
            (
                r#"{"protocol_version":1,"command":"ready","extra":null}"#,
                InvalidEnvelope,
            ),
            (
                r#"{"protocol_version":1,"command":"ready","command":"refresh"}"#,
                InvalidEnvelope,
            ),
            (r#"[1,"ready",null]"#, InvalidEnvelope),
            (
                r#"{"protocol_version":1,"command":"refresh","payload":{}}"#,
                InvalidPayload,
            ),
            (
                r#"{"protocol_version":1,"command":"filter","payload":{"provider":"all","range":"7d","limit":1}}"#,
                InvalidPayload,
            ),
            (
                r#"{"protocol_version":1,"command":"filter","payload":{"provider":"all","range":"7d","range":"30d"}}"#,
                InvalidPayload,
            ),
            (
                r#"{"protocol_version":1,"command":"filter","payload":{"provider":"all"}}"#,
                InvalidPayload,
            ),
            (
                r#"{"protocol_version":1,"command":"filter","payload":["all","7d"]}"#,
                InvalidPayload,
            ),
            (
                r#"{"protocol_version":1,"command":"update_config","payload":{"extra":1}}"#,
                InvalidPayload,
            ),
            (
                r#"{"protocol_version":1,"command":"update_config","payload":{"codex":{"enabled":true,"extra":1}}}"#,
                InvalidPayload,
            ),
            (
                r#"{"protocol_version":1,"command":"update_config","payload":{"codex":[true]}}"#,
                InvalidPayload,
            ),
            (
                r#"{"protocol_version":1,"command":"update_config","payload":{"cooldown_minutes":3,"cooldown_minutes":6}}"#,
                InvalidPayload,
            ),
            (
                r#"{"protocol_version":1,"command":"update_config","payload":{"overlay_opacity_pct":-1}}"#,
                InvalidPayload,
            ),
        ] {
            assert_eq!(check(raw), Err(reason), "{raw}");
        }
    }

    #[test]
    fn accepted_commands_and_filters_use_the_frontend_contract() {
        for (name, cmd) in [
            ("ready", HostCommand::Ready),
            ("refresh", HostCommand::Refresh),
            ("export_json", HostCommand::ExportJson),
            ("export_csv", HostCommand::ExportCsv),
            ("toggle_overlay", HostCommand::ToggleOverlay),
        ] {
            assert_eq!(check(&message(name, Value::Null)), Ok(cmd));
        }
        for provider in ["all", "codex", "antigravity"] {
            for range in ["today", "7d", "30d"] {
                assert_eq!(
                    check(&message(
                        "filter",
                        json!({ "provider": provider, "range": range })
                    )),
                    Ok(HostCommand::Filter {
                        provider: provider.into(),
                        range: range.into()
                    })
                );
            }
        }
        for (provider, range, reason) in [
            ("other", "7d", SecurityRejectReason::InvalidProvider),
            ("all", "365d", SecurityRejectReason::InvalidRange),
        ] {
            assert_eq!(
                check(&message(
                    "filter",
                    json!({ "provider":provider, "range":range })
                )),
                Err(reason)
            );
        }
    }

    #[test]
    fn configuration_is_sanitized_by_the_domain() {
        let raw = message(
            "update_config",
            json!({"overlay_opacity_pct":1, "low_quota_threshold_pct":101,
            "antigravity":{"enabled":true,"poll_interval_secs":5}, "cooldown_minutes":2000}),
        );
        let HostCommand::UpdateConfig(config) = check(&raw).unwrap() else {
            panic!("expected config");
        };
        assert_eq!(
            (
                config.overlay_opacity_pct,
                config.low_quota_threshold_pct,
                config.antigravity.poll_interval_secs,
                config.cooldown_minutes
            ),
            (30, 100, 60, 1440)
        );
    }

    #[test]
    fn window_expires_attempts_individually_and_counts_malformed_input() {
        let mut limiter = MessageRateLimiter::new();
        let start = Instant::now();
        let mut send = |ms, raw| {
            validate_incoming_message(PAGE, raw, &mut limiter, start + Duration::from_millis(ms))
        };
        assert_eq!(send(0, "?"), Err(SecurityRejectReason::InvalidEnvelope));
        for _ in 0..29 {
            assert_eq!(send(900, READY), Ok(HostCommand::Ready));
        }
        assert_eq!(send(999, READY), Err(SecurityRejectReason::RateLimited));
        assert_eq!(send(1000, READY), Ok(HostCommand::Ready));
        assert_eq!(send(1001, READY), Err(SecurityRejectReason::RateLimited));
        assert_eq!(send(1900, READY), Ok(HostCommand::Ready));
    }
}

use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Theme {
    Dark,
    Light,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Language {
    ZhCn,
    EnUs,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum DisplayPlacement {
    #[default]
    Taskbar,
    DesktopWidget,
    Floating,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodexConfig {
    pub enabled: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AntigravityConfig {
    pub enabled: bool,
    pub poll_interval_secs: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AppConfig {
    pub theme: Theme,
    pub language: Language,
    pub display_placement: DisplayPlacement,
    pub overlay_opacity_pct: u32,
    pub always_on_top: bool,
    pub click_through: bool,
    pub start_on_login: bool,
    pub codex: CodexConfig,
    pub antigravity: AntigravityConfig,
    pub low_quota_threshold_pct: u32,
    pub cooldown_minutes: u32,
}
impl Default for AppConfig {
    fn default() -> Self {
        Self {
            theme: Theme::Dark,
            language: Language::ZhCn,
            display_placement: DisplayPlacement::Taskbar,
            overlay_opacity_pct: 90,
            always_on_top: true,
            click_through: false,
            start_on_login: false,
            codex: CodexConfig { enabled: true },
            antigravity: AntigravityConfig {
                enabled: true,
                poll_interval_secs: 90,
            },
            low_quota_threshold_pct: 10,
            cooldown_minutes: 60,
        }
    }
}
impl AppConfig {
    pub fn sanitized(mut self) -> Self {
        self.overlay_opacity_pct = self.overlay_opacity_pct.clamp(30, 100);
        self.antigravity.poll_interval_secs = self.antigravity.poll_interval_secs.clamp(60, 300);
        self.low_quota_threshold_pct = self.low_quota_threshold_pct.min(100);
        self.cooldown_minutes = self.cooldown_minutes.clamp(1, 1440);
        self
    }
}

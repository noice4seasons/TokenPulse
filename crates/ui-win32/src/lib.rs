pub mod autostart;
pub mod overlay;
pub mod power;
pub mod tokens;
pub mod tray;
pub use autostart::*;
pub use overlay::*;
pub use power::*;
pub use tokenpulse_core_domain::{
    config::{Language, Theme},
    dto::OverlaySnapshotDto,
};
pub use tokens::*;
pub use tray::*;
use windows::Win32::UI::HiDpi::*;

pub fn enable_per_monitor_dpi_v2() -> windows::core::Result<()> {
    unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) }
}
pub(crate) fn localized<'a>(language: Language, chinese: &'a str, english: &'a str) -> &'a str {
    match language {
        Language::ZhCn => chinese,
        Language::EnUs => english,
    }
}

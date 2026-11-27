use crate::{localized, DisplayPlacement, Language};
use std::{mem::size_of, ptr::null_mut};
use tokenpulse_core_domain::{
    models::ProviderId,
    quota::{should_alert_low_quota, QuotaObservation, QuotaPeriodKind},
};
use windows::{
    core::{Error, Result, PCWSTR},
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        UI::{Shell::*, WindowsAndMessaging::*},
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayIconState {
    Healthy,
    LowQuota,
    Paused,
    Offline,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayCommand {
    OpenDetail,
    ToggleOverlay,
    SetPlacement(DisplayPlacement),
    ToggleOverlayMode,
    ToggleClickThrough,
    RefreshNow,
    TogglePause,
    ToggleTheme,
    ToggleLanguage,
    ToggleAutostart,
    Exit,
}
#[derive(Clone, Copy, Default)]
pub struct TrayMenuState {
    pub overlay_visible: bool,
    pub placement: DisplayPlacement,
    pub click_through: bool,
    pub paused: bool,
    pub autostart: bool,
}
pub const TRAY_MESSAGE: u32 = WM_APP + 0x41;
struct Icon(HICON);
impl Drop for Icon {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyIcon(self.0);
        }
    }
}
struct Bitmap(HBITMAP);
impl Drop for Bitmap {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteObject(self.0);
        }
    }
}
struct Menu(HMENU);
impl Drop for Menu {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyMenu(self.0);
        }
    }
}

fn sample_icon(state: TrayIconState, x: f32, y: f32) -> u32 {
    let radius = (x - 16.).hypot(y - 16.);
    let status = match state {
        TrayIconState::Healthy => 0x34d399,
        TrayIconState::LowQuota => 0xfbbf24,
        TrayIconState::Paused => 0x818cf8,
        TrayIconState::Offline => 0x64748b,
    };
    let pulse = [
        (5., 17.),
        (11., 17.),
        (14., 10.),
        (17., 22.),
        (20., 15.),
        (22., 17.),
        (26., 17.),
    ];
    let rgb = if state == TrayIconState::LowQuota && (x - 26.).hypot(y - 7.) < 4.5 {
        0xfb7185
    } else if radius > 15.4 {
        return 0;
    } else if radius > 13.0 {
        status
    } else if pulse.windows(2).any(|ends| {
        let (ax, ay) = ends[0];
        let (dx, dy) = (ends[1].0 - ax, ends[1].1 - ay);
        let t = (((x - ax) * dx + (y - ay) * dy) / (dx * dx + dy * dy)).clamp(0., 1.);
        (x - ax - t * dx).hypot(y - ay - t * dy) < 1.15
    }) {
        0xf1f5f9
    } else {
        0x11161f
    };
    0xff000000 | rgb
}
fn icon_pixel(state: TrayIconState, x: usize, y: usize) -> u32 {
    let mut channels = [0u32; 4];
    for row in 0..4 {
        for col in 0..4 {
            let pixel = sample_icon(
                state,
                x as f32 + (col as f32 + 0.5) / 4.,
                y as f32 + (row as f32 + 0.5) / 4.,
            );
            for (index, sum) in channels.iter_mut().enumerate() {
                *sum += (pixel >> (index * 8)) & 255;
            }
        }
    }
    channels
        .into_iter()
        .enumerate()
        .fold(0, |value, (index, sum)| {
            value | (((sum + 8) / 16) << (index * 8))
        })
}
fn make_icon(state: TrayIconState) -> Result<Icon> {
    unsafe {
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: 32,
                biHeight: -32,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = null_mut();
        let color = Bitmap(CreateDIBSection(
            None,
            &info,
            DIB_RGB_COLORS,
            &mut bits,
            None,
            0,
        )?);
        if bits.is_null() {
            return Err(Error::from_hresult(E_POINTER));
        }
        for (index, pixel) in std::slice::from_raw_parts_mut(bits.cast::<u32>(), 1024)
            .iter_mut()
            .enumerate()
        {
            *pixel = icon_pixel(state, index % 32, index / 32);
        }
        let mask_bits = [0u8; 128];
        let mask = Bitmap(CreateBitmap(32, 32, 1, 1, Some(mask_bits.as_ptr().cast())));
        if mask.0.is_invalid() {
            return Err(Error::from_win32());
        }
        Ok(Icon(CreateIconIndirect(&ICONINFO {
            fIcon: BOOL(1),
            hbmMask: mask.0,
            hbmColor: color.0,
            ..Default::default()
        })?))
    }
}
fn wide_field<const N: usize>(text: &str) -> [u16; N] {
    let mut field = [0; N];
    let mut index = 0;
    for ch in text.chars().take_while(|ch| *ch != '\0') {
        let end = index + ch.len_utf16();
        if end >= N {
            break;
        }
        ch.encode_utf16(&mut field[index..end]);
        index = end;
    }
    field
}
fn tooltip(state: TrayIconState, language: Language, detail: &str) -> [u16; 128] {
    let (zh, en) = match state {
        TrayIconState::Healthy => ("正常", "Healthy"),
        TrayIconState::LowQuota => ("额度不足", "Low quota"),
        TrayIconState::Paused => ("已暂停", "Paused"),
        TrayIconState::Offline => ("离线", "Offline"),
    };
    wide_field(&format!(
        "TokenPulse · {}{}{}",
        localized(language, zh, en),
        if detail.is_empty() { "" } else { "\n" },
        detail
    ))
}
#[derive(Default)]
struct AlertHistory(Vec<(ProviderId, String, String, QuotaPeriodKind, i64)>);
impl AlertHistory {
    fn due(&mut self, obs: &QuotaObservation, threshold: f64, now: i64) -> bool {
        self.0.retain(|entry| now < entry.4);
        should_alert_low_quota(obs, threshold, now)
            && !self.0.iter().any(|entry| {
                entry.0 == obs.provider
                    && entry.1 == obs.scope_id
                    && entry.2 == obs.target_id
                    && entry.3 == obs.period_kind
            })
    }
    fn sent(&mut self, obs: &QuotaObservation, now: i64, cooldown: i64) {
        self.0.push((
            obs.provider,
            obs.scope_id.clone(),
            obs.target_id.clone(),
            obs.period_kind,
            now.saturating_add(cooldown.max(0)),
        ))
    }
}
pub struct TrayHost {
    owner: HWND,
    id: u16,
    icon: Icon,
    state: TrayIconState,
    language: Language,
    history: AlertHistory,
    registered: bool,
}
impl TrayHost {
    fn data(&self) -> NOTIFYICONDATAW {
        NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.owner,
            uID: self.id.into(),
            ..Default::default()
        }
    }
    pub fn new(owner: HWND, id: u16, state: TrayIconState, language: Language) -> Result<Self> {
        let mut host = Self {
            owner,
            id,
            icon: make_icon(state)?,
            state,
            language,
            history: AlertHistory::default(),
            registered: false,
        };
        let mut data = host.data();
        data.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP | NIF_SHOWTIP;
        data.hIcon = host.icon.0;
        data.uCallbackMessage = TRAY_MESSAGE;
        data.szTip = tooltip(state, language, "");
        unsafe {
            Shell_NotifyIconW(NIM_ADD, &data)
                .ok()
                .map_err(|err| Error::new(err.code(), "Shell_NotifyIconW(NIM_ADD) failed"))?;
            host.registered = true;
            data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
            Shell_NotifyIconW(NIM_SETVERSION, &data)
                .ok()
                .map_err(|err| {
                    Error::new(err.code(), "Shell_NotifyIconW(NIM_SETVERSION) failed")
                })?;
        }
        Ok(host)
    }
    pub fn update(&mut self, state: TrayIconState, language: Language, detail: &str) -> Result<()> {
        let mut data = self.data();
        let next = if state != self.state {
            Some(make_icon(state)?)
        } else {
            None
        };
        data.uFlags = NIF_TIP | NIF_SHOWTIP;
        data.szTip = tooltip(state, language, detail);
        if let Some(icon) = &next {
            data.uFlags |= NIF_ICON;
            data.hIcon = icon.0;
        }
        unsafe {
            Shell_NotifyIconW(NIM_MODIFY, &data).ok()?;
        }
        if let Some(icon) = next {
            self.icon = icon;
        }
        self.state = state;
        self.language = language;
        Ok(())
    }
    pub fn notify_low_quota(
        &mut self,
        obs: &QuotaObservation,
        threshold: f64,
        now_ms: i64,
        cooldown_ms: i64,
    ) -> Result<bool> {
        if !self.history.due(obs, threshold, now_ms) {
            return Ok(false);
        }
        let mut data = self.data();
        data.uFlags = NIF_INFO | NIF_REALTIME;
        data.dwInfoFlags = NIIF_WARNING;
        data.szInfoTitle = wide_field(localized(
            self.language,
            "TokenPulse · 低额度",
            "TokenPulse · Low quota",
        ));
        data.szInfo = wide_field(&format!(
            "{} · {} {:.1}%",
            obs.target_label,
            localized(self.language, "剩余", "Remaining"),
            100. - obs.used_pct.unwrap_or_default()
        ));
        unsafe {
            Shell_NotifyIconW(NIM_MODIFY, &data).ok()?;
        }
        self.history.sent(obs, now_ms, cooldown_ms);
        Ok(true)
    }
    pub fn handle_callback(
        &self,
        lparam: LPARAM,
        flags: TrayMenuState,
    ) -> Result<Option<TrayCommand>> {
        if (lparam.0 >> 16) as u16 != self.id {
            return Ok(None);
        }
        match lparam.0 as u16 as u32 {
            NIN_SELECT | 1025 => Ok(Some(TrayCommand::ToggleOverlay)),
            WM_CONTEXTMENU => {
                let mut point = POINT::default();
                unsafe {
                    let ident = NOTIFYICONIDENTIFIER {
                        cbSize: size_of::<NOTIFYICONIDENTIFIER>() as u32,
                        hWnd: self.owner,
                        uID: self.id.into(),
                        ..Default::default()
                    };
                    if let Ok(area) = Shell_NotifyIconGetRect(&ident) {
                        point.x = area.left;
                        point.y = area.top;
                    } else {
                        GetCursorPos(&mut point)?;
                    }
                }
                self.popup(point, flags)
            }
            _ => Ok(None),
        }
    }
    pub fn popup(&self, point: POINT, flags: TrayMenuState) -> Result<Option<TrayCommand>> {
        use DisplayPlacement::*;
        use TrayCommand::*;
        enum Item {
            Sep,
            Cmd(TrayCommand, &'static str, &'static str, bool),
        }
        unsafe {
            let menu = Menu(CreatePopupMenu()?);
            let entries = [
                Item::Cmd(OpenDetail, "打开详情面板", "Open detail dashboard", false),
                Item::Cmd(
                    ToggleOverlay,
                    "显示状态栏 / 组件",
                    "Show status widget",
                    flags.overlay_visible,
                ),
                Item::Sep,
                Item::Cmd(
                    SetPlacement(Taskbar),
                    "任务栏模式（贴靠任务栏）",
                    "Taskbar dock mode",
                    flags.placement == Taskbar,
                ),
                Item::Cmd(
                    SetPlacement(DesktopWidget),
                    "桌面组件模式（贴底不挡窗口）",
                    "Desktop widget (pinned to desktop)",
                    flags.placement == DesktopWidget,
                ),
                Item::Cmd(
                    SetPlacement(Floating),
                    "悬浮窗模式（自由置顶）",
                    "Floating overlay (on top)",
                    flags.placement == Floating,
                ),
                Item::Sep,
                Item::Cmd(
                    ToggleOverlayMode,
                    "切换紧凑 / 展开卡片",
                    "Switch compact / expanded",
                    false,
                ),
                Item::Cmd(
                    ToggleClickThrough,
                    "悬浮窗点击穿透",
                    "Overlay click-through",
                    flags.click_through,
                ),
                Item::Sep,
                Item::Cmd(RefreshNow, "立即刷新", "Refresh now", false),
                Item::Cmd(TogglePause, "暂停监控", "Pause monitoring", flags.paused),
                Item::Cmd(ToggleTheme, "切换深色 / 浅色主题", "Switch theme", false),
                Item::Cmd(
                    ToggleLanguage,
                    "切换语言 / Language",
                    "Language / 切换语言",
                    false,
                ),
                Item::Cmd(
                    ToggleAutostart,
                    "开机自启",
                    "Start on login",
                    flags.autostart,
                ),
                Item::Sep,
                Item::Cmd(Exit, "退出", "Exit", false),
            ];
            let mut commands = Vec::new();
            for item in &entries {
                match item {
                    Item::Sep => {
                        AppendMenuW(menu.0, MF_SEPARATOR, 0, PCWSTR::null())?;
                    }
                    Item::Cmd(cmd, zh, en, checked) => {
                        commands.push(*cmd);
                        let id = commands.len();
                        let text: Vec<_> = localized(self.language, zh, en)
                            .encode_utf16()
                            .chain(Some(0))
                            .collect();
                        AppendMenuW(
                            menu.0,
                            MF_STRING | if *checked { MF_CHECKED } else { MF_UNCHECKED },
                            id,
                            PCWSTR(text.as_ptr()),
                        )?;
                    }
                }
            }
            let _ = SetForegroundWindow(self.owner);
            let id = TrackPopupMenuEx(
                menu.0,
                (TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY).0,
                point.x,
                point.y,
                self.owner,
                None,
            )
            .0;
            let _ = PostMessageW(self.owner, WM_NULL, WPARAM(0), LPARAM(0));
            Ok(id
                .checked_sub(1)
                .and_then(|index| commands.get(index as usize))
                .copied())
        }
    }
}
impl Drop for TrayHost {
    fn drop(&mut self) {
        if self.registered {
            unsafe {
                let _ = Shell_NotifyIconW(NIM_DELETE, &self.data());
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use tokenpulse_core_domain::quota::QuotaSourceKind;
    #[test]
    fn cooldown_per_scope_and_expiry() {
        let mut obs = QuotaObservation {
            provider: ProviderId::Codex,
            scope_id: "local".into(),
            account_confirmed: true,
            target_id: "one".into(),
            target_label: "display".into(),
            period_kind: QuotaPeriodKind::FiveHour,
            used_pct: Some(92.),
            resets_at_ms: Some(110),
            event_time_ms: None,
            event_time_trusted: false,
            ingested_at_ms: 0,
            source_kind: QuotaSourceKind::LocalLogSession,
        };
        let mut history = AlertHistory::default();
        assert!(history.due(&obs, 8., 10));
        history.sent(&obs, 10, 60);
        assert!(!history.due(&obs, 8., 69));
        assert!(history.due(&obs, 8., 70));
        history.sent(&obs, 70, 60);
        obs.scope_id = "other".into();
        assert!(history.due(&obs, 8., 71));
        assert!(!history.due(&obs, 8., 110));
    }
    #[test]
    fn icon_and_utf16_edges() {
        assert_eq!(icon_pixel(TrayIconState::Healthy, 0, 0), 0);
        assert_ne!(
            icon_pixel(TrayIconState::Healthy, 26, 7),
            icon_pixel(TrayIconState::LowQuota, 26, 7)
        );
        assert_eq!(wide_field::<3>("x😀"), [120, 0, 0]);
    }
    #[test]
    #[ignore = "requires an interactive Windows shell"]
    fn native_tray_lifecycle() -> Result<()> {
        unsafe {
            FindWindowW(windows::core::w!("Shell_TrayWnd"), None).map_err(|_| {
                Error::new(E_FAIL, "Shell_TrayWnd is unavailable on the test desktop")
            })?;
        }
        let owner =
            crate::OverlayWindow::create(crate::OverlayMode::CompactCapsule, Some((40, 40)))?;
        let mut tray = TrayHost::new(owner.hwnd(), 123, TrayIconState::Healthy, Language::EnUs)?;
        for state in [
            TrayIconState::LowQuota,
            TrayIconState::Paused,
            TrayIconState::Offline,
        ] {
            tray.update(state, Language::ZhCn, "本地测试")?;
        }
        assert_eq!(
            tray.handle_callback(
                LPARAM((123 << 16) | NIN_SELECT as isize),
                TrayMenuState::default()
            )?,
            Some(TrayCommand::ToggleOverlay)
        );
        drop(tray);
        drop(owner);
        Ok(())
    }
}

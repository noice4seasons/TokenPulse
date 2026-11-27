use crate::{
    clamp_to_visible_monitors, localized, snap_to_work_area, DisplayPlacement, Language,
    OverlayGeometry, OverlayMode, OverlaySnapshotDto, Rgba, Theme, ThemePalette,
};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    ffi::c_void,
    mem::size_of,
    rc::{Rc, Weak},
    time::{SystemTime, UNIX_EPOCH},
};
use tokenpulse_core_domain::{
    dto::Coverage,
    quota::{QuotaPeriodKind, QuotaWindowState},
};
use windows::{
    core::{w, Error, Interface, Result, HRESULT},
    Win32::{
        Foundation::*,
        Graphics::{
            Direct2D::{Common::*, *},
            Direct3D::*,
            Direct3D11::*,
            DirectComposition::*,
            DirectWrite::*,
            Dxgi::{Common::*, *},
            Gdi::*,
        },
        System::{LibraryLoader::GetModuleHandleW, Registry::*, Time::*},
        UI::{
            Accessibility::*, HiDpi::*, Input::KeyboardAndMouse::*, Shell::*,
            WindowsAndMessaging::*,
        },
    },
};

pub const OVERLAY_ACTION_OPEN_DETAIL: usize = 1;
pub const OVERLAY_ACTION_CONTEXT_MENU: usize = 2;
pub const OVERLAY_ACTION_MODE_TOGGLED: usize = 3;

const HOUR_MS: i64 = 3_600_000;
const DAY_MS: i64 = 86_400_000;
const EPOCH_DIFF_MS: i64 = 11_644_473_600_000;

pub fn is_pro_plan(plan_type: Option<&str>) -> bool {
    plan_type.is_some_and(|raw| {
        let lower = raw.trim().to_ascii_lowercase();
        lower == "pro" || lower.starts_with("pro")
    })
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ResetBucket {
    DaysHours(i64, i64),
    HoursMins(i64, i64),
    Mins(i64),
}
impl ResetBucket {
    fn from_remaining_ms(remaining_ms: i64) -> (Self, i64) {
        if remaining_ms > DAY_MS {
            let total_hours = 1 + (remaining_ms - 1) / HOUR_MS;
            let wake = 1 + (remaining_ms - 1) % HOUR_MS;
            return (Self::DaysHours(total_hours / 24, total_hours % 24), wake);
        }
        let total_mins = 1 + (remaining_ms - 1) / 60_000;
        let wake = 1 + (remaining_ms - 1) % 60_000;
        let bucket = if total_mins >= 1440 {
            Self::DaysHours(1, 0)
        } else if total_mins >= 60 {
            Self::HoursMins(total_mins / 60, total_mins % 60)
        } else {
            Self::Mins(total_mins)
        };
        (bucket, wake)
    }
    fn format(self, lang: Language) -> String {
        match (self, lang) {
            (Self::DaysHours(d, 0), Language::ZhCn) => format!("{d}天"),
            (Self::DaysHours(d, h), Language::ZhCn) => format!("{d}天{h}时"),
            (Self::DaysHours(d, 0), Language::EnUs) => format!("{d}d"),
            (Self::DaysHours(d, h), Language::EnUs) => format!("{d}d {h}h"),
            (Self::HoursMins(h, 0), Language::ZhCn) => format!("{h}小时"),
            (Self::HoursMins(h, m), Language::ZhCn) => format!("{h}时{m}分"),
            (Self::HoursMins(h, 0), Language::EnUs) => format!("{h}h"),
            (Self::HoursMins(h, m), Language::EnUs) => format!("{h}h {m}m"),
            (Self::Mins(m), Language::ZhCn) => format!("{m}分钟"),
            (Self::Mins(m), Language::EnUs) => format!("{m} min"),
        }
    }
}

pub fn format_reset_duration(remaining_ms: i64, lang: Language) -> String {
    if remaining_ms <= 0 {
        return localized(lang, "0分钟", "0 min").into();
    }
    ResetBucket::from_remaining_ms(remaining_ms).0.format(lang)
}

fn unix_ms_to_local_systemtime(unix_ms: i64) -> Option<SYSTEMTIME> {
    let ticks = unix_ms.checked_add(EPOCH_DIFF_MS)?.checked_mul(10_000)?;
    if ticks < 0 {
        return None;
    }
    let ft = FILETIME {
        dwLowDateTime: ticks as u32,
        dwHighDateTime: (ticks >> 32) as u32,
    };
    let mut utc = SYSTEMTIME::default();
    let mut local = SYSTEMTIME::default();
    unsafe {
        FileTimeToSystemTime(&ft, &mut utc).ok()?;
        SystemTimeToTzSpecificLocalTime(None, &utc, &mut local).ok()?;
    }
    Some(local)
}

pub fn format_local_reset_clock(reset_at_ms: i64, now_ms: i64) -> Option<String> {
    let target = unix_ms_to_local_systemtime(reset_at_ms)?;
    let current = unix_ms_to_local_systemtime(now_ms)?;
    if target.wYear == current.wYear
        && target.wMonth == current.wMonth
        && target.wDay == current.wDay
    {
        Some(format!("{:02}:{:02}", target.wHour, target.wMinute))
    } else {
        Some(format!(
            "{:02}/{:02} {:02}:{:02}",
            target.wMonth, target.wDay, target.wHour, target.wMinute
        ))
    }
}

#[derive(Clone, PartialEq, Debug)]
enum Quota {
    Active {
        remaining_pct: f64,
        bucket: Option<ResetBucket>,
        clock: Option<String>,
    },
    Expired,
    Restricted,
    Missing,
}
impl Quota {
    fn percent(&self) -> String {
        match self {
            Self::Active { remaining_pct, .. } => format!("{remaining_pct:.0}%"),
            _ => "—".into(),
        }
    }
    fn short_duration(&self, lang: Language) -> Option<String> {
        match self {
            Self::Active { bucket, .. } => bucket.map(|b| b.format(lang)),
            _ => None,
        }
    }
    fn caption(&self, lang: Language) -> String {
        match self {
            Self::Active {
                bucket: Some(b),
                clock: Some(clock),
                ..
            } => {
                let dur = b.format(lang);
                match lang {
                    Language::ZhCn => format!("{} · {}后 ({clock})", self.percent(), dur),
                    Language::EnUs => format!("{} · {} ({clock})", self.percent(), dur),
                }
            }
            Self::Active {
                bucket: Some(b),
                clock: None,
                ..
            } => {
                let dur = b.format(lang);
                match lang {
                    Language::ZhCn => format!("{} · {}后", self.percent(), dur),
                    Language::EnUs => format!("{} · {}", self.percent(), dur),
                }
            }
            Self::Expired => {
                localized(lang, "已过期 · 待新活动", "Expired · awaiting activity").into()
            }
            Self::Restricted => localized(lang, "套餐限制", "Plan restricted").into(),
            _ => self.percent(),
        }
    }
    fn tone(&self, default_ink: Rgba, palette: &ThemePalette) -> Rgba {
        match self {
            Self::Active { remaining_pct, .. } if *remaining_pct <= 10. => palette.danger,
            Self::Active { remaining_pct, .. } if *remaining_pct <= 25. => palette.warn,
            Self::Expired => palette.warn,
            _ => default_ink,
        }
    }
}
fn ratio(value: Option<f64>) -> String {
    value
        .filter(|n| (0.0..=1.0).contains(n))
        .map(|n| format!("{:.0}%", n * 100.))
        .unwrap_or_else(|| "—".into())
}
fn count(value: Option<u64>) -> String {
    value
        .map(|n| match n {
            1_000_000_000.. => format!("{:.1}B", n as f64 / 1e9),
            1_000_000.. => format!("{:.1}M", n as f64 / 1e6),
            10_000.. => format!("{:.1}K", n as f64 / 1e3),
            _ => n.to_string(),
        })
        .unwrap_or_else(|| "—".into())
}

#[derive(Clone, PartialEq, Debug)]
struct Frame {
    placement: DisplayPlacement,
    mode: OverlayMode,
    theme: Theme,
    lang: Language,
    opacity: u32,
    hide_five_hour: bool,
    bars: [Quota; 2],
    today: String,
    latest: String,
    total: String,
    ag: String,
}
impl Frame {
    fn layout_dip(&self) -> (i32, i32, i32, i32, f32) {
        if self.placement == DisplayPlacement::Taskbar {
            let w = if self.hide_five_hour { 198 } else { 218 };
            return (w, 38, w - 2, 36, 8.);
        }
        match self.mode {
            OverlayMode::CompactCapsule => (244, 46, 236, 44, 22.),
            OverlayMode::ExpandedCard => {
                let h = if self.hide_five_hour { 204 } else { 238 };
                (332, h, 328, h, 14.)
            }
        }
    }
    fn size_dip(&self) -> (i32, i32) {
        let (w, h, _, _, _) = self.layout_dip();
        (w, h)
    }
    fn footer_top_dip(&self) -> i32 {
        if self.hide_five_hour {
            172
        } else {
            206
        }
    }
}

#[derive(Clone)]
struct Model {
    placement: DisplayPlacement,
    mode: OverlayMode,
    theme: Theme,
    lang: Language,
    opacity: u32,
    snapshot: Option<OverlaySnapshotDto>,
    ag: Option<u64>,
}
impl Default for Model {
    fn default() -> Self {
        Self {
            placement: DisplayPlacement::Floating,
            mode: OverlayMode::CompactCapsule,
            theme: Theme::Dark,
            lang: Language::ZhCn,
            opacity: 90,
            snapshot: None,
            ag: None,
        }
    }
}
impl Model {
    fn frame(&self, now: i64) -> (Frame, Option<u32>) {
        let snapshot = self.snapshot.as_ref();
        let taskbar = self.placement == DisplayPlacement::Taskbar;
        let compact = !taskbar && self.mode == OverlayMode::CompactCapsule;
        let has_five_hour = snapshot.is_some_and(|dto| {
            dto.codex
                .iter()
                .any(|bar| bar.period_kind == QuotaPeriodKind::FiveHour)
        });
        let has_weekly = snapshot.is_some_and(|dto| {
            dto.codex
                .iter()
                .any(|bar| bar.period_kind == QuotaPeriodKind::Weekly)
        });
        let hide_five_hour =
            is_pro_plan(snapshot.and_then(|dto| dto.codex_plan_type.as_deref()))
                || (!has_five_hour && has_weekly);

        let show_clock = !taskbar && !compact;
        let mut wake = None;
        let bars = [QuotaPeriodKind::FiveHour, QuotaPeriodKind::Weekly].map(|kind| {
            if hide_five_hour && kind == QuotaPeriodKind::FiveHour {
                return Quota::Missing;
            }
            let Some((dto, bar)) = snapshot.and_then(|dto| {
                dto.codex
                    .iter()
                    .find(|bar| bar.period_kind == kind)
                    .map(|bar| (dto, bar))
            }) else {
                return Quota::Missing;
            };
            match bar.state {
                QuotaWindowState::Active {
                    remaining_pct,
                    resets_in_ms,
                    ..
                } if (0.0..=100.0).contains(&remaining_pct) => {
                    let reset_at = resets_in_ms.map(|ms| dto.updated_at_ms.saturating_add(ms));
                    let remaining = reset_at.map(|end| end.saturating_sub(now));
                    if remaining.is_some_and(|ms| ms <= 0) {
                        return Quota::Expired;
                    }
                    let show_duration = !compact || (hide_five_hour && kind == QuotaPeriodKind::Weekly);
                    let mut bucket = None;
                    let mut clock = None;
                    if let Some(ms) = remaining {
                        let (b, dur_wake) = ResetBucket::from_remaining_ms(ms);
                        let next = if show_duration { dur_wake } else { ms };
                        let delay = next.clamp(1, i32::MAX.into()) as u32;
                        wake = Some(wake.map_or(delay, |prev: u32| prev.min(delay)));
                        if show_duration {
                            bucket = Some(b);
                            if show_clock {
                                clock = reset_at.and_then(|end| format_local_reset_clock(end, now));
                            }
                        }
                    }
                    Quota::Active {
                        remaining_pct,
                        bucket,
                        clock,
                    }
                }
                QuotaWindowState::AwaitingNewActivity { .. } => Quota::Expired,
                QuotaWindowState::PlanRestricted { .. } => Quota::Restricted,
                _ => Quota::Missing,
            }
        });
        let mut today = count(snapshot.and_then(|dto| dto.today_tokens));
        if today != "—" && snapshot.is_some_and(|dto| dto.today_coverage == Coverage::Partial) {
            today.insert(0, '≥');
        }
        let show_ag = taskbar || !compact;
        (
            Frame {
                placement: self.placement,
                mode: self.mode,
                theme: self.theme,
                lang: self.lang,
                opacity: self.opacity,
                hide_five_hour,
                bars,
                today,
                latest: ratio(snapshot.and_then(|dto| dto.latest_turn_cache_hit_ratio)),
                total: if compact || taskbar {
                    String::new()
                } else {
                    ratio(snapshot.and_then(|dto| dto.cache_hit_ratio))
                },
                ag: if show_ag {
                    count(self.ag)
                } else {
                    String::new()
                },
            },
            wake,
        )
    }
}
fn dirty(last: Option<&Frame>, next: &Frame) -> bool {
    last != Some(next)
}
unsafe fn d3d(kind: D3D_DRIVER_TYPE) -> Result<ID3D11Device> {
    let mut device = None;
    D3D11CreateDevice(
        None,
        kind,
        HMODULE::default(),
        D3D11_CREATE_DEVICE_BGRA_SUPPORT,
        None,
        D3D11_SDK_VERSION,
        Some(&mut device),
        None,
        None,
    )?;
    device.ok_or(Error::from_hresult(E_POINTER))
}

fn rect(left: f32, top: f32, right: f32, bottom: f32) -> D2D_RECT_F {
    D2D_RECT_F {
        left,
        top,
        right,
        bottom,
    }
}
struct Pulse {
    surface: IDCompositionSurface,
    effect: IDCompositionEffectGroup,
    side: u32,
}
impl Pulse {
    unsafe fn new(
        composition: &IDCompositionDesktopDevice,
        parent: &IDCompositionVisual2,
        device: &ID2D1Device,
        dpi: u32,
        x_dip: f32,
        y_dip: f32,
    ) -> Result<Self> {
        let visual = composition.CreateVisual()?;
        let effect = composition.CreateEffectGroup()?;
        let scale = dpi as f32 / 96.;
        let side = (16. * scale).ceil() as u32;
        let surface = composition.CreateSurfaceFactory(device)?.CreateSurface(
            side,
            side,
            DXGI_FORMAT_B8G8R8A8_UNORM,
            DXGI_ALPHA_MODE_PREMULTIPLIED,
        )?;
        visual.SetOffsetX2(x_dip * scale)?;
        visual.SetOffsetY2(y_dip * scale)?;
        visual.SetContent(&surface)?;
        visual.SetEffect(&effect)?;
        parent.AddVisual(&visual, true, None)?;
        Ok(Self {
            surface,
            effect,
            side,
        })
    }
    unsafe fn paint(&self, device: &IDCompositionDesktopDevice, color: Rgba) -> Result<()> {
        let mut origin = POINT::default();
        let ctx: ID2D1DeviceContext = self.surface.BeginDraw(None, &mut origin)?;
        let draw = (|| -> Result<()> {
            let rt: ID2D1RenderTarget = ctx.cast()?;
            rt.SetDpi(96., 96.);
            rt.Clear(Some(&D2D1_COLOR_F::default()));
            let brush = solid_brush(&rt)?;
            brush.SetColor(&D2D1_COLOR_F {
                r: color[0],
                g: color[1],
                b: color[2],
                a: 1.,
            });
            let radius = self.side as f32 / 2.;
            rt.FillEllipse(
                &D2D1_ELLIPSE {
                    point: D2D_POINT_2F {
                        x: origin.x as f32 + radius,
                        y: origin.y as f32 + radius,
                    },
                    radiusX: radius,
                    radiusY: radius,
                },
                &brush,
            );
            Ok(())
        })();
        let end = self.surface.EndDraw();
        draw?;
        end?;
        let pulse = device.CreateAnimation()?;
        pulse.AddCubic(0., 0., 1., 0., 0.)?;
        pulse.AddCubic(0.32, 0.32, -0.5, 0., 0.)?;
        pulse.End(0.96, 0.)?;
        self.effect.SetOpacity(&pulse)
    }
}
struct Renderer {
    context: ID2D1DeviceContext,
    chain: IDXGISwapChain1,
    composition: IDCompositionDesktopDevice,
    _target: IDCompositionTarget,
    effect: IDCompositionEffectGroup,
    opacity: Cell<Option<f32>>,
    pulse: Option<Pulse>,
    brush: ID2D1SolidColorBrush,
    formats_left: Vec<IDWriteTextFormat>,
    formats_right: Vec<IDWriteTextFormat>,
}
impl Renderer {
    unsafe fn new(
        device: &ID3D11Device,
        hwnd: HWND,
        width: u32,
        height: u32,
        dpi: u32,
        pulse_pos_dip: Option<(f32, f32)>,
    ) -> Result<Self> {
        let dxgi: IDXGIDevice = device.cast()?;
        let factory: IDXGIFactory2 = dxgi.GetAdapter()?.GetParent()?;
        let chain = factory.CreateSwapChainForComposition(
            device,
            &DXGI_SWAP_CHAIN_DESC1 {
                Width: width,
                Height: height,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                Scaling: DXGI_SCALING_STRETCH,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
                AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
                ..Default::default()
            },
            None,
        )?;
        let factory: ID2D1Factory1 = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
        let context = factory
            .CreateDevice(&dxgi)?
            .CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;
        let surface: IDXGISurface = chain.GetBuffer(0)?;
        let bitmap = context.CreateBitmapFromDxgiSurface(
            &surface,
            Some(&D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
                dpiX: dpi as f32,
                dpiY: dpi as f32,
                ..Default::default()
            }),
        )?;
        context.SetTarget(&bitmap);
        context.SetDpi(dpi as f32, dpi as f32);
        context.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
        let composition: IDCompositionDesktopDevice = DCompositionCreateDevice2(&dxgi)?;
        let target = composition.CreateTargetForHwnd(hwnd, true)?;
        let visual = composition.CreateVisual()?;
        let effect = composition.CreateEffectGroup()?;

        visual.SetEffect(&effect)?;
        visual.SetContent(&chain)?;
        target.SetRoot(&visual)?;
        let write: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
        let mut formats_left = Vec::new();
        let mut formats_right = Vec::new();
        for (family, locale) in [
            (w!("Segoe UI"), w!("en-US")),
            (w!("Microsoft YaHei UI"), w!("zh-CN")),
        ] {
            for (size, weight) in [
                (10.5, DWRITE_FONT_WEIGHT_NORMAL),
                (12.5, DWRITE_FONT_WEIGHT_NORMAL),
                (14.5, DWRITE_FONT_WEIGHT_SEMI_BOLD),
                (19.0, DWRITE_FONT_WEIGHT_SEMI_BOLD),
            ] {
                for (align, bucket) in [
                    (DWRITE_TEXT_ALIGNMENT_LEADING, &mut formats_left),
                    (DWRITE_TEXT_ALIGNMENT_TRAILING, &mut formats_right),
                ] {
                    let format = write.CreateTextFormat(
                        family,
                        None,
                        weight,
                        DWRITE_FONT_STYLE_NORMAL,
                        DWRITE_FONT_STRETCH_NORMAL,
                        size,
                        locale,
                    )?;
                    format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
                    format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
                    format.SetTextAlignment(align)?;
                    bucket.push(format);
                }
            }
        }
        let brush = solid_brush(&context.cast()?)?;
        let pulse = match pulse_pos_dip {
            Some((x, y)) => Some(Pulse::new(
                &composition,
                &visual,
                &context.GetDevice()?,
                dpi,
                x,
                y,
            )?),
            None => None,
        };
        Ok(Self {
            context,
            chain,
            composition,
            _target: target,
            effect,
            opacity: Cell::new(None),
            pulse,
            brush,
            formats_left,
            formats_right,
        })
    }
}

#[repr(C)]
struct BrushVtablePrefix {
    resource: ID2D1Resource_Vtbl,
    bitmap_methods: [usize; 4],
    create_solid: unsafe extern "system" fn(
        *mut c_void,
        *const D2D1_COLOR_F,
        *const c_void,
        *mut *mut c_void,
    ) -> HRESULT,
}
const _: () = assert!(
    size_of::<BrushVtablePrefix>()
        == std::mem::offset_of!(ID2D1RenderTarget_Vtbl, CreateGradientStopCollection)
);
unsafe fn solid_brush(target: &ID2D1RenderTarget) -> Result<ID2D1SolidColorBrush> {
    let prefix = &*(target.vtable() as *const ID2D1RenderTarget_Vtbl).cast::<BrushVtablePrefix>();
    let mut brush = std::ptr::null_mut();
    (prefix.create_solid)(
        target.as_raw(),
        &D2D1_COLOR_F::default(),
        std::ptr::null(),
        &mut brush,
    )
    .ok()?;
    if brush.is_null() {
        return Err(Error::from_hresult(E_POINTER));
    }
    Ok(ID2D1SolidColorBrush::from_raw(brush))
}
struct Canvas<'a> {
    rt: ID2D1RenderTarget,
    brush: &'a ID2D1SolidColorBrush,
    formats_left: &'a [IDWriteTextFormat],
    formats_right: &'a [IDWriteTextFormat],
}
impl Canvas<'_> {
    unsafe fn ink(&self, [r, g, b, a]: Rgba) {
        self.brush.SetColor(&D2D1_COLOR_F { r, g, b, a });
    }
    unsafe fn round(&self, area: D2D_RECT_F, radius: f32, color: Rgba) {
        self.ink(color);
        self.rt.FillRoundedRectangle(
            &D2D1_ROUNDED_RECT {
                rect: area,
                radiusX: radius,
                radiusY: radius,
            },
            self.brush,
        );
    }
    unsafe fn text(&self, text: &str, area: D2D_RECT_F, size: usize, color: Rgba) {
        self.text_aligned(text, area, size, color, false);
    }
    unsafe fn text_right(&self, text: &str, area: D2D_RECT_F, size: usize, color: Rgba) {
        self.text_aligned(text, area, size, color, true);
    }
    unsafe fn text_aligned(
        &self,
        text: &str,
        area: D2D_RECT_F,
        size: usize,
        color: Rgba,
        right: bool,
    ) {
        self.ink(color);
        let format = if right {
            &self.formats_right[size]
        } else {
            &self.formats_left[size]
        };
        self.rt.DrawText(
            &text.encode_utf16().collect::<Vec<_>>(),
            format,
            &area,
            self.brush,
            D2D1_DRAW_TEXT_OPTIONS_CLIP,
            DWRITE_MEASURING_MODE_NATURAL,
        );
    }
    unsafe fn ring_at(
        &self,
        cx: f32,
        cy: f32,
        radius: f32,
        stroke: f32,
        quota: &Quota,
        ink: Rgba,
        p: &ThemePalette,
    ) {
        let ellipse = D2D1_ELLIPSE {
            point: D2D_POINT_2F { x: cx, y: cy },
            radiusX: radius,
            radiusY: radius,
        };
        self.ink(if *quota == Quota::Expired {
            p.warn
        } else {
            p.track
        });
        self.rt.DrawEllipse(&ellipse, self.brush, stroke, None);
        if let Quota::Active { remaining_pct, .. } = quota {
            self.ink(quota.tone(ink, p));
            let angle = (*remaining_pct as f32 / 100.) * std::f32::consts::TAU;
            let steps = (angle * radius).ceil() as u32;
            let point = |index: u32| {
                let phase = angle * index as f32 / steps as f32;
                D2D_POINT_2F {
                    x: cx + phase.sin() * radius,
                    y: cy - phase.cos() * radius,
                }
            };
            for index in 0..steps {
                self.rt
                    .DrawLine(point(index), point(index + 1), self.brush, stroke, None);
            }
        }
    }
}
impl Renderer {
    unsafe fn draw(&self, frame: &Frame) -> Result<()> {
        let rt: ID2D1RenderTarget = self.context.cast()?;
        let palette = ThemePalette::for_theme(frame.theme);
        let chinese = frame.lang == Language::ZhCn;
        let offset = if chinese { 4 } else { 0 };
        let local = |zh, en| localized(frame.lang, zh, en);
        let canvas = Canvas {
            rt,
            brush: &self.brush,
            formats_left: &self.formats_left[offset..],
            formats_right: &self.formats_right[offset..],
        };
        let panel = &canvas;
        let opacity = if frame.placement == DisplayPlacement::Taskbar {
            (frame.opacity.max(85) as f32) / 100.
        } else {
            frame.opacity as f32 / 100.
        };
        match self.opacity.get() {
            Some(previous) if previous != opacity => {
                let fade = self.composition.CreateAnimation()?;
                fade.AddCubic(0., previous, (opacity - previous) / 0.16, 0., 0.)?;
                fade.End(0.16, opacity)?;
                self.effect.SetOpacity(&fade)?;
            }
            None => self.effect.SetOpacity2(opacity)?,
            _ => {}
        }
        self.opacity.set(Some(opacity));
        panel.rt.BeginDraw();
        panel.rt.Clear(Some(&D2D1_COLOR_F::default()));

        let (width, height, sw, sh, radius) = frame.layout_dip();
        let left = (width - sw) as f32 / 2.;
        let top = (height - sh) as f32 / 2.;
        let area = rect(
            left + 0.5,
            top + 0.5,
            left + sw as f32 - 0.5,
            top + sh as f32 - 0.5,
        );
        panel.round(area, radius, palette.bg_overlay);
        panel.ink(if frame.placement == DisplayPlacement::Taskbar {
            palette.border_subtle
        } else {
            palette.border_strong
        });
        panel.rt.DrawRoundedRectangle(
            &D2D1_ROUNDED_RECT {
                rect: area,
                radiusX: radius,
                radiusY: radius,
            },
            panel.brush,
            1.,
            None,
        );

        let active_bars: &[Quota] = if frame.hide_five_hour {
            &frame.bars[1..2]
        } else {
            &frame.bars[..]
        };
        let (status, color) = if active_bars.contains(&Quota::Expired) {
            (local("待新活动", "Expired"), palette.warn)
        } else if active_bars
            .iter()
            .any(|q| matches!(q, Quota::Active { remaining_pct, .. } if *remaining_pct <= 10.))
        {
            (local("低额度", "Low quota"), palette.danger)
        } else if active_bars
            .iter()
            .any(|q| matches!(q, Quota::Active { .. }))
        {
            (local("正常", "Active"), palette.ok)
        } else {
            (local("暂无额度", "No quota"), palette.text_muted)
        };

        if frame.placement == DisplayPlacement::Taskbar {
            let cy = height as f32 / 2.;
            if frame.hide_five_hour {
                panel.ring_at(20., cy, 10.5, 2.8, &frame.bars[1], palette.accent_codex, &palette);
            } else {
                panel.ring_at(20., cy, 11.0, 2.6, &frame.bars[0], palette.accent_codex, &palette);
                panel.ring_at(20., cy, 6.2, 2.4, &frame.bars[1], palette.accent_ag, &palette);
            }
            let headline = if frame.hide_five_hour {
                match frame.bars[1].short_duration(frame.lang) {
                    Some(dur) => format!("7d {} · {dur}", frame.bars[1].percent()),
                    None => format!("7d {} {}", local("剩余", "Left"), frame.bars[1].percent()),
                }
            } else {
                format!("5h {} · 7d {}", frame.bars[0].percent(), frame.bars[1].percent())
            };
            let subline = if !frame.ag.is_empty() && frame.ag != "—" && frame.ag != "0" {
                format!("{} {} · AG {}", local("今日", "Today"), frame.today, frame.ag)
            } else {
                format!(
                    "{} {} · {} {}",
                    local("今日", "Today"),
                    frame.today,
                    local("缓存", "Cache"),
                    frame.latest
                )
            };
            let text_right = (width - 20) as f32;
            panel.text(&headline, rect(38., 2., text_right, 20.), 1, palette.text_primary);
            panel.text(&subline, rect(38., 19., text_right, 36.), 0, palette.text_secondary);
            let dot_x = (width - 14) as f32;
            let mut halo = color;
            halo[3] = 0.2;
            panel.round(rect(dot_x - 4., cy - 6., dot_x + 8., cy + 6.), 6., halo);
            panel.round(rect(dot_x - 1., cy - 3., dot_x + 5., cy + 3.), 3., color);
        } else if frame.mode == OverlayMode::CompactCapsule {
            if frame.hide_five_hour {
                panel.ring_at(29., 23., 11.0, 3.0, &frame.bars[1], palette.accent_codex, &palette);
                let headline = match frame.bars[1].short_duration(frame.lang) {
                    Some(dur) => format!("7d {} · {dur}", frame.bars[1].percent()),
                    None => format!("7d {} {}", local("剩余", "Left"), frame.bars[1].percent()),
                };
                panel.text(&headline, rect(51., 2., 212., 24.), 2, palette.text_primary);
            } else {
                panel.ring_at(29., 23., 11.5, 3.0, &frame.bars[0], palette.accent_codex, &palette);
                panel.ring_at(29., 23., 6.5, 2.8, &frame.bars[1], palette.accent_ag, &palette);
                panel.text(
                    &format!(
                        "{} {} / {}",
                        local("剩余", "Left"),
                        frame.bars[0].percent(),
                        frame.bars[1].percent()
                    ),
                    rect(51., 2., 212., 24.),
                    2,
                    palette.text_primary,
                );
            }
            panel.text(
                &format!(
                    "{} {} · {} {}",
                    local("今日", "Today"),
                    frame.today,
                    local("本轮缓存", "Cache"),
                    frame.latest
                ),
                rect(51., 23., 212., 43.),
                0,
                palette.text_secondary,
            );
            let mut halo = color;
            halo[3] = 0.18;
            panel.round(rect(216., 16., 230., 30.), 7., halo);
            panel.round(rect(220., 20., 226., 26.), 3., color);
        } else {
            panel.text(
                "TokenPulse",
                rect(16., 8., 205., 36.),
                3,
                palette.text_primary,
            );
            panel.round(rect(236., 12., 314., 33.), 10., palette.track);
            panel.text_right(status, rect(236., 12., 306., 33.), 0, color);

            let visible_rows: &[(&str, &Quota)] = if frame.hide_five_hour {
                &[("Codex 7d", &frame.bars[1])]
            } else {
                &[("Codex 5h", &frame.bars[0]), ("Codex 7d", &frame.bars[1])]
            };
            for (index, (label, quota)) in visible_rows.iter().enumerate() {
                let y = 43. + index as f32 * 35.;
                panel.text(label, rect(16., y, 92., y + 18.), 1, palette.text_primary);
                panel.text_right(
                    &quota.caption(frame.lang),
                    rect(94., y, 314., y + 18.),
                    0,
                    if **quota == Quota::Expired {
                        palette.warn
                    } else {
                        palette.text_secondary
                    },
                );
                panel.round(rect(16., y + 21., 314., y + 27.), 3., palette.track);
                if let Quota::Active { remaining_pct, .. } = quota {
                    if *remaining_pct > 0. {
                        let fill_w = (298. * (*remaining_pct as f32 / 100.)).max(4.);
                        panel.round(
                            rect(16., y + 21., 16. + fill_w, y + 27.),
                            3.,
                            quota.tone(palette.accent_codex, &palette),
                        );
                    }
                }
            }

            let y_base = if frame.hide_five_hour { 83. } else { 117. };
            panel.text(
                local("今日总 Token", "Today's tokens"),
                rect(16., y_base, 155., y_base + 28.),
                1,
                palette.text_secondary,
            );
            panel.text_right(
                &frame.today,
                rect(155., y_base, 314., y_base + 28.),
                3,
                palette.text_primary,
            );
            panel.text(
                &format!(
                    "{} {}   ·   {} {}",
                    local("本轮缓存", "Turn cache"),
                    frame.latest,
                    local("累计", "Overall"),
                    frame.total
                ),
                rect(16., y_base + 30., 314., y_base + 52.),
                1,
                palette.text_secondary,
            );
            panel.text(
                &format!("Antigravity · {}", local("离线 Token", "Offline tokens")),
                rect(16., y_base + 58., 220., y_base + 81.),
                1,
                palette.accent_ag,
            );
            panel.text_right(
                &frame.ag,
                rect(220., y_base + 58., 314., y_base + 81.),
                1,
                palette.accent_ag,
            );

            let footer_y = frame.footer_top_dip() as f32;
            panel.round(rect(16., footer_y, 314., footer_y + 1.), 0., palette.border_subtle);
            panel.text(
                local("打开详情 ↗", "Open details ↗"),
                rect(16., footer_y + 4., 160., footer_y + 28.),
                0,
                palette.accent_codex,
            );
            panel.text_right(
                local("切换紧凑胶囊", "Compact mode"),
                rect(168., footer_y + 4., 314., footer_y + 28.),
                0,
                palette.text_secondary,
            );
        }
        panel.rt.EndDraw(None, None)?;
        if let Some(pulse) = &self.pulse {
            pulse.paint(&self.composition, color)?;
        }
        self.chain.Present(1, DXGI_PRESENT(0)).ok()?;
        self.composition.Commit()
    }
}

const CLASS: windows::core::PCWSTR = w!("TokenPulse.Overlay.1");
const POSITION_KEY: windows::core::PCWSTR = w!("Software\\TokenPulse\\Overlay");
const COUNTDOWN: usize = 1;
const TASKBAR_GUARD: usize = 2;
thread_local! {static WINDOWS:RefCell<HashMap<usize,Weak<Shared>>>=RefCell::new(HashMap::new());}
fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}
fn window_rect(hwnd: HWND) -> Result<RECT> {
    let mut area = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut area)? }
    Ok(area)
}
fn work_area(area: RECT) -> Result<RECT> {
    unsafe {
        let monitor = MonitorFromRect(&area, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        GetMonitorInfoW(monitor, &mut info).ok()?;
        Ok(info.rcWork)
    }
}
fn taskbar_geometry() -> Option<(HWND, RECT, Option<RECT>, bool)> {
    unsafe {
        let taskbar = FindWindowW(w!("Shell_TrayWnd"), None).ok()?;
        let visible = IsWindowVisible(taskbar).as_bool();
        let mut tb_rect = RECT::default();
        GetWindowRect(taskbar, &mut tb_rect).ok()?;
        let monitor = MonitorFromRect(&tb_rect, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let auto_hidden = if GetMonitorInfoW(monitor, &mut info).as_bool() {
            let h = tb_rect.bottom - tb_rect.top;
            h <= 6 || tb_rect.top >= info.rcMonitor.bottom - 4
        } else {
            false
        };
        let tray_rect = FindWindowExW(taskbar, None, w!("TrayNotifyWnd"), None)
            .ok()
            .and_then(|tray| {
                let mut r = RECT::default();
                GetWindowRect(tray, &mut r).ok().map(|()| r)
            });
        Some((taskbar, tb_rect, tray_rect, visible && !auto_hidden))
    }
}
fn fullscreen_active() -> bool {
    unsafe {
        SHQueryUserNotificationState().is_ok_and(|state| {
            matches!(
                state,
                QUNS_RUNNING_D3D_FULL_SCREEN | QUNS_PRESENTATION_MODE
            )
        })
    }
}
fn is_window_above(candidate: HWND, reference: HWND) -> bool {
    unsafe {
        let mut current = GetWindow(reference, GW_HWNDPREV).ok();
        let mut steps = 0;
        while let Some(hwnd) = current {
            if hwnd == candidate {
                return true;
            }
            steps += 1;
            if steps > 64 {
                break;
            }
            current = GetWindow(hwnd, GW_HWNDPREV).ok();
        }
        false
    }
}
fn saved_position() -> Option<(i32, i32)> {
    unsafe {
        let mut pos = [0i32; 2];
        let mut size = 8;
        let status = RegGetValueW(
            HKEY_CURRENT_USER,
            POSITION_KEY,
            w!("PositionPx"),
            RRF_RT_REG_BINARY,
            None,
            Some(pos.as_mut_ptr().cast()),
            Some(&mut size),
        );
        (status == ERROR_SUCCESS && size == 8).then_some((pos[0], pos[1]))
    }
}
fn save_position(x: i32, y: i32) -> Result<()> {
    let pos = [x, y];
    unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            POSITION_KEY,
            w!("PositionPx"),
            REG_BINARY.0,
            Some(pos.as_ptr().cast()),
            8,
        )
        .ok()
    }
}

#[derive(Default)]
struct Graphics {
    renderer: Option<Renderer>,
    surface: (i32, i32, u32, bool),
    last: Option<Frame>,
}
struct Shared {
    hwnd: HWND,
    model: RefCell<Model>,
    graphics: RefCell<Graphics>,
    visible: Cell<bool>,
    fullscreen_hidden: Cell<bool>,
    topmost: Cell<bool>,
    dpi: Cell<u32>,
    scale: Cell<u32>,
    count: Cell<u64>,
    drag: Cell<Option<(POINT, RECT)>>,
    moved: Cell<bool>,
    timer: Cell<bool>,
    taskbar_timer: Cell<bool>,
    taskbar_offset_x: Cell<Option<i32>>,
    owner_taskbar: Cell<isize>,
    win_event_hook: Cell<isize>,
    taskbar_created_msg: Cell<u32>,
    click_through: Cell<bool>,
    action_host: Cell<Option<(HWND, u32)>>,
    error: RefCell<Option<Error>>,
}
impl Shared {
    fn effective_dpi(&self) -> u32 {
        ((u64::from(self.dpi.get()) * u64::from(self.scale.get()) + 50) / 100)
            .clamp(1, u32::MAX.into()) as u32
    }
    fn px(&self, dip: i32) -> i32 {
        OverlayGeometry::dip_to_px(dip, self.effective_dpi())
    }
    fn post_action(&self, action: usize) {
        if let Some((host, msg)) = self.action_host.get() {
            unsafe {
                let _ = PostMessageW(host, msg, WPARAM(action), LPARAM(0));
            }
        }
    }
    fn cancel_timer(&self) {
        if self.timer.replace(false) {
            unsafe {
                let _ = KillTimer(self.hwnd, COUNTDOWN);
            }
        }
    }
    fn sync_taskbar_owner(&self, taskbar: Option<HWND>) {
        let want = if self.model.borrow().placement == DisplayPlacement::Taskbar {
            taskbar.map_or(0, |h| h.0 as isize)
        } else {
            0
        };
        if self.owner_taskbar.get() != want {
            unsafe {
                SetWindowLongPtrW(self.hwnd, GWLP_HWNDPARENT, want);
            }
            self.owner_taskbar.set(want);
        }
    }
    fn update_taskbar_timer(&self) {
        let want = self.visible.get() && self.model.borrow().placement == DisplayPlacement::Taskbar;
        if want && !self.taskbar_timer.get() {
            if unsafe { SetTimer(self.hwnd, TASKBAR_GUARD, 1500, None) } != 0 {
                self.taskbar_timer.set(true);
            }
        } else if !want && self.taskbar_timer.replace(false) {
            unsafe {
                let _ = KillTimer(self.hwnd, TASKBAR_GUARD);
            }
        }
        if want && self.win_event_hook.get() == 0 {
            let hook = unsafe {
                SetWinEventHook(
                    EVENT_SYSTEM_FOREGROUND,
                    EVENT_SYSTEM_FOREGROUND,
                    None,
                    Some(foreground_hook_proc),
                    0,
                    0,
                    WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
                )
            };
            if !hook.is_invalid() {
                self.win_event_hook.set(hook.0 as isize);
            }
        } else if !want {
            let raw = self.win_event_hook.replace(0);
            if raw != 0 {
                unsafe {
                    let _ = UnhookWinEvent(HWINEVENTHOOK(raw as _));
                }
            }
        }
    }
    fn compute_taskbar_pos(&self, width: i32, height: i32) -> Option<(i32, i32, bool)> {
        let (_, tb, tray, shown) = taskbar_geometry()?;
        let gap = self.px(8);
        let pad = self.px(6);
        let right_bound = match tray {
            Some(tr) if tr.left > tb.left + width + gap => tr.left - width - gap,
            _ => tb.right - width - gap,
        };
        let min_x = tb.left + pad;
        let max_x = right_bound.max(min_x);
        let x = match self.taskbar_offset_x.get() {
            Some(offset) => tb.left.saturating_add(offset).clamp(min_x, max_x),
            None => max_x,
        };
        let y = tb.top + ((tb.bottom - tb.top) - height) / 2;
        Some((x, y, shown))
    }
    fn apply_region_and_pos(&self, frame: &Frame, x: i32, y: i32) -> Result<()> {
        let (wd, hd, sw, sh, radius) = frame.layout_dip();
        let (width, height) = (self.px(wd), self.px(hd));
        let (left, top) = if frame.placement == DisplayPlacement::Taskbar {
            self.sync_taskbar_owner(taskbar_geometry().map(|g| g.0));
            if let Some((tx, ty, _)) = self.compute_taskbar_pos(width, height) {
                (tx, ty)
            } else {
                let area = work_area(RECT {
                    left: x,
                    top: y,
                    right: x.saturating_add(width),
                    bottom: y.saturating_add(height),
                })?;
                clamp_to_visible_monitors(x, y, width, height, &[area])
            }
        } else {
            let area = work_area(RECT {
                left: x,
                top: y,
                right: x.saturating_add(width),
                bottom: y.saturating_add(height),
            })?;
            clamp_to_visible_monitors(x, y, width, height, &[area])
        };
        let insert_after = match frame.placement {
            DisplayPlacement::Taskbar => HWND_TOPMOST,
            DisplayPlacement::DesktopWidget => HWND_BOTTOM,
            DisplayPlacement::Floating => {
                if self.topmost.get() {
                    HWND_TOPMOST
                } else {
                    HWND_NOTOPMOST
                }
            }
        };
        unsafe {
            SetWindowPos(
                self.hwnd,
                insert_after,
                left,
                top,
                width,
                height,
                SWP_NOACTIVATE,
            )?;
            let dx = self.px((wd - sw) / 2);
            let dy = self.px((hd - sh) / 2);
            let diameter = self.px((radius * 2.).round() as i32);
            let region =
                CreateRoundRectRgn(dx, dy, width - dx + 1, height - dy + 1, diameter, diameter);
            if region.is_invalid() {
                return Err(Error::from_win32());
            }
            if SetWindowRgn(self.hwnd, region, true) == 0 {
                let err = Error::from_win32();
                let _ = DeleteObject(region);
                return Err(err);
            }
        }
        Ok(())
    }
    fn apply_exstyle(&self) -> Result<()> {
        let placement = self.model.borrow().placement;
        let transparent = self.click_through.get() && placement == DisplayPlacement::Floating;
        let no_activate = placement != DisplayPlacement::Floating;
        let taskbar = if placement == DisplayPlacement::Taskbar {
            taskbar_geometry().map(|g| g.0)
        } else {
            None
        };
        self.sync_taskbar_owner(taskbar);
        unsafe {
            let hwnd = self.hwnd;
            let mut style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            let flag_trans = WS_EX_TRANSPARENT.0 as isize;
            let flag_noact = WS_EX_NOACTIVATE.0 as isize;
            if transparent {
                style |= flag_trans;
            } else {
                style &= !flag_trans;
            }
            if no_activate {
                style |= flag_noact;
            } else {
                style &= !flag_noact;
            }
            SetLastError(ERROR_SUCCESS);
            if SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style) == 0 && GetLastError() != ERROR_SUCCESS {
                return Err(Error::from_win32());
            }
            let insert_after = match placement {
                DisplayPlacement::Taskbar => HWND_TOPMOST,
                DisplayPlacement::DesktopWidget => HWND_BOTTOM,
                DisplayPlacement::Floating => {
                    if self.topmost.get() {
                        HWND_TOPMOST
                    } else {
                        HWND_NOTOPMOST
                    }
                }
            };
            SetWindowPos(
                hwnd,
                insert_after,
                0,
                0,
                0,
                0,
                SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE | SWP_FRAMECHANGED,
            )
        }
    }
    fn guard_taskbar(&self) {
        if !self.visible.get() || self.model.borrow().placement != DisplayPlacement::Taskbar {
            return;
        }
        let (wd, hd) = self.model.borrow().frame(now_ms()).0.size_dip();
        let (width, height) = (self.px(wd), self.px(hd));
        let should_hide = fullscreen_active();
        let geom = taskbar_geometry();
        self.sync_taskbar_owner(geom.as_ref().map(|g| g.0));
        let visible_bar = geom.as_ref().is_some_and(|g| g.3);
        if should_hide || !visible_bar {
            if !self.fullscreen_hidden.replace(true) {
                unsafe {
                    let _ = ShowWindow(self.hwnd, SW_HIDE);
                }
            }
            return;
        }
        if let Some((x, y, _)) = self.compute_taskbar_pos(width, height) {
            let need_move = window_rect(self.hwnd)
                .map(|r| r.left != x || r.top != y || (r.right - r.left) != width || (r.bottom - r.top) != height)
                .unwrap_or(true);
            let need_raise = geom
                .as_ref()
                .is_some_and(|(tb, _, _, _)| is_window_above(*tb, self.hwnd));
            if self.fullscreen_hidden.replace(false) {
                unsafe {
                    let _ = SetWindowPos(
                        self.hwnd,
                        HWND_TOPMOST,
                        x,
                        y,
                        width,
                        height,
                        SWP_NOACTIVATE | SWP_SHOWWINDOW,
                    );
                }
            } else if need_move {
                unsafe {
                    let _ = SetWindowPos(
                        self.hwnd,
                        HWND_TOPMOST,
                        x,
                        y,
                        width,
                        height,
                        SWP_NOACTIVATE,
                    );
                }
            } else if need_raise {
                unsafe {
                    let _ = SetWindowPos(
                        self.hwnd,
                        HWND_TOPMOST,
                        0,
                        0,
                        0,
                        0,
                        SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                    );
                }
            }
        }
    }
    fn render(&self) -> Result<()> {
        self.cancel_timer();
        if !self.visible.get() {
            return Ok(());
        }
        let sampled_at = now_ms();
        let (frame, wake) = self.model.borrow().frame(sampled_at);
        let (width, height) = frame.size_dip();
        let px_w = self.px(width);
        let px_h = self.px(height);
        let has_pulse = frame.placement != DisplayPlacement::Taskbar
            && frame.mode == OverlayMode::CompactCapsule;
        let surface = (px_w, px_h, self.effective_dpi(), has_pulse);
        let mut graphics = self
            .graphics
            .try_borrow_mut()
            .map_err(|_| Error::from_hresult(ERROR_BUSY.to_hresult()))?;
        if graphics.surface != surface {
            if let Ok(cur) = window_rect(self.hwnd) {
                if cur.right - cur.left != px_w || cur.bottom - cur.top != px_h {
                    self.apply_region_and_pos(&frame, cur.left, cur.top)?;
                }
            }
            graphics.renderer = None;
            graphics.last = None;
            graphics.surface = surface;
        }
        if dirty(graphics.last.as_ref(), &frame) {
            let pulse_pos = has_pulse.then_some((215., 15.));
            for attempt in 0..2 {
                let result = (|| unsafe {
                    if graphics.renderer.is_none() {
                        let device =
                            d3d(D3D_DRIVER_TYPE_HARDWARE).or_else(|_| d3d(D3D_DRIVER_TYPE_WARP))?;
                        graphics.renderer = Some(Renderer::new(
                            &device,
                            self.hwnd,
                            surface.0 as u32,
                            surface.1 as u32,
                            surface.2,
                            pulse_pos,
                        )?);
                    }
                    graphics
                        .renderer
                        .as_ref()
                        .ok_or(Error::from_hresult(E_UNEXPECTED))?
                        .draw(&frame)
                })();
                match result {
                    Ok(()) => {
                        graphics.last = Some(frame);
                        self.count.set(self.count.get().saturating_add(1));
                        break;
                    }
                    Err(err) => {
                        graphics.last = None;
                        graphics.renderer = None;
                        if attempt != 0
                            || !matches!(
                                err.code(),
                                DXGI_ERROR_DEVICE_REMOVED
                                    | DXGI_ERROR_DEVICE_RESET
                                    | D2DERR_RECREATE_TARGET
                            )
                        {
                            return Err(err);
                        }
                    }
                }
            }
        }
        if let Some(delay) = wake {
            let deadline = sampled_at.saturating_add(delay.into());
            let delay = deadline.saturating_sub(now_ms()).clamp(1, i32::MAX.into()) as u32;
            if unsafe { SetTimer(self.hwnd, COUNTDOWN, delay, None) } == 0 {
                return Err(Error::from_win32());
            }
            self.timer.set(true);
        }
        Ok(())
    }
    fn layout(&self, x: i32, y: i32) -> Result<()> {
        let (frame, _) = self.model.borrow().frame(now_ms());
        self.apply_region_and_pos(&frame, x, y)?;
        self.render()
    }
    fn mode(&self, mode: OverlayMode) -> Result<()> {
        self.model.borrow_mut().mode = mode;
        let area = window_rect(self.hwnd)?;
        self.layout(area.left, area.top)
    }
    fn snap(&self) -> Result<()> {
        let placement = self.model.borrow().placement;
        let area = window_rect(self.hwnd)?;
        if placement == DisplayPlacement::Taskbar {
            if let Some((_, tb, tray, _)) = taskbar_geometry() {
                let w = area.right - area.left;
                let h = area.bottom - area.top;
                let gap = self.px(8);
                let pad = self.px(6);
                let right_bound = match tray {
                    Some(tr) if tr.left > tb.left + w + gap => tr.left - w - gap,
                    _ => tb.right - w - gap,
                };
                let min_x = tb.left + pad;
                let max_x = right_bound.max(min_x);
                let x = area.left.clamp(min_x, max_x);
                let y = tb.top + ((tb.bottom - tb.top) - h) / 2;
                self.taskbar_offset_x.set(if (max_x - x).abs() <= self.px(12) {
                    None
                } else {
                    Some(x.saturating_sub(tb.left))
                });
                unsafe {
                    SetWindowPos(
                        self.hwnd,
                        HWND_TOPMOST,
                        x,
                        y,
                        0,
                        0,
                        SWP_NOSIZE | SWP_NOACTIVATE,
                    )?;
                }
            }
            return Ok(());
        }
        let work = work_area(area)?;
        let snap = snap_to_work_area(area, work, self.px(10));
        let (left, top) = clamp_to_visible_monitors(
            snap.left,
            snap.top,
            area.right - area.left,
            area.bottom - area.top,
            &[work],
        );
        unsafe {
            SetWindowPos(
                self.hwnd,
                None,
                left,
                top,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            )?;
        }
        save_position(left, top)
    }
    fn drag_move(&self) -> Result<()> {
        if let Some((start, area)) = self.drag.get() {
            let mut point = POINT::default();
            unsafe { GetCursorPos(&mut point)? };
            let dx = point.x.saturating_sub(start.x);
            let dy = point.y.saturating_sub(start.y);
            if !self.moved.get()
                && dx.unsigned_abs() < self.px(4) as u32
                && dy.unsigned_abs() < self.px(4) as u32
            {
                return Ok(());
            }
            self.moved.set(true);
            let placement = self.model.borrow().placement;
            let (next_x, next_y) = if placement == DisplayPlacement::Taskbar {
                (area.left.saturating_add(dx), area.top)
            } else {
                (area.left.saturating_add(dx), area.top.saturating_add(dy))
            };
            unsafe {
                SetWindowPos(
                    self.hwnd,
                    None,
                    next_x,
                    next_y,
                    0,
                    0,
                    SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                )?;
            }
        }
        Ok(())
    }
    fn handle(&self, msg: u32, wp: WPARAM, lp: LPARAM) -> Result<Option<LRESULT>> {
        unsafe {
            match msg {
                WM_MOUSEACTIVATE if self.model.borrow().placement != DisplayPlacement::Floating => {
                    return Ok(Some(LRESULT(MA_NOACTIVATE as isize)));
                }
                WM_WINDOWPOSCHANGING
                    if self.model.borrow().placement == DisplayPlacement::DesktopWidget =>
                {
                    if lp.0 != 0 {
                        let info = &mut *(lp.0 as *mut WINDOWPOS);
                        info.hwndInsertAfter = HWND_BOTTOM;
                        info.flags &= !SWP_NOZORDER;
                    }
                    return Ok(Some(LRESULT(0)));
                }
                WM_DPICHANGED => {
                    self.dpi.set((wp.0 as u16 as u32).max(1));
                    if lp.0 != 0 {
                        let suggested = *(lp.0 as *const RECT);
                        self.layout(suggested.left, suggested.top)?;
                    }
                }
                WM_DISPLAYCHANGE => {
                    let area = window_rect(self.hwnd)?;
                    self.layout(area.left, area.top)?;
                }
                WM_TIMER if wp.0 == COUNTDOWN => {
                    self.render()?;
                }
                WM_TIMER if wp.0 == TASKBAR_GUARD => {
                    self.guard_taskbar();
                }
                WM_LBUTTONDOWN => {
                    let mut point = POINT::default();
                    GetCursorPos(&mut point)?;
                    self.drag.set(Some((point, window_rect(self.hwnd)?)));
                    self.moved.set(false);
                    SetCapture(self.hwnd);
                }
                WM_MOUSEMOVE if self.drag.get().is_some() => self.drag_move()?,
                WM_LBUTTONUP if self.drag.take().is_some() => {
                    ReleaseCapture()?;
                    if self.moved.get() {
                        self.snap()?;
                    } else {
                        let (placement, mode, footer_top) = {
                            let model = self.model.borrow();
                            let (frame, _) = model.frame(now_ms());
                            (model.placement, model.mode, frame.footer_top_dip())
                        };
                        if placement == DisplayPlacement::Taskbar {
                            self.post_action(OVERLAY_ACTION_OPEN_DETAIL);
                        } else if mode == OverlayMode::ExpandedCard {
                            let click_x = (lp.0 as i16) as i32;
                            let click_y = ((lp.0 >> 16) as i16) as i32;
                            if click_y >= self.px(footer_top) {
                                if click_x < self.px(166) {
                                    self.post_action(OVERLAY_ACTION_OPEN_DETAIL);
                                } else {
                                    self.mode(OverlayMode::CompactCapsule)?;
                                    self.post_action(OVERLAY_ACTION_MODE_TOGGLED);
                                }
                            }
                        }
                    }
                }
                WM_CAPTURECHANGED => {
                    if self.drag.take().is_some() && self.moved.get() {
                        self.snap()?;
                    }
                }
                WM_LBUTTONDBLCLK => {
                    let placement = self.model.borrow().placement;
                    if placement == DisplayPlacement::Taskbar {
                        self.post_action(OVERLAY_ACTION_OPEN_DETAIL);
                    } else {
                        let mode = self.model.borrow().mode.toggled();
                        self.mode(mode)?;
                        self.post_action(OVERLAY_ACTION_MODE_TOGGLED);
                    }
                }
                WM_RBUTTONUP => {
                    self.post_action(OVERLAY_ACTION_CONTEXT_MENU);
                }
                WM_NCHITTEST
                    if self.click_through.get()
                        && self.model.borrow().placement == DisplayPlacement::Floating =>
                {
                    return Ok(Some(LRESULT(HTTRANSPARENT as isize)));
                }
                WM_CLOSE => {
                    self.visible.set(false);
                    self.cancel_timer();
                    self.update_taskbar_timer();
                    let _ = ShowWindow(self.hwnd, SW_HIDE);
                }
                _ if self.taskbar_created_msg.get() != 0
                    && msg == self.taskbar_created_msg.get() =>
                {
                    self.owner_taskbar.set(0);
                    self.guard_taskbar();
                }
                _ => return Ok(None),
            }
            Ok(Some(LRESULT(0)))
        }
    }
}
unsafe extern "system" fn foreground_hook_proc(
    _hook: HWINEVENTHOOK,
    _event: u32,
    _hwnd: HWND,
    _id_object: i32,
    _id_child: i32,
    _event_thread: u32,
    _event_time: u32,
) {
    let states: Vec<Rc<Shared>> = WINDOWS.with(|map| {
        map.borrow()
            .values()
            .filter_map(Weak::upgrade)
            .collect()
    });
    for state in states {
        state.guard_taskbar();
    }
}
unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_NCDESTROY {
        WINDOWS.with(|map| map.borrow_mut().remove(&(hwnd.0 as usize)));
    }
    let state = WINDOWS.with(|map| map.borrow().get(&(hwnd.0 as usize)).and_then(Weak::upgrade));
    if let Some(state) = state {
        match state.handle(msg, wp, lp) {
            Ok(Some(value)) => return value,
            Err(err) => {
                *state.error.borrow_mut() = Some(err);
                return LRESULT(0);
            }
            _ => {}
        }
    }
    DefWindowProcW(hwnd, msg, wp, lp)
}
pub struct OverlayWindow {
    state: Rc<Shared>,
}
impl OverlayWindow {
    pub fn create(mode: OverlayMode, position: Option<(i32, i32)>) -> Result<Self> {
        unsafe {
            let instance = GetModuleHandleW(None)?;
            let class = WNDCLASSW {
                style: CS_DBLCLKS,
                lpfnWndProc: Some(wndproc),
                hInstance: instance.into(),
                hCursor: LoadCursorW(None, IDC_ARROW)?,
                lpszClassName: CLASS,
                ..Default::default()
            };
            if RegisterClassW(&class) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
                return Err(Error::from_win32());
            }
            let (x, y) = position.or_else(saved_position).unwrap_or((48, 48));
            let (width, height) = mode.size_dip();
            let hwnd = CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOPMOST,
                CLASS,
                w!("TokenPulse"),
                WS_POPUP,
                x,
                y,
                width,
                height,
                None,
                None,
                instance,
                None,
            )?;
            let state = Rc::new(Shared {
                hwnd,
                model: RefCell::new(Model {
                    mode,
                    ..Default::default()
                }),
                graphics: RefCell::default(),
                visible: Cell::new(false),
                fullscreen_hidden: Cell::new(false),
                topmost: Cell::new(true),
                dpi: Cell::new(GetDpiForWindow(hwnd).max(1)),
                scale: Cell::new(100),
                count: Cell::new(0),
                drag: Cell::new(None),
                moved: Cell::new(false),
                timer: Cell::new(false),
                taskbar_timer: Cell::new(false),
                taskbar_offset_x: Cell::new(None),
                owner_taskbar: Cell::new(0),
                win_event_hook: Cell::new(0),
                taskbar_created_msg: Cell::new(RegisterWindowMessageW(w!("TaskbarCreated"))),
                click_through: Cell::new(false),
                action_host: Cell::new(None),
                error: RefCell::default(),
            });
            WINDOWS.with(|map| {
                map.borrow_mut()
                    .insert(hwnd.0 as usize, Rc::downgrade(&state))
            });
            let window = Self { state };
            window.state.layout(x, y)?;
            Ok(window)
        }
    }
    pub fn hwnd(&self) -> HWND {
        self.state.hwnd
    }
    pub fn set_action_host(&self, host: HWND, message: u32) {
        self.state.action_host.set(Some((host, message)));
    }
    pub fn position(&self) -> Result<(i32, i32)> {
        let area = window_rect(self.hwnd())?;
        Ok((area.left, area.top))
    }
    pub fn present_commit_count(&self) -> u64 {
        self.state.count.get()
    }
    pub fn is_visible(&self) -> bool {
        self.state.visible.get()
    }
    pub fn take_event_error(&self) -> Option<Error> {
        self.state.error.borrow_mut().take()
    }
    pub fn show(&self) -> Result<()> {
        self.state.visible.set(true);
        self.state.fullscreen_hidden.set(false);
        if let Err(err) = self.state.render() {
            self.hide();
            return Err(err);
        }
        let placement = self.state.model.borrow().placement;
        let hide_for_fs = placement == DisplayPlacement::Taskbar && fullscreen_active();
        if hide_for_fs {
            self.state.fullscreen_hidden.set(true);
        } else {
            unsafe {
                let _ = ShowWindow(self.hwnd(), SW_SHOWNOACTIVATE);
            }
            if placement == DisplayPlacement::DesktopWidget {
                unsafe {
                    let _ = SetWindowPos(
                        self.hwnd(),
                        HWND_BOTTOM,
                        0,
                        0,
                        0,
                        0,
                        SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
                    );
                }
            }
        }
        self.state.update_taskbar_timer();
        Ok(())
    }
    pub fn hide(&self) {
        self.state.visible.set(false);
        self.state.fullscreen_hidden.set(false);
        self.state.cancel_timer();
        self.state.update_taskbar_timer();
        unsafe {
            let _ = ShowWindow(self.hwnd(), SW_HIDE);
        }
    }
    pub fn update_snapshot(&self, snapshot: OverlaySnapshotDto) -> Result<()> {
        self.update_snapshot_with_offline_tokens(snapshot, None)
    }
    pub fn update_snapshot_with_offline_tokens(
        &self,
        snapshot: OverlaySnapshotDto,
        ag_today: Option<u64>,
    ) -> Result<()> {
        {
            let mut model = self.state.model.borrow_mut();
            model.snapshot = Some(snapshot);
            model.ag = ag_today;
        }
        self.state.render()
    }
    pub fn set_placement(&self, placement: DisplayPlacement) -> Result<()> {
        let changed = {
            let mut model = self.state.model.borrow_mut();
            let prev = model.placement;
            model.placement = placement;
            prev != placement
        };
        self.state.apply_exstyle()?;
        self.state.update_taskbar_timer();
        if changed {
            let (left, top) = if placement == DisplayPlacement::Taskbar {
                (0, 0)
            } else {
                saved_position().unwrap_or_else(|| self.position().unwrap_or((48, 48)))
            };
            self.state.layout(left, top)?;
        } else {
            self.state.render()?;
        }
        Ok(())
    }
    pub fn set_theme(&self, theme: Theme) -> Result<()> {
        self.state.model.borrow_mut().theme = theme;
        self.state.render()
    }
    pub fn set_language(&self, language: Language) -> Result<()> {
        self.state.model.borrow_mut().lang = language;
        self.state.render()
    }
    pub fn set_mode(&self, mode: OverlayMode) -> Result<()> {
        self.state.mode(mode)
    }
    pub fn set_scale(&self, percent: u32) -> Result<()> {
        self.state.scale.set(percent.clamp(50, 200));
        let (left, top) = self.position()?;
        self.state.layout(left, top)
    }
    pub fn set_opacity(&self, percent: u32) -> Result<()> {
        self.state.model.borrow_mut().opacity = percent.clamp(30, 100);
        self.state.render()
    }
    pub fn set_topmost(&self, enabled: bool) -> Result<()> {
        self.state.topmost.set(enabled);
        if self.state.model.borrow().placement != DisplayPlacement::Floating {
            return Ok(());
        }
        unsafe {
            SetWindowPos(
                self.hwnd(),
                if enabled {
                    HWND_TOPMOST
                } else {
                    HWND_NOTOPMOST
                },
                0,
                0,
                0,
                0,
                SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
            )
        }
    }
    pub fn set_click_through(&self, enabled: bool) -> Result<()> {
        self.state.click_through.set(enabled);
        self.state.apply_exstyle()
    }
    pub fn destroy(self) {
        drop(self)
    }
}
impl Drop for OverlayWindow {
    fn drop(&mut self) {
        self.state.cancel_timer();
        if self.state.taskbar_timer.replace(false) {
            unsafe {
                let _ = KillTimer(self.hwnd(), TASKBAR_GUARD);
            }
        }
        let hook = self.state.win_event_hook.replace(0);
        if hook != 0 {
            unsafe {
                let _ = UnhookWinEvent(HWINEVENTHOOK(hook as _));
            }
        }
        if self.state.owner_taskbar.replace(0) != 0 {
            unsafe {
                SetWindowLongPtrW(self.hwnd(), GWLP_HWNDPARENT, 0);
            }
        }
        self.state.graphics.borrow_mut().renderer = None;
        let hwnd = self.hwnd();
        if WINDOWS
            .with(|map| map.borrow_mut().remove(&(hwnd.0 as usize)))
            .is_some()
        {
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokenpulse_core_domain::dto::{PrivacyBadge, QuotaBarDto};
    fn model() -> Model {
        Model {
            placement: DisplayPlacement::Floating,
            mode: OverlayMode::ExpandedCard,
            snapshot: Some(OverlaySnapshotDto {
                protocol_version: 1,
                codex: vec![QuotaBarDto {
                    target_label: "local".into(),
                    period_kind: QuotaPeriodKind::FiveHour,
                    state: QuotaWindowState::Active {
                        used_pct: 25.,
                        remaining_pct: 75.,
                        resets_in_ms: Some(120_000),
                    },
                    observed_at_ms: None,
                    account_confirmed: true,
                }],
                codex_plan_type: None,
                antigravity: vec![],
                today_tokens: Some(0),
                today_coverage: Coverage::Complete,
                cache_hit_ratio: None,
                latest_turn_cache_hit_ratio: Some(0.35),
                privacy_badge: PrivacyBadge::LocalOnly,
                updated_at_ms: 100,
            }),
            ..Default::default()
        }
    }
    #[test]
    fn minute_boundary_and_expiry() {
        let model = model();
        let (first, delay) = model.frame(100);
        assert_eq!(delay, Some(60_000));
        assert!(first.bars[0].caption(Language::EnUs).starts_with("75% · 2 min"));
        assert!(!dirty(Some(&first), &model.frame(60_099).0));
        assert!(dirty(Some(&first), &model.frame(60_100).0));
        let (expired, delay) = model.frame(120_100);
        assert_eq!(expired.bars[0], Quota::Expired);
        assert_eq!(delay, None);
    }
    #[test]
    fn pro_plan_hides_five_hour_and_formats_days_hours() {
        let mut model = model();
        let snap = model.snapshot.as_mut().unwrap();
        snap.codex_plan_type = Some("prolite".into());
        snap.codex = vec![QuotaBarDto {
            target_label: "Primary".into(),
            period_kind: QuotaPeriodKind::Weekly,
            state: QuotaWindowState::Active {
                used_pct: 8.,
                remaining_pct: 92.,
                resets_in_ms: Some(6 * DAY_MS + 11 * HOUR_MS),
            },
            observed_at_ms: None,
            account_confirmed: false,
        }];
        let (frame, delay) = model.frame(100);
        assert!(frame.hide_five_hour);
        assert_eq!(frame.bars[0], Quota::Missing);
        assert_eq!(frame.size_dip(), (332, 204));
        assert_eq!(delay, Some(HOUR_MS as u32));
        assert!(frame.bars[1].caption(Language::ZhCn).starts_with("92% · 6天11时后"));
        assert!(frame.bars[1].caption(Language::EnUs).starts_with("92% · 6d 11h"));
        assert!(is_pro_plan(Some("pro")));
        assert!(is_pro_plan(Some("prolite")));
        assert!(!is_pro_plan(Some("plus")));
    }
    #[test]
    fn invisible_fields_and_formatting() {
        let mut model = model();
        model.mode = OverlayMode::CompactCapsule;
        let (first, delay) = model.frame(100);
        assert_eq!(delay, Some(120_000));
        model.ag = Some(800);
        assert!(!dirty(Some(&first), &model.frame(60_100).0));
        assert_eq!((first.latest.as_str(), first.today.as_str()), ("35%", "0"));
        assert_eq!(count(None), "—");
        assert_eq!(count(Some(1_250_000)), "1.2M");
        assert_eq!(ratio(Some(f64::NAN)), "—");
        assert_eq!(format_reset_duration(3 * HOUR_MS + 15 * 60_000, Language::ZhCn), "3时15分");
        assert_eq!(format_reset_duration(2 * DAY_MS + 5 * HOUR_MS, Language::EnUs), "2d 5h");
    }
    #[test]
    #[ignore = "requires a composited Windows desktop"]
    fn desktop_dirty_check() -> Result<()> {
        let window = OverlayWindow::create(OverlayMode::CompactCapsule, Some((30, 30)))?;
        window.show()?;
        let count = window.present_commit_count();
        window.set_theme(Theme::Dark)?;
        window.set_opacity(90)?;
        assert_eq!(window.present_commit_count(), count);
        window.set_mode(OverlayMode::ExpandedCard)?;
        assert_eq!(window.present_commit_count(), count + 1);
        window.hide();
        window.set_language(Language::EnUs)?;
        assert_eq!(window.present_commit_count(), count + 1);
        window.show()?;
        assert_eq!(window.present_commit_count(), count + 2);
        window.set_opacity(10)?;
        window.set_opacity(30)?;
        assert_eq!(window.present_commit_count(), count + 3);
        let suggested = window_rect(window.hwnd())?;
        let changed_dpi = if window.state.dpi.get() == 144 {
            192
        } else {
            144
        };
        unsafe {
            SendMessageW(
                window.hwnd(),
                WM_DPICHANGED,
                WPARAM(changed_dpi | (changed_dpi << 16)),
                LPARAM((&suggested as *const RECT) as isize),
            );
        }
        assert!(window.take_event_error().is_none());
        assert_eq!(window.present_commit_count(), count + 4);
        Ok(())
    }
    #[test]
    #[ignore = "requires a composited Windows desktop"]
    fn desktop_expiry_then_idle() -> Result<()> {
        use std::time::{Duration, Instant};
        let window = OverlayWindow::create(OverlayMode::CompactCapsule, Some((50, 50)))?;
        window.show()?;
        let mut dto = model().snapshot.unwrap();
        dto.updated_at_ms = now_ms();
        if let QuotaWindowState::Active {
            ref mut resets_in_ms,
            ..
        } = dto.codex[0].state
        {
            *resets_in_ms = Some(500);
        }
        window.update_snapshot(dto.clone())?;
        let baseline = window.present_commit_count();
        window.update_snapshot(dto)?;
        assert_eq!(window.present_commit_count(), baseline);
        window.set_click_through(true)?;
        window.set_click_through(false)?;
        let pump = |duration| {
            let start = Instant::now();
            while start.elapsed() < duration {
                unsafe {
                    let mut msg = MSG::default();
                    while PeekMessageW(&mut msg, window.hwnd(), 0, 0, PM_REMOVE).as_bool() {
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        };
        pump(Duration::from_millis(750));
        assert_eq!(window.present_commit_count(), baseline + 1);
        pump(Duration::from_millis(1100));
        assert_eq!(window.present_commit_count(), baseline + 1);
        assert!(window.take_event_error().is_none());
        Ok(())
    }
}

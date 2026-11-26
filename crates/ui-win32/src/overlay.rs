use crate::{
    clamp_to_visible_monitors, localized, snap_to_work_area, Language, OverlayGeometry,
    OverlayMode, OverlaySnapshotDto, Rgba, Theme, ThemePalette,
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
        System::{LibraryLoader::GetModuleHandleW, Registry::*},
        UI::{HiDpi::*, Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
    },
};

#[derive(Clone, PartialEq, Debug)]
enum Quota {
    Active(f64, Option<i64>),
    Expired,
    Restricted,
    Missing,
}
impl Quota {
    fn percent(&self) -> String {
        match self {
            Self::Active(value, _) => format!("{value:.0}%"),
            _ => "—".into(),
        }
    }
    fn caption(&self, lang: Language) -> String {
        match self {
            Self::Active(_, Some(minutes)) => format!(
                "{}  ·  {} {}",
                self.percent(),
                minutes,
                localized(lang, "分钟", "min")
            ),
            Self::Expired => {
                localized(lang, "已过期 · 待新活动", "Expired · awaiting activity").into()
            }
            Self::Restricted => localized(lang, "套餐限制", "Plan restricted").into(),
            _ => self.percent(),
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
    mode: OverlayMode,
    theme: Theme,
    lang: Language,
    opacity: u32,
    bars: [Quota; 2],
    today: String,
    latest: String,
    total: String,
    ag: String,
}
#[derive(Clone)]
struct Model {
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
        let compact = self.mode == OverlayMode::CompactCapsule;
        let mut wake = None;
        let bars = [QuotaPeriodKind::FiveHour, QuotaPeriodKind::Weekly].map(|kind| {
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
                    let remaining = resets_in_ms
                        .map(|ms| dto.updated_at_ms.saturating_add(ms).saturating_sub(now));
                    if remaining.is_some_and(|ms| ms <= 0) {
                        return Quota::Expired;
                    }
                    if let Some(ms) = remaining {
                        let next = if compact { ms } else { 1 + (ms - 1) % 60_000 };
                        let delay = next.clamp(1, i32::MAX.into()) as u32;
                        wake = Some(wake.map_or(delay, |prev: u32| prev.min(delay)));
                    }
                    Quota::Active(
                        remaining_pct,
                        remaining
                            .filter(|_| !compact)
                            .map(|ms| 1 + (ms - 1) / 60_000),
                    )
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
        (
            Frame {
                mode: self.mode,
                theme: self.theme,
                lang: self.lang,
                opacity: self.opacity,
                bars,
                today,
                latest: ratio(snapshot.and_then(|dto| dto.latest_turn_cache_hit_ratio)),
                total: if compact {
                    String::new()
                } else {
                    ratio(snapshot.and_then(|dto| dto.cache_hit_ratio))
                },
                ag: if compact {
                    String::new()
                } else {
                    count(self.ag)
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
        visual.SetOffsetX2(215. * scale)?;
        visual.SetOffsetY2(15. * scale)?;
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
    formats: Vec<IDWriteTextFormat>,
}
impl Renderer {
    unsafe fn new(
        device: &ID3D11Device,
        hwnd: HWND,
        width: u32,
        height: u32,
        dpi: u32,
        compact: bool,
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
        let mut formats = Vec::new();
        for (family, locale) in [
            (w!("Segoe UI"), w!("en-US")),
            (w!("Microsoft YaHei UI"), w!("zh-CN")),
        ] {
            for size in [11., 13., 16., 20.] {
                let format = write.CreateTextFormat(
                    family,
                    None,
                    DWRITE_FONT_WEIGHT_NORMAL,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    size,
                    locale,
                )?;
                format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
                format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
                formats.push(format);
            }
        }
        let brush = solid_brush(&context.cast()?)?;
        let pulse = if compact {
            Some(Pulse::new(
                &composition,
                &visual,
                &context.GetDevice()?,
                dpi,
            )?)
        } else {
            None
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
            formats,
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
    formats: &'a [IDWriteTextFormat],
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
        self.ink(color);
        self.rt.DrawText(
            &text.encode_utf16().collect::<Vec<_>>(),
            &self.formats[size],
            &area,
            self.brush,
            D2D1_DRAW_TEXT_OPTIONS_CLIP,
            DWRITE_MEASURING_MODE_NATURAL,
        );
    }
    unsafe fn ring(&self, radius: f32, quota: &Quota, ink: Rgba, p: &ThemePalette) {
        let ellipse = D2D1_ELLIPSE {
            point: D2D_POINT_2F { x: 29., y: 23. },
            radiusX: radius,
            radiusY: radius,
        };
        self.ink(if *quota == Quota::Expired {
            p.warn
        } else {
            p.track
        });
        self.rt.DrawEllipse(&ellipse, self.brush, 3., None);
        if let Quota::Active(percent, _) = quota {
            self.ink(ink);
            let angle = (*percent as f32 / 100.) * std::f32::consts::TAU;
            let steps = (angle * radius).ceil() as u32;
            let point = |index: u32| {
                let phase = angle * index as f32 / steps as f32;
                D2D_POINT_2F {
                    x: 29. + phase.sin() * radius,
                    y: 23. - phase.cos() * radius,
                }
            };
            for index in 0..steps {
                self.rt
                    .DrawLine(point(index), point(index + 1), self.brush, 3., None);
            }
        }
    }
}
impl Renderer {
    unsafe fn draw(&self, frame: &Frame) -> Result<()> {
        let rt: ID2D1RenderTarget = self.context.cast()?;
        let palette = ThemePalette::for_theme(frame.theme);
        let chinese = frame.lang == Language::ZhCn;
        let local = |zh, en| localized(frame.lang, zh, en);
        let canvas = Canvas {
            rt,
            brush: &self.brush,
            formats: &self.formats[if chinese { 4 } else { 0 }..],
        };
        let panel = &canvas;
        let opacity = frame.opacity as f32 / 100.;
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
        let compact = frame.mode == OverlayMode::CompactCapsule;
        let (width, height) = frame.mode.size_dip();
        let (sw, sh) = frame.mode.surface_dip();
        let left = (width - sw) as f32 / 2.;
        let top = (height - sh) as f32 / 2.;
        let area = rect(
            left + 0.5,
            top + 0.5,
            left + sw as f32 - 0.5,
            top + sh as f32 - 0.5,
        );
        let radius = if compact { 22. } else { 14. };
        panel.round(area, radius, palette.bg_overlay);
        panel.ink(palette.border_strong);
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
        let (status, color) = if frame.bars.contains(&Quota::Expired) {
            (local("待新活动", "Expired"), palette.warn)
        } else if frame
            .bars
            .iter()
            .any(|q| matches!(q,Quota::Active(n,_) if *n<=10.))
        {
            (local("低额度", "Low quota"), palette.danger)
        } else if frame.bars.iter().any(|q| matches!(q, Quota::Active(..))) {
            (local("正常", "Active"), palette.ok)
        } else {
            (local("暂无额度", "No quota"), palette.text_muted)
        };
        if compact {
            panel.ring(11.5, &frame.bars[0], palette.accent_codex, &palette);
            panel.ring(6.5, &frame.bars[1], palette.accent_ag, &palette);
            panel.text(
                &format!(
                    "{} {} / {}",
                    local("剩余", "Left"),
                    frame.bars[0].percent(),
                    frame.bars[1].percent()
                ),
                rect(51., 2., 215., 24.),
                2,
                palette.text_primary,
            );
            panel.text(
                &format!(
                    "{} {} · {} {}",
                    local("今日", "Today"),
                    frame.today,
                    local("本轮缓存", "Cache"),
                    frame.latest
                ),
                rect(51., 23., 216., 43.),
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
                rect(16., 7., 205., 36.),
                3,
                palette.text_primary,
            );
            panel.round(rect(236., 12., 315., 33.), 10., palette.track);
            panel.text(status, rect(246., 12., 314., 33.), 0, color);
            for (index, quota) in frame.bars.iter().enumerate() {
                let y = 41. + index as f32 * 36.;
                panel.text(
                    if index == 0 { "Codex 5h" } else { "Codex 7d" },
                    rect(16., y, 104., y + 18.),
                    1,
                    palette.text_primary,
                );
                panel.text(
                    &quota.caption(frame.lang),
                    rect(112., y, 316., y + 18.),
                    0,
                    if *quota == Quota::Expired {
                        palette.warn
                    } else {
                        palette.text_secondary
                    },
                );
                panel.round(rect(16., y + 21., 316., y + 27.), 3., palette.track);
                if let Quota::Active(n, _) = quota {
                    if *n > 0. {
                        panel.round(
                            rect(16., y + 21., 16. + 3. * (*n as f32), y + 27.),
                            3.,
                            palette.accent_codex,
                        );
                    }
                }
            }
            panel.text(
                local("今日总 Token", "Today's tokens"),
                rect(16., 111., 145., 139.),
                1,
                palette.text_secondary,
            );
            panel.text(
                &frame.today,
                rect(156., 111., 315., 139.),
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
                rect(16., 142., 316., 164.),
                1,
                palette.text_secondary,
            );
            panel.text(
                &format!(
                    "Antigravity · {}   {}",
                    local("离线 Token", "Offline tokens"),
                    frame.ag
                ),
                rect(16., 173., 316., 196.),
                1,
                palette.accent_ag,
            );
            panel.round(rect(16., 204., 316., 205.), 0., palette.border_subtle);
            panel.text(
                local("零外网 · 0 帧空闲", "No network · 0 idle frames"),
                rect(16., 208., 192., 232.),
                0,
                palette.text_muted,
            );
            panel.text(
                local("点击切换胶囊", "Click for capsule"),
                rect(198., 208., 316., 232.),
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
    surface: (i32, i32, u32),
    last: Option<Frame>,
}
struct Shared {
    hwnd: HWND,
    model: RefCell<Model>,
    graphics: RefCell<Graphics>,
    visible: Cell<bool>,
    dpi: Cell<u32>,
    scale: Cell<u32>,
    count: Cell<u64>,
    drag: Cell<Option<(POINT, RECT)>>,
    moved: Cell<bool>,
    timer: Cell<bool>,
    click_through: Cell<bool>,
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
    fn cancel_timer(&self) {
        if self.timer.replace(false) {
            unsafe {
                let _ = KillTimer(self.hwnd, COUNTDOWN);
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
        let (width, height) = frame.mode.size_dip();
        let surface = (self.px(width), self.px(height), self.effective_dpi());
        let mut graphics = self
            .graphics
            .try_borrow_mut()
            .map_err(|_| Error::from_hresult(ERROR_BUSY.to_hresult()))?;
        if graphics.surface != surface {
            graphics.renderer = None;
            graphics.last = None;
            graphics.surface = surface;
        }
        if dirty(graphics.last.as_ref(), &frame) {
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
                            frame.mode == OverlayMode::CompactCapsule,
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
        let mode = self.model.borrow().mode;
        let (wd, hd) = mode.size_dip();
        let (width, height) = (self.px(wd), self.px(hd));
        let area = work_area(RECT {
            left: x,
            top: y,
            right: x.saturating_add(width),
            bottom: y.saturating_add(height),
        })?;
        let (left, top) = clamp_to_visible_monitors(x, y, width, height, &[area]);
        unsafe {
            SetWindowPos(
                self.hwnd,
                None,
                left,
                top,
                width,
                height,
                SWP_NOZORDER | SWP_NOACTIVATE,
            )?;
            let (sw, sh) = mode.surface_dip();
            let dx = self.px((wd - sw) / 2);
            let dy = self.px((hd - sh) / 2);
            let diameter = self.px(if mode == OverlayMode::CompactCapsule {
                44
            } else {
                28
            });
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
        self.render()
    }
    fn mode(&self, mode: OverlayMode) -> Result<()> {
        self.model.borrow_mut().mode = mode;
        let area = window_rect(self.hwnd)?;
        self.layout(area.left, area.top)
    }
    fn snap(&self) -> Result<()> {
        let area = window_rect(self.hwnd)?;
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
            unsafe {
                SetWindowPos(
                    self.hwnd,
                    None,
                    area.left.saturating_add(dx),
                    area.top.saturating_add(dy),
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
                        self.snap()?
                    } else if self.model.borrow().mode == OverlayMode::ExpandedCard
                        && ((lp.0 >> 16) as i16 as i32) >= self.px(204)
                        && (lp.0 as i16 as i32) >= self.px(190)
                    {
                        self.mode(OverlayMode::CompactCapsule)?;
                    }
                }
                WM_CAPTURECHANGED => {
                    if self.drag.take().is_some() && self.moved.get() {
                        self.snap()?;
                    }
                }
                WM_LBUTTONDBLCLK => {
                    let mode = self.model.borrow().mode.toggled();
                    self.mode(mode)?;
                }
                WM_NCHITTEST if self.click_through.get() => {
                    return Ok(Some(LRESULT(HTTRANSPARENT as isize)))
                }
                WM_CLOSE => {
                    self.visible.set(false);
                    self.cancel_timer();
                    let _ = ShowWindow(self.hwnd, SW_HIDE);
                }
                _ => return Ok(None),
            }
            Ok(Some(LRESULT(0)))
        }
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
                dpi: Cell::new(GetDpiForWindow(hwnd).max(1)),
                scale: Cell::new(100),
                count: Cell::new(0),
                drag: Cell::new(None),
                moved: Cell::new(false),
                timer: Cell::new(false),
                click_through: Cell::new(false),
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
        if let Err(err) = self.state.render() {
            self.hide();
            return Err(err);
        }
        unsafe {
            let _ = ShowWindow(self.hwnd(), SW_SHOWNOACTIVATE);
        }
        Ok(())
    }
    pub fn hide(&self) {
        self.state.visible.set(false);
        self.state.cancel_timer();
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
        unsafe {
            let hwnd = self.hwnd();
            let old = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            let flag = WS_EX_TRANSPARENT.0 as isize;
            SetLastError(ERROR_SUCCESS);
            if SetWindowLongPtrW(
                hwnd,
                GWL_EXSTYLE,
                if enabled { old | flag } else { old & !flag },
            ) == 0
                && GetLastError() != ERROR_SUCCESS
            {
                return Err(Error::from_win32());
            }
            self.state.click_through.set(enabled);
            SetWindowPos(
                hwnd,
                None,
                0,
                0,
                0,
                0,
                SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_FRAMECHANGED,
            )
        }
    }
    pub fn destroy(self) {
        drop(self)
    }
}
impl Drop for OverlayWindow {
    fn drop(&mut self) {
        self.state.cancel_timer();
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
        assert_eq!(first.bars[0].caption(Language::EnUs), "75%  ·  2 min");
        assert!(!dirty(Some(&first), &model.frame(60_099).0));
        assert!(dirty(Some(&first), &model.frame(60_100).0));
        let (expired, delay) = model.frame(120_100);
        assert_eq!(expired.bars[0], Quota::Expired);
        assert_eq!(delay, None);
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

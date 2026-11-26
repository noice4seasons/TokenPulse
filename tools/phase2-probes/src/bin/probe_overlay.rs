use serde_json::{json, Value};
use std::{mem::size_of, time::Instant};
use windows::{
    core::{w, Error, Interface, Result},
    Win32::{
        Foundation::*,
        Graphics::{
            Direct2D::{Common::*, *},
            Direct3D::*,
            Direct3D11::*,
            DirectComposition::*,
            DirectWrite::*,
            Dxgi::{Common::*, *},
        },
        System::{ProcessStatus::*, Threading::GetCurrentProcess},
        UI::{HiDpi::*, WindowsAndMessaging::*},
    },
};

struct Window(HWND);
impl Drop for Window {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.0);
        }
    }
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
fn color(r: f32, g: f32, b: f32, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r, g, b, a }
}

struct Renderer {
    context: ID2D1DeviceContext,
    chain: IDXGISwapChain1,
    composition: IDCompositionDesktopDevice,
    _target: IDCompositionTarget,
    formats: [IDWriteTextFormat; 2],
    frames: u32,
}

impl Renderer {
    unsafe fn new(
        device: &ID3D11Device,
        hwnd: HWND,
        width: u32,
        height: u32,
        dpi: u32,
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
        effect.SetOpacity2(0.88)?;
        visual.SetEffect(&effect)?;
        visual.SetContent(&chain)?;
        target.SetRoot(&visual)?;
        let write: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
        let format = |family, size| {
            write.CreateTextFormat(
                family,
                None,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                size,
                w!("zh-CN"),
            )
        };
        Ok(Self {
            context,
            chain,
            composition,
            _target: target,
            formats: [
                format(w!("Segoe UI"), 22.)?,
                format(w!("Microsoft YaHei UI"), 16.)?,
            ],
            frames: 0,
        })
    }

    unsafe fn draw(&mut self, updated: bool) -> Result<()> {
        let ctx = &self.context;
        // Call ID2D1RenderTarget methods explicitly where the device context adds overloads.
        let rt: ID2D1RenderTarget = ctx.cast()?;
        let brush = rt.CreateSolidColorBrush(&color(0.06, 0.09, 0.14, 0.95), None)?;
        rt.BeginDraw();
        rt.Clear(Some(&color(0., 0., 0., 0.)));
        rt.FillRoundedRectangle(
            &D2D1_ROUNDED_RECT {
                rect: rect(0., 0., 320., 160.),
                radiusX: 18.,
                radiusY: 18.,
            },
            &brush,
        );
        brush.SetColor(&color(0.93, 0.96, 1., 1.));
        let label = if updated {
            "剩余额度  72%"
        } else {
            "剩余额度  48%"
        };
        for (index, text, top) in [(0, "TokenPulse", 18.), (1, label, 70.)] {
            rt.DrawText(
                &text.encode_utf16().collect::<Vec<_>>(),
                &self.formats[index],
                &rect(20., top, 302., top + 40.),
                &brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        }
        brush.SetColor(&color(0.2, 0.24, 0.29, 1.));
        rt.FillRectangle(&rect(20., 124., 300., 132.), &brush);
        brush.SetColor(&color(0.25, 0.84, 0.68, 1.));
        rt.FillRectangle(
            &rect(
                20.,
                124.,
                20. + 280. * if updated { 0.72 } else { 0.48 },
                132.,
            ),
            &brush,
        );
        rt.EndDraw(None, None)?;
        self.chain.Present(1, DXGI_PRESENT(0)).ok()?;
        self.composition.Commit()?;
        self.frames += 1;
        Ok(())
    }
}

unsafe fn style(hwnd: HWND, value: isize) -> Result<()> {
    SetLastError(ERROR_SUCCESS);
    if SetWindowLongPtrW(hwnd, GWL_EXSTYLE, value) == 0 && GetLastError() != ERROR_SUCCESS {
        return Err(Error::from_win32());
    }
    if GetWindowLongPtrW(hwnd, GWL_EXSTYLE) != value {
        return Err(Error::from_hresult(E_FAIL));
    }
    Ok(())
}

unsafe fn run(duration: u32) -> Result<Value> {
    let start = Instant::now();
    SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)?;
    let window = Window(CreateWindowExW(
        WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
        w!("STATIC"),
        w!("TokenPulse"),
        WS_POPUP,
        48,
        48,
        320,
        160,
        None,
        None,
        None,
        None,
    )?);
    let hwnd = window.0;
    let dpi = GetDpiForWindow(hwnd);
    if dpi == 0 {
        return Err(Error::from_win32());
    }
    let pixels = |dip: u32| (dip * dpi + 48) / 96;
    let (width, height) = (pixels(320), pixels(160));
    SetWindowPos(
        hwnd,
        HWND_TOPMOST,
        0,
        0,
        width as i32,
        height as i32,
        SWP_NOMOVE | SWP_NOACTIVATE,
    )?;
    let (device, driver) = match d3d(D3D_DRIVER_TYPE_HARDWARE) {
        Ok(d) => (d, "hardware"),
        Err(_) => (d3d(D3D_DRIVER_TYPE_WARP)?, "warp"),
    };
    let mut renderer = Renderer::new(&device, hwnd, width, height, dpi)?;
    let old = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
    style(hwnd, old | WS_EX_TRANSPARENT.0 as isize)?;
    style(hwnd, old)?;
    let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
    renderer.draw(false)?;
    let first_ms = start.elapsed().as_millis();
    if SetTimer(hwnd, 1, duration / 2, None) == 0 || SetTimer(hwnd, 2, duration, None) == 0 {
        return Err(Error::from_win32());
    }
    let mut baseline = renderer.frames;
    let mut before = None;
    let mut msg = MSG::default();
    loop {
        match GetMessageW(&mut msg, None, 0, 0).0 {
            -1 => return Err(Error::from_win32()),
            0 => return Err(Error::from_hresult(E_ABORT)),
            _ => {}
        }
        if msg.hwnd == hwnd && msg.message == WM_CLOSE {
            return Err(Error::from_hresult(E_ABORT));
        }
        if msg.hwnd == hwnd && msg.message == WM_TIMER {
            match msg.wParam.0 {
                1 if before.is_none() => {
                    KillTimer(hwnd, 1)?;
                    before = Some(renderer.frames - baseline);
                    renderer.draw(true)?;
                    baseline = renderer.frames;
                }
                2 => {
                    KillTimer(hwnd, 2)?;
                    break;
                }
                _ => {}
            }
        } else {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    let after = renderer.frames - baseline;
    let mut counters = PROCESS_MEMORY_COUNTERS_EX::default();
    let size = size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
    counters.cb = size;
    GetProcessMemoryInfo(
        GetCurrentProcess(),
        (&mut counters as *mut PROCESS_MEMORY_COUNTERS_EX).cast(),
        size,
    )?;
    Ok(
        json!({"passed":before==Some(0)&&after==0&&renderer.frames==2,"dpi":dpi,"driver_type":driver,
      "duration_ms":duration,"first_frame_ms":first_ms,"present_commit_count":renderer.frames,
      "idle_commits_before_update":before,"idle_commits_after_update":after,
      "idle_commits":before.map(|v|v+after),"opacity_supported":true,"click_through_toggled":true,
      "private_bytes_mb":counters.PrivateUsage as f64/1048576.,"working_set_mb":counters.WorkingSetSize as f64/1048576.}),
    )
}

fn duration() -> Result<u32> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    match args.as_slice() {
        [] => Ok(3000),
        [flag, v] if flag == "--duration-ms" => v
            .parse::<u32>()
            .ok()
            .filter(|v| (20..=0x7fff_ffff).contains(v))
            .ok_or(Error::from_hresult(E_INVALIDARG)),
        _ => Err(Error::from_hresult(E_INVALIDARG)),
    }
}
fn main() {
    let report = duration()
        .and_then(|ms| unsafe { run(ms) })
        .unwrap_or_else(|e| json!({"error":e.to_string(),"hresult":e.code().0}));
    println!("{report}");
    if report["passed"] != true {
        std::process::exit(1);
    }
}

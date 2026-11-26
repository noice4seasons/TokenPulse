use crate::{
    assets::{resolve_asset, STRICT_CSP},
    security::{
        is_allowed_top_level_navigation, validate_incoming_message, HostCommand, MessageRateLimiter,
    },
};
use serde::Serialize;
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    ffi::c_void,
    fmt,
    os::windows::ffi::OsStrExt,
    path::Path,
    ptr,
    rc::Rc,
    time::{Duration, Instant},
};
use tokenpulse_core_domain::{
    config::AppConfig,
    dto::{DashboardSnapshotDto, PROTOCOL_VERSION},
};
use webview2_com::{Microsoft::Web::WebView2::Win32::*, *};
use windows::{
    core::{w, Error, Interface, Result as WinResult, HRESULT, HSTRING, PCWSTR, PWSTR},
    Win32::{
        Foundation::*,
        System::{Com::*, LibraryLoader::GetModuleHandleW, WinRT::EventRegistrationToken},
        UI::WindowsAndMessaging::*,
    },
};

// These two SDK functions live behind extra windows-rs feature groups.
// Keep the manifest's small feature set and use their fixed Win32 ABIs.
#[link(name = "ole32")]
extern "system" {
    fn CreateStreamOnHGlobal(
        memory: HGLOBAL,
        delete_on_release: BOOL,
        stream: *mut *mut c_void,
    ) -> HRESULT;
}
#[link(name = "user32")]
extern "system" {
    fn GetDpiForWindow(hwnd: HWND) -> u32;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetailHostError {
    RuntimeMissing(i32),
    Win32(String),
    AlreadyClosed,
}
impl fmt::Display for DetailHostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RuntimeMissing(hr) => {
                write!(f, "Microsoft WebView2 Runtime is unavailable (0x{hr:08X})")
            }
            Self::Win32(message) => f.write_str(message),
            Self::AlreadyClosed => f.write_str("TokenPulse detail window has closed"),
        }
    }
}
impl std::error::Error for DetailHostError {}
impl From<Error> for DetailHostError {
    fn from(error: Error) -> Self {
        Self::Win32(error.to_string())
    }
}
type HostResult<T> = std::result::Result<T, DetailHostError>;

const CLASS: PCWSTR = w!("TokenPulse.Detail.Web");
const DISPATCH_COMMAND: u32 = WM_APP + 1;
thread_local! {
    static OPEN_GENERATION: Rc<Cell<u64>> = Rc::new(Cell::new(0));
    static IN_USE: Cell<bool> = const { Cell::new(false) };
}

// Keep a rapid reopen from sharing a browser process with an instance still
// closing on this STA (including a nested open during the exit message pump).
struct Lease;
impl Lease {
    fn acquire() -> HostResult<Self> {
        if IN_USE.with(|busy| busy.replace(true)) {
            Err(DetailHostError::Win32(
                "Detail window is already open or closing".into(),
            ))
        } else {
            Ok(Self)
        }
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        IN_USE.with(|busy| busy.set(false));
    }
}

struct ExitWatch {
    events: ICoreWebView2Environment5,
    token: EventRegistrationToken,
    exited: Rc<Cell<bool>>,
}
impl ExitWatch {
    unsafe fn subscribe(env: &ICoreWebView2Environment) -> WinResult<Self> {
        let events: ICoreWebView2Environment5 = env.cast()?;
        let exited = Rc::new(Cell::new(false));
        let signal = exited.clone();
        let mut token = Default::default();
        events.add_BrowserProcessExited(
            &BrowserProcessExitedEventHandler::create(Box::new(move |_, _| {
                signal.set(true);
                Ok(())
            })),
            &mut token,
        )?;
        Ok(Self {
            events,
            token,
            exited,
        })
    }
}
impl Drop for ExitWatch {
    fn drop(&mut self) {
        unsafe {
            let _ = self.events.remove_BrowserProcessExited(self.token);
        }
    }
}

#[derive(Default)]
struct Session {
    controller: Option<ICoreWebView2Controller>,
    view: Option<ICoreWebView2>,
    remove_handlers: Vec<Box<dyn FnOnce() -> WinResult<()>>>,
    exit: Option<ExitWatch>,
    had_controller: bool,
    lease: Option<Lease>,
}

struct State {
    hwnd: Cell<HWND>,
    open_generation: Rc<Cell<u64>>,
    opened: u64,
    session: RefCell<Option<Session>>,
    commands: RefCell<VecDeque<HostCommand>>,
    on_command: Box<dyn Fn(HostCommand)>,
    close_error: RefCell<Option<DetailHostError>>,
}

impl State {
    fn current(&self) -> bool {
        self.open_generation.get() == self.opened
    }

    unsafe fn destroy_hwnd(&self) -> WinResult<()> {
        let hwnd = self.hwnd.get();
        if !hwnd.is_invalid() {
            DestroyWindow(hwnd)?;
        }
        Ok(())
    }

    unsafe fn teardown(&self, mut live: Session) -> HostResult<()> {
        let mut result = Ok(());
        // Always attempt every stage, even when an earlier COM call failed.
        while let Some(remove) = live.remove_handlers.pop() {
            result = result.and(remove());
        }
        if let Some(controller) = live.controller.take() {
            result = result.and(controller.Close());
        }
        drop(live.view.take());
        result = result.and(self.destroy_hwnd());
        if let Some(watch) = live.exit.take() {
            if live.had_controller {
                let deadline = Instant::now() + Duration::from_millis(3000);
                result = result.and(pump_until(|| watch.exited.get(), Some(deadline), || {}));
            }
            drop(watch);
        }
        drop(live.lease.take());
        Ok(result?)
    }

    unsafe fn close(&self) -> HostResult<()> {
        if !self.current() {
            return Err(DetailHostError::AlreadyClosed);
        }
        self.open_generation.set(self.opened + 1);
        self.commands.borrow_mut().clear();
        let live = self.session.borrow_mut().take();
        // During creation, open() owns the pending session and its exit watch.
        match live {
            Some(live) => self.teardown(live),
            None => Ok(self.destroy_hwnd()?),
        }
    }

    unsafe fn resize(&self) -> WinResult<()> {
        let controller = self
            .session
            .borrow()
            .as_ref()
            .and_then(|s| s.controller.clone());
        if let Some(controller) = controller {
            fit(&controller, self.hwnd.get())?;
        }
        Ok(())
    }
}

// Preserve the outer application's WM_QUIT across a nested pump. During
// creation it also cancels the generation while the COM callback drains.
unsafe fn pump_until(
    ready: impl Fn() -> bool,
    deadline: Option<Instant>,
    on_quit: impl Fn(),
) -> WinResult<()> {
    struct Quit(Option<i32>);
    impl Drop for Quit {
        fn drop(&mut self) {
            if let Some(code) = self.0 {
                unsafe {
                    PostQuitMessage(code);
                }
            }
        }
    }
    let mut quit = Quit(None);
    while !ready() {
        let timeout = match deadline {
            Some(end) => {
                let remaining = end.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err(Error::from_hresult(ERROR_TIMEOUT.to_hresult()));
                }
                remaining.as_millis().max(1) as u32
            }
            None => u32::MAX,
        };
        let mut msg = MSG::default();
        if PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
            if msg.message == WM_QUIT {
                quit.0 = Some(msg.wParam.0 as i32);
                on_quit();
            } else {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        } else if !ready()
            && MsgWaitForMultipleObjectsEx(None, timeout, QS_ALLINPUT, MWMO_INPUTAVAILABLE)
                == WAIT_FAILED
        {
            return Err(Error::from_win32());
        }
    }
    Ok(())
}

/// Owns one on-demand detail window. Open it on the application's initialized
/// UI STA, with its dedicated user-data directory. Creation dispatches Windows
/// messages, so WM_CLOSE can cancel either asynchronous creation step.
pub struct DetailWindowHost {
    state: Rc<State>,
}

impl DetailWindowHost {
    pub fn open(
        user_data_folder: &Path,
        on_command: impl Fn(HostCommand) + 'static,
    ) -> HostResult<Self> {
        unsafe {
            runtime()?;
            let mut apartment = APTTYPE::default();
            CoGetApartmentType(&mut apartment, &mut APTTYPEQUALIFIER::default())?;
            if !matches!(apartment, APTTYPE_STA | APTTYPE_MAINSTA) {
                return Err(DetailHostError::Win32(
                    "Detail window requires the UI STA".into(),
                ));
            }
            let folder: Vec<u16> = user_data_folder
                .as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect();
            if !user_data_folder.is_absolute() || folder[..folder.len() - 1].contains(&0) {
                return Err(DetailHostError::Win32(
                    "Expected an absolute WebView2 user-data directory".into(),
                ));
            }
            let mut live = Session {
                lease: Some(Lease::acquire()?),
                ..Default::default()
            };
            let open_generation = OPEN_GENERATION.with(Rc::clone);
            let opened = open_generation.get() + 1;
            open_generation.set(opened);
            let host = Self {
                state: Rc::new(State {
                    hwnd: Cell::new(HWND::default()),
                    open_generation,
                    opened,
                    session: RefCell::new(None),
                    commands: RefCell::new(VecDeque::new()),
                    on_command: Box::new(on_command),
                    close_error: RefCell::new(None),
                }),
            };
            let init = create_window(&host.state)
                .map_err(DetailHostError::from)
                .and_then(|()| host.initialize(&folder, &mut live));
            if let Err(err) = init {
                if host.state.current() {
                    host.state.open_generation.set(opened + 1);
                }
                host.state.teardown(live)?;
                return Err(host.take_close_error().unwrap_or(err));
            }
            host.state.session.replace(Some(live));
            Ok(host)
        }
    }

    unsafe fn initialize(&self, folder: &[u16], live: &mut Session) -> HostResult<()> {
        let state = &self.state;
        if !state.current() {
            return Err(DetailHostError::AlreadyClosed);
        }
        let cancel = || {
            if state.current() {
                let _ = state.close();
            }
        };
        let env_result = Rc::new(RefCell::new(None));
        let slot = env_result.clone();
        let generation = state.open_generation.clone();
        let opened = state.opened;
        CreateCoreWebView2EnvironmentWithOptions(
            PCWSTR::null(),
            PCWSTR(folder.as_ptr()),
            None,
            &CreateCoreWebView2EnvironmentCompletedHandler::create(Box::new(move |hr, env| {
                let result = if generation.get() == opened {
                    hr.and_then(|()| required(env))
                } else {
                    Err(Error::from_hresult(E_ABORT))
                };
                slot.replace(Some(result));
                Ok(())
            })),
        )
        .map_err(runtime_error)?;
        // No controller can be created by a late environment callback alone.
        pump_until(
            || !state.current() || env_result.borrow().is_some(),
            Some(Instant::now() + Duration::from_secs(30)),
            cancel,
        )?;
        if !state.current() {
            return Err(DetailHostError::AlreadyClosed);
        }
        let env = env_result
            .take()
            .ok_or_else(|| Error::from_hresult(E_UNEXPECTED))?
            .map_err(runtime_error)?;
        live.exit = Some(ExitWatch::subscribe(&env)?);
        let controller_result = Rc::new(RefCell::new(None));
        let slot = controller_result.clone();
        let generation = state.open_generation.clone();
        env.CreateCoreWebView2Controller(
            state.hwnd.get(),
            &CreateCoreWebView2ControllerCompletedHandler::create(Box::new(
                move |hr, controller| {
                    let created = controller.is_some();
                    let result = match controller {
                        Some(c) if generation.get() != opened || hr.is_err() => {
                            c.Close().and(hr).map(|()| None)
                        }
                        other => hr.and_then(|()| required(other)).map(Some),
                    };
                    slot.replace(Some((created, result)));
                    Ok(())
                },
            )),
        )?;
        pump_until(|| controller_result.borrow().is_some(), None, cancel)?;
        let (created, result) = controller_result
            .take()
            .ok_or_else(|| Error::from_hresult(E_UNEXPECTED))?;
        live.had_controller = created;
        // WM_CLOSE can also arrive between callback completion and pump return.
        match result {
            Ok(controller) => live.controller = controller,
            Err(_) if !state.current() && !created => return Err(DetailHostError::AlreadyClosed),
            Err(e) => return Err(e.into()),
        }
        if !state.current() {
            return Err(DetailHostError::AlreadyClosed);
        }
        let controller = required(live.controller.clone())?;
        let view = controller.CoreWebView2()?;
        live.view = Some(view.clone());
        install(state, &env, &view, live)?;
        fit(&controller, state.hwnd.get())?;
        controller.SetIsVisible(true)?;
        if !state.current() {
            return Err(DetailHostError::AlreadyClosed);
        }
        view.Navigate(w!("https://app.tokenpulse.local/"))?;
        if !state.current() {
            return Err(DetailHostError::AlreadyClosed);
        }
        Ok(())
    }

    fn post(&self, message: &impl Serialize) -> HostResult<()> {
        let view = self
            .state
            .session
            .borrow()
            .as_ref()
            .and_then(|s| s.view.clone())
            .ok_or(DetailHostError::AlreadyClosed)?;
        let source = unsafe { string_arg(|out| view.Source(out), 256)? };
        if !is_allowed_top_level_navigation(&source) {
            return Err(DetailHostError::Win32(
                "Trusted detail document is not ready".into(),
            ));
        }
        let json =
            serde_json::to_string(message).map_err(|e| DetailHostError::Win32(e.to_string()))?;
        unsafe { Ok(view.PostWebMessageAsJson(&HSTRING::from(json))?) }
    }

    pub fn push_snapshot(
        &self,
        snapshot: &DashboardSnapshotDto,
        config: &AppConfig,
    ) -> HostResult<()> {
        #[derive(Serialize)]
        struct Snapshot<'a> {
            protocol_version: u32,
            #[serde(rename = "type")]
            kind: &'static str,
            snapshot: &'a DashboardSnapshotDto,
            config: &'a AppConfig,
        }
        self.post(&Snapshot {
            protocol_version: PROTOCOL_VERSION,
            kind: "snapshot",
            snapshot,
            config,
        })
    }

    pub fn push_toast(&self, msg: &str) -> HostResult<()> {
        self.post(&serde_json::json!({"protocol_version": PROTOCOL_VERSION, "type": "toast", "message": msg}))
    }

    /// Returns a teardown failure from WM_CLOSE, if there was one.
    pub fn take_close_error(&self) -> Option<DetailHostError> {
        self.state.close_error.take()
    }

    pub fn is_closed(&self) -> bool {
        !self.state.current()
    }

    pub fn close(&self) -> HostResult<()> {
        unsafe { self.state.close() }
    }
}

impl Drop for DetailWindowHost {
    fn drop(&mut self) {
        if self.state.current() {
            let _ = self.close();
        }
    }
}

unsafe fn fit(controller: &ICoreWebView2Controller, hwnd: HWND) -> WinResult<()> {
    let mut rect = RECT::default();
    GetClientRect(hwnd, &mut rect)?;
    controller.SetBounds(rect)
}
fn pixels(dip: i32, dpi: u32) -> i32 {
    ((dip as i64 * i64::from(if dpi == 0 { 96 } else { dpi }) + 48) / 96) as i32
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        let cs = &*(lp.0 as *const CREATESTRUCTW);
        let raw = cs.lpCreateParams as *const State;
        Rc::increment_strong_count(raw);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, raw as isize);
        (*raw).hwnd.set(hwnd);
    }
    let raw = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const State;
    if raw.is_null() {
        return DefWindowProcW(hwnd, msg, wp, lp);
    }
    // Each dispatch owns a reference across reentrant COM/Win32 calls.
    Rc::increment_strong_count(raw);
    let state = Rc::from_raw(raw);
    match msg {
        WM_CLOSE => {
            if state.current() {
                if let Err(error) = state.close() {
                    *state.close_error.borrow_mut() = Some(error);
                }
            }
        }
        WM_NCDESTROY => {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            state.hwnd.set(HWND::default());
            drop(Rc::from_raw(raw));
            if state.current() {
                if let Err(error) = state.close() {
                    *state.close_error.borrow_mut() = Some(error);
                }
            }
            return DefWindowProcW(hwnd, msg, wp, lp);
        }
        WM_GETMINMAXINFO => {
            let info = &mut *(lp.0 as *mut MINMAXINFO);
            let dpi = GetDpiForWindow(hwnd);
            info.ptMinTrackSize = POINT {
                x: pixels(900, dpi),
                y: pixels(600, dpi),
            };
        }
        WM_DPICHANGED => {
            let r = &*(lp.0 as *const RECT);
            let _ = SetWindowPos(
                hwnd,
                None,
                r.left,
                r.top,
                r.right - r.left,
                r.bottom - r.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
        WM_SIZE => {
            let _ = state.resize();
        }
        DISPATCH_COMMAND => {
            let cmd = state.commands.borrow_mut().pop_front();
            if let Some(cmd) = cmd {
                if state.current() {
                    (state.on_command)(cmd);
                }
            }
        }
        _ => return DefWindowProcW(hwnd, msg, wp, lp),
    }
    LRESULT(0)
}

unsafe fn create_window(state: &Rc<State>) -> WinResult<()> {
    let instance: HINSTANCE = GetModuleHandleW(None)?.into();
    let class = WNDCLASSW {
        lpfnWndProc: Some(window_proc),
        hInstance: instance,
        hCursor: LoadCursorW(None, IDC_ARROW)?,
        lpszClassName: CLASS,
        ..Default::default()
    };
    if RegisterClassW(&class) == 0 {
        let e = Error::from_win32();
        if e.code() != ERROR_CLASS_ALREADY_EXISTS.to_hresult() {
            return Err(e);
        }
    }
    let hwnd = CreateWindowExW(
        WINDOW_EX_STYLE::default(),
        CLASS,
        w!("TokenPulse"),
        WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        1040,
        720,
        None,
        None,
        instance,
        Some(Rc::as_ptr(state).cast()),
    )?;
    let dpi = GetDpiForWindow(hwnd);
    SetWindowPos(
        hwnd,
        None,
        0,
        0,
        pixels(1040, dpi),
        pixels(720, dpi),
        SWP_NOMOVE | SWP_NOZORDER,
    )?;
    let _ = ShowWindow(hwnd, SW_SHOWNORMAL);
    Ok(())
}

fn runtime_error(e: Error) -> DetailHostError {
    if [
        ERROR_FILE_NOT_FOUND,
        ERROR_PATH_NOT_FOUND,
        ERROR_PRODUCT_UNINSTALLED,
    ]
    .iter()
    .any(|code| code.to_hresult() == e.code())
    {
        DetailHostError::RuntimeMissing(e.code().0)
    } else {
        e.into()
    }
}

unsafe fn runtime() -> HostResult<()> {
    let mut version = PWSTR::null();
    let check = GetAvailableCoreWebView2BrowserVersionString(PCWSTR::null(), &mut version);
    let _owned = CoTaskMemPWSTR::from(version);
    check.map_err(runtime_error)?;
    if version.is_null() || *version.0 == 0 {
        return Err(DetailHostError::RuntimeMissing(
            ERROR_FILE_NOT_FOUND.to_hresult().0,
        ));
    }
    Ok(())
}

unsafe fn memory_stream(bytes: &[u8]) -> WinResult<IStream> {
    let mut raw = ptr::null_mut();
    CreateStreamOnHGlobal(HGLOBAL::default(), TRUE, &mut raw).ok()?;
    let stream = IStream::from_raw(raw);
    let mut written = 0;
    stream
        .Write(
            bytes.as_ptr().cast(),
            bytes.len() as u32,
            Some(&mut written),
        )
        .ok()?;
    if written as usize != bytes.len() {
        return Err(Error::from_hresult(E_FAIL));
    }
    stream.Seek(0, STREAM_SEEK_SET, None)?;
    Ok(stream)
}

#[cfg(test)]
#[test]
fn response_streams_start_at_zero_and_contain_exactly_the_embedded_bytes() {
    for path in ["/", "/index.html", "/app.css", "/app.js", "/logo.svg"] {
        let asset = resolve_asset(&format!("{}{path}", crate::APP_ORIGIN)).unwrap();
        unsafe {
            let stream = memory_stream(asset.bytes).unwrap();
            let mut body = vec![0; asset.bytes.len() + 1];
            let mut read = 0;
            stream
                .Read(body.as_mut_ptr().cast(), body.len() as u32, Some(&mut read))
                .ok()
                .unwrap();
            assert_eq!(read as usize, asset.bytes.len(), "{path}");
            assert_eq!(&body[..read as usize], asset.bytes, "{path}");
        }
    }
}

// Free the COM string even when the getter/UTF-16 conversion fails. Bound
// conversion before allocating UTF-8. security.rs checks the exact byte cap.
unsafe fn string_arg(
    get: impl FnOnce(*mut PWSTR) -> WinResult<()>,
    max_units: usize,
) -> WinResult<String> {
    let mut p = PWSTR::null();
    let status = get(&mut p);
    let _owned = CoTaskMemPWSTR::from(p);
    status?;
    if p.is_null() || p.as_wide().len() > max_units {
        return Err(Error::from_hresult(E_INVALIDARG));
    }
    p.to_string().map_err(|_| Error::from_hresult(E_INVALIDARG))
}

fn required<T>(arg: Option<T>) -> WinResult<T> {
    arg.ok_or_else(|| Error::from_hresult(E_POINTER))
}

unsafe fn install(
    state: &Rc<State>,
    env: &ICoreWebView2Environment,
    view: &ICoreWebView2,
    live: &mut Session,
) -> WinResult<()> {
    let settings = view.Settings()?;
    settings.SetAreDevToolsEnabled(false)?;
    settings.SetAreDefaultContextMenusEnabled(false)?;
    settings.SetIsStatusBarEnabled(false)?;
    settings.SetAreHostObjectsAllowed(false)?;
    settings.SetIsWebMessageEnabled(true)?;
    let keyboard: ICoreWebView2Settings3 = settings.cast()?;
    keyboard.SetAreBrowserAcceleratorKeysEnabled(false)?;
    let controller4: ICoreWebView2Controller4 = required(live.controller.clone())?.cast()?;
    controller4.SetAllowExternalDrop(false)?;

    // Record each successful subscription immediately so partial setup rolls back.
    macro_rules! subscribe {
        ($target:expr, $add:ident, $remove:ident, $handler:ident, $callback:expr) => {{
            let target = $target.clone();
            let mut token = EventRegistrationToken::default();
            target.$add(&$handler::create(Box::new($callback)), &mut token)?;
            live.remove_handlers
                .push(Box::new(move || target.$remove(token)));
        }};
    }
    let headers = format!("Content-Security-Policy: {STRICT_CSP}\r\nX-Content-Type-Options: nosniff\r\nCache-Control: no-store");
    let forbidden = env.CreateWebResourceResponse(
        None,
        403,
        w!("Forbidden"),
        &HSTRING::from(headers.as_str()),
    )?;
    let resources = env.clone();
    subscribe!(
        view,
        add_WebResourceRequested,
        remove_WebResourceRequested,
        WebResourceRequestedEventHandler,
        move |_, args| {
            let args = required(args)?;
            // Install the denial before any fallible lookup, allocation, or stream IO.
            args.SetResponse(&forbidden)?;
            let uri = args
                .Request()
                .and_then(|req| string_arg(|out| req.Uri(out), 256));
            if let Some(asset) = uri.ok().as_deref().and_then(resolve_asset) {
                let stream = memory_stream(asset.bytes)?;
                let headers =
                    HSTRING::from(format!("Content-Type: {}\r\n{headers}", asset.content_type));
                let ok = resources.CreateWebResourceResponse(&stream, 200, w!("OK"), &headers)?;
                args.SetResponse(&ok)?;
            }
            Ok(())
        }
    );
    view.AddWebResourceRequestedFilter(w!("*"), COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL)?;
    let all_sources: ICoreWebView2_22 = view.cast()?;
    all_sources.AddWebResourceRequestedFilterWithRequestSourceKinds(
        w!("*"),
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
        COREWEBVIEW2_WEB_RESOURCE_REQUEST_SOURCE_KINDS_ALL,
    )?;
    subscribe!(
        view,
        add_NavigationStarting,
        remove_NavigationStarting,
        NavigationStartingEventHandler,
        |_, args| {
            let args = required(args)?;
            if !string_arg(|out| args.Uri(out), 256)
                .is_ok_and(|uri| is_allowed_top_level_navigation(&uri))
            {
                args.SetCancel(true)?;
            }
            Ok(())
        }
    );
    subscribe!(
        view,
        add_FrameNavigationStarting,
        remove_FrameNavigationStarting,
        NavigationStartingEventHandler,
        |_, args| required(args)?.SetCancel(true)
    );
    subscribe!(
        view,
        add_NewWindowRequested,
        remove_NewWindowRequested,
        NewWindowRequestedEventHandler,
        |_, args| required(args)?.SetHandled(true)
    );
    subscribe!(
        view,
        add_PermissionRequested,
        remove_PermissionRequested,
        PermissionRequestedEventHandler,
        |_, args| required(args)?.SetState(COREWEBVIEW2_PERMISSION_STATE_DENY)
    );
    let downloads: ICoreWebView2_4 = view.cast()?;
    subscribe!(
        downloads,
        add_DownloadStarting,
        remove_DownloadStarting,
        DownloadStartingEventHandler,
        |_, args| required(args)?.SetCancel(true)
    );
    let external: ICoreWebView2_18 = view.cast()?;
    subscribe!(
        external,
        add_LaunchingExternalUriScheme,
        remove_LaunchingExternalUriScheme,
        LaunchingExternalUriSchemeEventHandler,
        |_, args| required(args)?.SetCancel(true)
    );

    let weak = Rc::downgrade(state);
    subscribe!(
        view,
        add_ProcessFailed,
        remove_ProcessFailed,
        ProcessFailedEventHandler,
        move |_, args| {
            let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND::default();
            required(args)?.ProcessFailedKind(&mut kind)?;
            if matches!(
                kind,
                COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED
                    | COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED
            ) {
                if let Some(state) = weak.upgrade().filter(|s| s.current()) {
                    state
                        .close_error
                        .replace(Some(DetailHostError::Win32(format!(
                            "The detail WebView2 process exited (ProcessFailedKind {})",
                            kind.0
                        ))));
                    PostMessageW(state.hwnd.get(), WM_CLOSE, WPARAM(0), LPARAM(0))?;
                }
            }
            Ok(())
        }
    );
    let weak = Rc::downgrade(state);
    let limiter = RefCell::new(MessageRateLimiter::new());
    // This is the top-level view event. Frame message events are never registered.
    subscribe!(
        view,
        add_WebMessageReceived,
        remove_WebMessageReceived,
        WebMessageReceivedEventHandler,
        move |_, args| {
            let Some(state) = weak.upgrade().filter(|s| s.current()) else {
                return Ok(());
            };
            let args = required(args)?;
            let source = string_arg(|out| args.Source(out), 256)?;
            if !is_allowed_top_level_navigation(&source) {
                return Ok(());
            }
            let data = string_arg(|out| args.WebMessageAsJson(out), 16 * 1024)?;
            if let Ok(cmd) =
                validate_incoming_message(&source, &data, &mut limiter.borrow_mut(), Instant::now())
            {
                // Dispatch after returning to the STA pump. The caller may close
                // the host from on_command without nesting a pump in a COM event.
                let mut commands = state.commands.borrow_mut();
                if commands.len() < 30 {
                    PostMessageW(state.hwnd.get(), DISPATCH_COMMAND, WPARAM(0), LPARAM(0))?;
                    commands.push_back(cmd);
                }
            }
            Ok(())
        }
    );
    Ok(())
}

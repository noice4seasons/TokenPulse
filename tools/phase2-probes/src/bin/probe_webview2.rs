use serde_json::{json, Value};
use std::{
    cell::Cell,
    ffi::OsStr,
    mem::size_of,
    os::windows::ffi::OsStrExt,
    path::Path,
    rc::Rc,
    sync::mpsc::{channel, Receiver, TryRecvError},
    time::{Duration, Instant},
};
use webview2_com::{Microsoft::Web::WebView2::Win32::*, *};
use windows::{
    core::{w, Error, Interface, Result, PCWSTR, PWSTR},
    Win32::{
        Foundation::*,
        System::{Com::*, ProcessStatus::*, Threading::GetCurrentProcess},
        UI::{HiDpi::*, WindowsAndMessaging::*},
    },
};

fn wide(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain(Some(0)).collect()
}

unsafe fn wait<T>(rx: &Receiver<T>, ms: u64) -> Result<T> {
    let deadline = Instant::now() + Duration::from_millis(ms);
    loop {
        match rx.try_recv() {
            Ok(v) => return Ok(v),
            Err(TryRecvError::Disconnected) => return Err(Error::from_hresult(E_ABORT)),
            Err(TryRecvError::Empty) => {}
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(Error::from_hresult(ERROR_TIMEOUT.to_hresult()));
        }
        let mut msg = MSG::default();
        if PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
            if msg.message == WM_QUIT {
                return Err(Error::from_hresult(E_ABORT));
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        } else {
            MsgWaitForMultipleObjectsEx(
                None,
                remaining.as_millis().max(1) as u32,
                QS_ALLINPUT,
                MWMO_INPUTAVAILABLE,
            );
        }
    }
}

struct Window(HWND);
impl Drop for Window {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.0);
        }
    }
}
unsafe fn environment(udf: &Path, browser: Option<&Path>) -> Result<ICoreWebView2Environment> {
    let folder = wide(udf.as_os_str());
    let browser = browser.map(|p| wide(p.as_os_str()));
    let (send, recv) = channel();
    let callback =
        CreateCoreWebView2EnvironmentCompletedHandler::create(Box::new(move |status, env| {
            let _ = send.send(status.and_then(|()| env.ok_or(Error::from_hresult(E_POINTER))));
            Ok(())
        }));
    CreateCoreWebView2EnvironmentWithOptions(
        browser
            .as_ref()
            .map_or(PCWSTR::null(), |p| PCWSTR(p.as_ptr())),
        PCWSTR(folder.as_ptr()),
        None,
        &callback,
    )?;
    wait(&recv, 30000)?
}

unsafe fn memory() -> Result<(f64, f64)> {
    let mut c = PROCESS_MEMORY_COUNTERS_EX::default();
    let size = size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
    c.cb = size;
    GetProcessMemoryInfo(
        GetCurrentProcess(),
        (&mut c as *mut PROCESS_MEMORY_COUNTERS_EX).cast(),
        size,
    )?;
    Ok((
        c.PrivateUsage as f64 / 1048576.,
        c.WorkingSetSize as f64 / 1048576.,
    ))
}

unsafe fn cycle(udf: &Path, generation: &Rc<Cell<u64>>, cancel: bool) -> Result<Value> {
    let start = Instant::now();
    let window = Window(CreateWindowExW(
        WS_EX_TOOLWINDOW,
        w!("STATIC"),
        w!("TokenPulse"),
        WS_POPUP,
        0,
        0,
        64,
        64,
        None,
        None,
        None,
        None,
    )?);
    let env = environment(udf, None)?;
    let events: ICoreWebView2Environment5 = env.cast()?;
    let (exit_tx, exit_rx) = channel();
    let mut exit_token = Default::default();
    events.add_BrowserProcessExited(
        &BrowserProcessExitedEventHandler::create(Box::new(move |_, _| {
            let _ = exit_tx.send(Instant::now());
            Ok(())
        })),
        &mut exit_token,
    )?;
    let opened = generation.get() + 1;
    generation.set(opened);
    let current = generation.clone();
    let (tx, rx) = channel();
    let callback =
        CreateCoreWebView2ControllerCompletedHandler::create(Box::new(move |hr, controller| {
            let outcome = if current.get() != opened {
                controller.map_or(Ok(()), |c| c.Close()).map(|()| None)
            } else {
                hr.and_then(|()| controller.ok_or(Error::from_hresult(E_POINTER)))
                    .map(Some)
            };
            let _ = tx.send(outcome);
            Ok(())
        }));
    env.CreateCoreWebView2Controller(window.0, &callback)?;
    let window = Some(window);
    if cancel {
        // Advance generation while controller creation is in-flight so the late
        // callback must discard & Close() the newly created controller cleanly.
        generation.set(opened + 1);
    }
    let controller = wait(&rx, 30000)??;
    let stale_discarded = controller.is_none();
    let mut open_ms = None;
    let closing;
    if let Some(controller) = controller {
        controller.SetBounds(RECT {
            left: 0,
            top: 0,
            right: 64,
            bottom: 64,
        })?;
        let view = controller.CoreWebView2()?;
        let (tx, rx) = channel();
        let mut token = Default::default();
        view.add_NavigationCompleted(
            &NavigationCompletedEventHandler::create(Box::new(move |_, args| {
                let mut success = BOOL::default();
                let result = args
                    .ok_or(Error::from_hresult(E_POINTER))
                    .and_then(|a| a.IsSuccess(&mut success))
                    .map(|()| success.as_bool());
                let _ = tx.send(result);
                Ok(())
            })),
            &mut token,
        )?;
        let nav = view
            .NavigateToString(w!("<!doctype html><meta charset=utf-8><p>TokenPulse</p>"))
            .and_then(|()| wait(&rx, 15000))
            .and_then(|v| v);
        view.remove_NavigationCompleted(token)?;
        open_ms = Some(start.elapsed().as_millis());
        closing = Instant::now();
        controller.Close()?;
        drop(controller);
        drop(view);
        if !nav? {
            return Err(Error::from_hresult(E_FAIL));
        }
    } else {
        closing = Instant::now();
    }
    drop(callback);
    drop(env);
    drop(window);
    // Keep the exit subscription alive until the notification or its deadline.
    let exit = match wait(&exit_rx, 5000) {
        Ok(at) => Some(at.saturating_duration_since(closing).as_millis()),
        Err(e) if e.code() == ERROR_TIMEOUT.to_hresult() => None,
        Err(e) => return Err(e),
    };
    events.remove_BrowserProcessExited(exit_token)?;
    drop(events);
    let (private, working) = memory()?;
    Ok(
        json!({"open_generation":opened,"stale_callback_discarded":stale_discarded,
        "open_ms":open_ms,"close_to_exit_ms":exit,"browser_exited":exit.is_some(),
        "private_bytes_mb":private,"working_set_mb":working}),
    )
}

unsafe fn runtime() -> Result<String> {
    let mut p = PWSTR::null();
    GetAvailableCoreWebView2BrowserVersionString(PCWSTR::null(), &mut p)?;
    let s = p.to_string().map_err(|_| Error::from_hresult(E_UNEXPECTED));
    CoTaskMemFree(Some(p.0.cast()));
    s
}

unsafe fn run() -> Result<Value> {
    let udf = std::env::temp_dir().join(format!("tokenpulse-probe-wv2-udf-{}", std::process::id()));
    std::fs::create_dir(&udf).map_err(Error::from)?;
    let missing = environment(&udf, Some(&udf.join("absent-runtime")))
        .err()
        .map(|e| e.code().0);
    let missing_handled = missing.is_some_and(|c| {
        [
            ERROR_FILE_NOT_FOUND,
            ERROR_PATH_NOT_FOUND,
            ERROR_PRODUCT_UNINSTALLED,
        ]
        .iter()
        .any(|e| e.to_hresult().0 == c)
    });
    let version = runtime();
    let mut rounds = Vec::new();
    let generation = Rc::new(Cell::new(0));
    let attempt = |cancel| {
        cycle(&udf, &generation, cancel)
            .unwrap_or_else(|e| json!({"hresult":e.code().0,"error":e.to_string()}))
    };
    let cancellation = if version.is_ok() {
        for _ in 0..10 {
            rounds.push(attempt(false));
        }
        attempt(true)
    } else {
        Value::Null
    };
    let ok = missing_handled
        && rounds.len() == 10
        && rounds.iter().all(|r| r["browser_exited"] == true)
        && cancellation["stale_callback_discarded"] == true
        && cancellation["browser_exited"] == true;
    let cleanup = std::fs::remove_dir_all(&udf).err().map(|e| e.to_string());
    Ok(
        json!({"passed":ok,"runtime_version":version.as_ref().ok(),"runtime_hresult":version.err().map(|e|e.code().0),
      "rounds":rounds,"cancellation":cancellation,"missing_runtime_hresult":missing,
      "missing_runtime_handled":missing_handled,"udf_cleanup_error":cleanup}),
    )
}

fn main() {
    unsafe {
        let result = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)
            .and_then(|()| CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok())
            .and_then(|()| {
                let v = run();
                CoUninitialize();
                v
            });
        let report = result.unwrap_or_else(|e| json!({"error":e.to_string(),"hresult":e.code().0}));
        println!("{report}");
        if report["passed"] != true {
            std::process::exit(1);
        }
    }
}

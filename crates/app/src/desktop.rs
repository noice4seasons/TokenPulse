use crate::{
    config_store::{load_state, save_state, AppPaths, PersistedState},
    export_dialog::save_export,
    pipeline::{sync_antigravity_once, sync_codex_once},
    single_instance::{activate_message_id, InstanceGuard},
};
use std::{
    cell::RefCell,
    collections::HashMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokenpulse_core_domain::{
    config::{AppConfig, DisplayPlacement, Language, Theme},
    dto::{
        Coverage, DashboardSnapshotDto, OverlaySnapshotDto, PrivacyBadge, QuotaBarDto,
        PROTOCOL_VERSION,
    },
    models::ProviderId,
    quota::{
        should_alert_low_quota, ProviderAvailability, QuotaObservation, QuotaPeriodKind,
        QuotaWindowState,
    },
};
use tokenpulse_provider_antigravity::{AntigravityRoots, AntigravityScanner};
use tokenpulse_provider_codex::CodexRoots;
use tokenpulse_scheduler::{
    debounce::{DebounceQueue, DirtyKey},
    power::{CancelToken, CancellationController, PauseReason, ResumeAction},
    watcher::{DirectoryWatcher, WatchEvent},
};
use tokenpulse_store_sqlite::{ExactRange, ExportFormat, StoreHandle, TimeZoneSpec};
use tokenpulse_ui_detail_web::{DetailHostError, DetailWindowHost, HostCommand};
use tokenpulse_ui_win32::{
    self as native, format_reset_duration, is_pro_plan, OverlayMode, OverlayWindow,
    PowerSessionEvent, TrayCommand, TrayHost, TrayIconState, TrayMenuState,
    OVERLAY_ACTION_CONTEXT_MENU, OVERLAY_ACTION_MODE_TOGGLED, OVERLAY_ACTION_OPEN_DETAIL,
    TRAY_MESSAGE,
};
use windows::{
    core::{w, Error, HSTRING},
    Win32::{
        Foundation::*,
        System::{
            Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED},
            LibraryLoader::GetModuleHandleW,
            ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX},
            Threading::{GetCurrentProcess, GetCurrentThreadId},
        },
        UI::{
            Shell::{Shell_NotifyIconGetRect, NIN_SELECT, NOTIFYICONIDENTIFIER},
            WindowsAndMessaging::*,
        },
    },
};

const WM_SYNC_DONE: u32 = WM_APP + 0x42;
const WM_OVERLAY_ACTION: u32 = WM_APP + 0x43;
const TRAY_ID: u16 = 1;
const FALLBACK: Duration = Duration::from_secs(60);
const BENCH_TIMER: usize = 1;
type AppResult<T> = Result<T, String>;
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}

// Scheduler generations protect publications. Each generation also has its
// own SQLite progress flag so a quick resume cannot revive an old query.
#[derive(Default)]
struct Gate {
    ctl: CancellationController,
    sql_cancel: Mutex<Arc<AtomicBool>>,
}
impl Gate {
    fn begin(&self) -> Option<(CancelToken, Arc<AtomicBool>)> {
        let flag = self.sql_cancel.lock().unwrap();
        self.ctl.active_token().map(|token| (token, flag.clone()))
    }
    fn interrupt(&self, change: impl FnOnce(&CancellationController)) {
        let mut flag = self.sql_cancel.lock().unwrap();
        change(&self.ctl);
        flag.store(true, Ordering::Release);
        *flag = Arc::default();
    }
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
struct Filter {
    provider: Option<ProviderId>,
    days: u32,
}
impl Filter {
    fn new(provider: &str, range: &str) -> AppResult<Self> {
        Ok(Self {
            provider: match provider {
                "all" => None,
                "codex" => Some(ProviderId::Codex),
                "antigravity" => Some(ProviderId::Antigravity),
                _ => return Err("Invalid provider".into()),
            },
            days: match range {
                "today" => 1,
                "7d" => 7,
                "30d" => 30,
                _ => return Err("Invalid range".into()),
            },
        })
    }
}

// Use the same explicit UTC reporting calendar as pipeline::run_verification.
fn calendar() -> TimeZoneSpec {
    TimeZoneSpec::FixedOffset { offset_minutes: 0 }
}
fn utc_bounds(now: i64, days: u32) -> (i64, i64) {
    const DAY: i64 = 86_400_000;
    let end = now - now.rem_euclid(DAY) + DAY;
    (end - i64::from(days.max(1)) * DAY, end)
}

struct Frame {
    overlay: OverlaySnapshotDto,
    ag_today_tokens: Option<u64>,
    quotas: Vec<QuotaObservation>,
    scope: Option<String>,
}

fn effective(q: &QuotaObservation) -> i64 {
    q.event_time_ms
        .filter(|_| q.event_time_trusted)
        .unwrap_or(q.ingested_at_ms)
}

fn cached_frame(store: &StoreHandle, now: i64) -> AppResult<Frame> {
    let (lo, hi) = utc_bounds(now, 1);
    let today = store
        .query_exact_range(lo, hi, None)
        .map_err(err)?
        .aggregate;
    let ag = store
        .query_exact_range(lo, hi, Some(ProviderId::Antigravity))
        .map_err(err)?
        .aggregate;
    let mut quotas = store
        .latest_quotas(Some(ProviderId::Codex), None)
        .map_err(err)?;
    let scope = quotas
        .iter()
        .max_by_key(|q| effective(q))
        .map(|q| q.scope_id.clone());
    let scope = match scope {
        Some(id) => Some(id),
        None => store
            .latest_credits(Some(ProviderId::Codex), None)
            .map_err(err)?
            .into_iter()
            .max_by_key(|c| c.event_time_ms.unwrap_or(c.ingested_at_ms))
            .map(|c| c.scope_id),
    };
    quotas.retain(|q| Some(&q.scope_id) == scope.as_ref());
    let codex_plan_type = store
        .latest_credits(Some(ProviderId::Codex), scope.clone())
        .map_err(err)?
        .into_iter()
        .max_by_key(|c| c.event_time_ms.unwrap_or(c.ingested_at_ms))
        .and_then(|c| c.plan_type);
    let metrics = today.metrics;
    let prompt =
        u128::from(metrics.non_cached_input_tokens) + u128::from(metrics.cache_read_tokens);
    let overlay = OverlaySnapshotDto {
        protocol_version: PROTOCOL_VERSION,
        codex: quotas
            .iter()
            .map(|q| QuotaBarDto {
                target_label: q.target_label.clone(),
                period_kind: q.period_kind,
                state: q.window_state(now),
                observed_at_ms: Some(effective(q)),
                account_confirmed: q.account_confirmed,
            })
            .collect(),
        codex_plan_type,
        antigravity: Vec::new(),
        today_tokens: (today.coverage != Coverage::Unknown).then_some(metrics.total_tokens),
        today_coverage: today.coverage,
        cache_hit_ratio: (prompt > 0).then(|| metrics.cache_read_tokens as f64 / prompt as f64),
        latest_turn_cache_hit_ratio: store.latest_turn_cache_hit_ratio(None).map_err(err)?,
        privacy_badge: PrivacyBadge::LocalOnly,
        updated_at_ms: now,
    };
    Ok(Frame {
        overlay,
        quotas,
        scope,
        ag_today_tokens: (ag.coverage != Coverage::Unknown).then_some(ag.metrics.total_tokens),
    })
}

fn selected_range(store: &StoreHandle, now: i64, filter: Filter) -> AppResult<ExactRange> {
    let (lo, hi) = utc_bounds(now, filter.days);
    store
        .query_exact_range(lo, hi, filter.provider)
        .map_err(err)
}

fn dashboard(
    store: &StoreHandle,
    now: i64,
    scope: Option<String>,
    filter: Filter,
) -> AppResult<DashboardSnapshotDto> {
    let mut d = store
        .query_dashboard_snapshot(now, calendar(), scope)
        .map_err(err)?;
    if let Some(p) = filter.provider {
        for aggregate in [&mut d.today, &mut d.last_7_days, &mut d.last_30_days] {
            *aggregate = store
                .query_exact_range(aggregate.from_ms, aggregate.to_ms, Some(p))
                .map_err(err)?
                .aggregate;
        }
        d.priced_token_coverage_ratio = d.today.metrics.priced_token_coverage_ratio();
        d.latest_turn_cache_hit_ratio = store.latest_turn_cache_hit_ratio(Some(p)).map_err(err)?;
        d.providers.retain(|card| card.provider == p);
    }
    if filter.provider.is_some() || filter.days > 1 {
        d.model_distribution = selected_range(store, now, filter)?.models;
    }
    if filter.provider.is_some() {
        d.daily_heatmap = store
            .query_daily_heatmap(now, 30, calendar(), filter.provider)
            .map_err(err)?;
    }
    Ok(d)
}

enum Read {
    Initial,
    Snapshot(CancelToken, Arc<AtomicBool>),
    Detail(u64, Filter, Option<ExportFormat>, Arc<AtomicBool>),
    Stop,
}
enum Work {
    Watch(WatchEvent),
    Refresh { full: bool },
    Stop,
}
impl From<WatchEvent> for Work {
    fn from(value: WatchEvent) -> Self {
        Self::Watch(value)
    }
}
enum Event {
    Cached(Box<Frame>),
    Synced(CancelToken, Box<(Frame, DashboardSnapshotDto)>),
    Detail(u64, Arc<AtomicBool>, Box<DashboardSnapshotDto>),
    Export(u64, Arc<AtomicBool>, ExportFormat, Box<ExactRange>),
    Web(u64, HostCommand),
    Tray(isize),
    OverlayAction(usize),
    Open,
    Power,
    Exit,
    Error(String),
    Fatal(String),
}

#[derive(Clone)]
struct Mailbox {
    // HWND is STA-owned. Other threads use its numeric value only for PostMessageW.
    hwnd: usize,
    sender: Sender<Event>,
}
impl Mailbox {
    fn post(&self, event: Event) {
        if self.sender.send(event).is_ok() {
            unsafe {
                let _ = PostMessageW(HWND(self.hwnd as _), WM_SYNC_DONE, WPARAM(0), LPARAM(0));
            }
        }
    }
}

fn read_loop(store: StoreHandle, rx: Receiver<Read>, ui: Mailbox, shutdown: Arc<AtomicBool>) {
    let reader = store.with_cancellation(shutdown.clone());
    let mut scope = None;
    while let Ok(job) = rx.recv() {
        if shutdown.load(Ordering::Acquire) {
            break;
        }
        let now = now_ms();
        match job {
            Read::Stop => break,
            Read::Initial => match cached_frame(&reader, now) {
                Ok(frame) => {
                    scope.clone_from(&frame.scope);
                    ui.post(Event::Cached(Box::new(frame)));
                }
                Err(e) => {
                    ui.post(Event::Fatal(e));
                    break;
                }
            },
            Read::Snapshot(token, flag) => {
                if token.is_cancelled() {
                    continue;
                }
                let live = store.with_cancellation(flag);
                let result = cached_frame(&live, now).and_then(|f| {
                    let d = dashboard(&live, now, f.scope.clone(), Filter::default())?;
                    Ok((f, d))
                });
                token.publish_if_current(|| match result {
                    Ok(pair) => {
                        scope.clone_from(&pair.0.scope);
                        ui.post(Event::Synced(token.clone(), Box::new(pair)));
                    }
                    Err(error) => ui.post(Event::Error(error)),
                });
            }
            Read::Detail(id, filter, format, flag) => {
                if flag.load(Ordering::Acquire) {
                    continue;
                }
                let reader = store.with_cancellation(flag.clone());
                let result = if let Some(format) = format {
                    selected_range(&reader, now, filter)
                        .map(|r| Event::Export(id, flag.clone(), format, Box::new(r)))
                } else {
                    dashboard(&reader, now, scope.clone(), filter)
                        .map(|d| Event::Detail(id, flag.clone(), Box::new(d)))
                };
                if !flag.load(Ordering::Acquire) {
                    ui.post(result.unwrap_or_else(Event::Error));
                }
            }
        }
    }
}

fn scan_loop(
    store: &StoreHandle,
    settings: &Mutex<AppConfig>,
    gate: &Gate,
    tx: Sender<Work>,
    rx: Receiver<Work>,
    reads: &Sender<Read>,
    ui: &Mailbox,
) -> AppResult<()> {
    let codex = CodexRoots::from_env().map_err(err);
    let ag = AntigravityRoots::from_env().map_err(err);
    let mut scanners: HashMap<String, AntigravityScanner> = HashMap::new();
    let mut watcher: Option<DirectoryWatcher<Work>> = None;
    let mut queue = DebounceQueue::new(Default::default());
    let mut periodic = Instant::now() + FALLBACK;
    let mut full = false;
    let mut started = false;
    loop {
        let now = Instant::now();
        if now >= periodic {
            periodic = now + FALLBACK;
            if started {
                queue.reconcile_all(now);
            }
        }
        if !queue.drain_ready(now).is_empty() {
            if let Some((token, flag)) = gate.begin() {
                let rebuild = std::mem::take(&mut full);
                // The fixed pipeline API has no scan-cancellation parameter.
                // A call already inside a file may finish and commit. Check the
                // generation again before starting another provider or publishing.
                let live = store.with_cancellation(flag.clone());
                let config = settings.lock().unwrap().clone();
                if config.codex.enabled {
                    let result = codex.as_ref().map_err(Clone::clone).and_then(|codex| {
                        let watch_result = match &mut watcher {
                            Some(w) => w.refresh(),
                            None => DirectoryWatcher::start(&codex.home, tx.clone())
                                .map(|w| watcher = Some(w)),
                        };
                        if let Err(e) = watch_result {
                            ui.post(Event::Error(err(e)));
                        }
                        if !token.is_cancelled() && codex.scan_roots().iter().any(|p| p.is_dir()) {
                            sync_codex_once(codex, &live, now_ms(), rebuild)?;
                        }
                        Ok(())
                    });
                    if let Err(e) = result {
                        if !token.is_cancelled() {
                            ui.post(Event::Error(e));
                        }
                    }
                } else {
                    watcher.take();
                }
                if !token.is_cancelled() && config.antigravity.enabled {
                    let result = ag.as_ref().map_err(Clone::clone).and_then(|ag| {
                        if ag.brain_dir.is_dir() {
                            sync_antigravity_once(ag, &live, &mut scanners, now_ms(), rebuild)?;
                        }
                        Ok(())
                    });
                    if let Err(e) = result {
                        if !token.is_cancelled() {
                            ui.post(Event::Error(e));
                        }
                    }
                }
                if !token.is_cancelled() {
                    reads.send(Read::Snapshot(token, flag)).map_err(err)?;
                }
            }
        }
        let deadline = queue.next_deadline().unwrap_or(periodic).min(periodic);
        match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(Work::Watch(WatchEvent::Dirty(_))) => {
                queue.mark_dirty(DirtyKey::Source(()), Instant::now())
            }
            Ok(Work::Watch(WatchEvent::OverflowReconcile)) => queue.reconcile_all(Instant::now()),
            Ok(Work::Refresh { full: rebuild }) => {
                started = true;
                full |= rebuild;
                queue.reconcile_all(Instant::now());
            }
            Ok(Work::Stop) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {}
        }
    }
    if let Some(mut w) = watcher {
        w.stop().map_err(err)?;
    }
    Ok(())
}

struct Workers {
    tx: Sender<Work>,
    settings: Arc<Mutex<AppConfig>>,
    reads: Sender<Read>,
    gate: Arc<Gate>,
    shutdown: Arc<AtomicBool>,
    join: Option<JoinHandle<AppResult<()>>>,
}
impl Workers {
    fn start(
        path: PathBuf,
        config: AppConfig,
        ui: Mailbox,
        gate: Arc<Gate>,
        tx: Sender<Work>,
        rx: Receiver<Work>,
    ) -> AppResult<Self> {
        let (reads, queries) = mpsc::channel();
        reads.send(Read::Initial).map_err(err)?;
        let shutdown = Arc::new(AtomicBool::new(false));
        let stop = shutdown.clone();
        let cancel = gate.clone();
        let requests = tx.clone();
        let reader_tx = reads.clone();
        let settings = Arc::new(Mutex::new(config));
        let worker_settings = settings.clone();
        let join = thread::Builder::new()
            .name("TokenPulse sync".into())
            .spawn(move || {
                let run = || -> AppResult<()> {
                    let store = StoreHandle::open(path).map_err(err)?;
                    let source = store.clone();
                    let target = ui.clone();
                    let reader = thread::Builder::new()
                        .name("TokenPulse cache".into())
                        .spawn(move || read_loop(source, queries, target, stop))
                        .map_err(err)?;
                    let outcome = scan_loop(
                        &store,
                        &worker_settings,
                        &cancel,
                        requests,
                        rx,
                        &reader_tx,
                        &ui,
                    );
                    let _ = reader_tx.send(Read::Stop);
                    reader.join().map_err(|_| "Cache thread panicked")?;
                    outcome
                };
                let result = run();
                if let Err(e) = &result {
                    ui.post(Event::Fatal(e.clone()));
                }
                result
            })
            .map_err(err)?;
        Ok(Self {
            tx,
            settings,
            reads,
            gate,
            shutdown,
            join: Some(join),
        })
    }
    fn stop(&mut self) -> AppResult<()> {
        let Some(join) = self.join.take() else {
            return Ok(());
        };
        self.shutdown.store(true, Ordering::Release);
        self.gate.interrupt(|c| {
            c.disable();
        });
        let _ = self.tx.send(Work::Stop);
        let _ = self.reads.send(Read::Stop);
        join.join().map_err(|_| "Sync thread panicked")?
    }
}
impl Drop for Workers {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

// Keep application code outside the window procedure. Menus, common dialogs,
// and WebView2 initialization/teardown can all run nested message pumps.
struct Route {
    ui: Mailbox,
    gate: Arc<Gate>,
    work: Sender<Work>,
    activate: u32,
}
thread_local! { static ROUTE: RefCell<Option<Route>> = const { RefCell::new(None) }; }

unsafe extern "system" fn host_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_SYNC_DONE {
        return LRESULT(0);
    }
    let handled = ROUTE.with(|slot| {
        let route = slot.borrow();
        let Some(route) = route.as_ref() else {
            return false;
        };
        let event = if msg == TRAY_MESSAGE {
            Event::Tray(lp.0)
        } else if msg == WM_OVERLAY_ACTION {
            Event::OverlayAction(wp.0)
        } else if msg != 0 && msg == route.activate {
            Event::Open
        } else if msg == WM_CLOSE {
            Event::Exit
        } else if let Some(change) = native::classify_window_message(msg, wp.0) {
            let (reason, pause) = match change {
                PowerSessionEvent::Suspended => (PauseReason::SystemSuspended, true),
                PowerSessionEvent::Resumed => (PauseReason::SystemSuspended, false),
                PowerSessionEvent::SessionLocked => (PauseReason::ScreenLocked, true),
                PowerSessionEvent::SessionUnlocked => (PauseReason::ScreenLocked, false),
            };
            if pause {
                route.gate.interrupt(|controller| {
                    controller.pause(reason);
                });
            } else if route.gate.ctl.resume(reason) == ResumeAction::FullReconcile {
                let _ = route.work.send(Work::Refresh { full: true });
            }
            Event::Power
        } else {
            return false;
        };
        route.ui.post(event);
        true
    });
    if handled {
        LRESULT(isize::from(msg == WM_POWERBROADCAST))
    } else {
        unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
    }
}

struct HostWindow {
    hwnd: HWND,
    instance: HINSTANCE,
    registered: bool,
    subscribed: bool,
}
impl HostWindow {
    fn create() -> AppResult<Self> {
        unsafe {
            let instance = GetModuleHandleW(None).map_err(err)?.into();
            let class = WNDCLASSW {
                lpfnWndProc: Some(host_proc),
                lpszClassName: w!("TokenPulse.AppHost"),
                hInstance: instance,
                ..Default::default()
            };
            let registered = RegisterClassW(&class) != 0;
            if !registered && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
                return Err(err(Error::from_win32()));
            }
            let mut host = Self {
                hwnd: HWND::default(),
                instance,
                registered,
                subscribed: false,
            };
            host.hwnd = CreateWindowExW(
                WS_EX_TOOLWINDOW,
                class.lpszClassName,
                w!("TokenPulse"),
                WS_POPUP,
                0,
                0,
                0,
                0,
                None,
                None,
                instance,
                None,
            )
            .map_err(err)?;
            native::register_session_notifications(host.hwnd).map_err(err)?;
            host.subscribed = true;
            Ok(host)
        }
    }
}
impl Drop for HostWindow {
    fn drop(&mut self) {
        ROUTE.with(|r| {
            r.borrow_mut().take();
        });
        if self.subscribed {
            let _ = native::unregister_session_notifications(self.hwnd);
        }
        unsafe {
            if !self.hwnd.is_invalid() {
                let _ = DestroyWindow(self.hwnd);
            }
            if self.registered {
                let _ = UnregisterClassW(w!("TokenPulse.AppHost"), self.instance);
            }
        }
    }
}
struct Apartment;
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}

fn detail_hwnd() -> AppResult<HWND> {
    // The supplied host keeps its HWND private. Restrict lookup to the thread
    // that owns this host so a concurrent benchmark cannot receive focus.
    unsafe {
        let mut after = HWND::default();
        loop {
            let hwnd =
                FindWindowExW(None, after, w!("TokenPulse.Detail.Web"), None).map_err(err)?;
            if GetWindowThreadProcessId(hwnd, None) == GetCurrentThreadId() {
                return Ok(hwnd);
            }
            after = hwnd;
        }
    }
}

struct Desktop {
    hwnd: HWND,
    paths: AppPaths,
    state: PersistedState,
    tray: Option<TrayHost>,
    overlay: Option<OverlayWindow>,
    detail: Option<DetailWindowHost>,
    detail_session: u64,
    view_revision: u64,
    ready: bool,
    filter: Filter,
    frame: Option<Frame>,
    latest: Option<DashboardSnapshotDto>,
    paused: bool,
    bench: bool,
    ui: Mailbox,
    workers: Workers,
}
impl Desktop {
    fn mode(&self) -> OverlayMode {
        if self.state.overlay_compact {
            OverlayMode::CompactCapsule
        } else {
            OverlayMode::ExpandedCard
        }
    }
    fn setup_overlay(&mut self) -> AppResult<()> {
        if !self.state.overlay_visible {
            if let Some(o) = &self.overlay {
                if self.state.config.display_placement != DisplayPlacement::Taskbar {
                    self.state.overlay_pos = Some(o.position().map_err(err)?);
                }
            }
            self.overlay.take();
            return Ok(());
        }
        let mode = self.mode();
        let fresh = self.overlay.is_none();
        if fresh {
            let created = OverlayWindow::create(mode, self.state.overlay_pos).map_err(err)?;
            created.set_action_host(self.hwnd, WM_OVERLAY_ACTION);
            self.overlay = Some(created);
        }
        let o = self.overlay.as_ref().unwrap();
        let config = &self.state.config;
        o.set_mode(mode).map_err(err)?;
        o.set_theme(config.theme).map_err(err)?;
        o.set_language(config.language).map_err(err)?;
        o.set_topmost(config.always_on_top).map_err(err)?;
        o.set_click_through(config.click_through).map_err(err)?;
        o.set_opacity(config.overlay_opacity_pct).map_err(err)?;
        o.set_placement(config.display_placement).map_err(err)?;
        self.paint_overlay()?;
        if fresh {
            o.show().map_err(err)?;
        }
        Ok(())
    }
    fn paint_overlay(&self) -> AppResult<()> {
        if let (Some(o), Some(f)) = (&self.overlay, &self.frame) {
            let mut dto = f.overlay.clone();
            if !self.state.config.codex.enabled {
                dto.codex.clear();
            }
            let ag = if self.state.config.antigravity.enabled {
                f.ag_today_tokens
            } else {
                None
            };
            o.update_snapshot_with_offline_tokens(dto, ag)
                .map_err(err)?;
        }
        Ok(())
    }
    fn tray_status(&mut self) -> AppResult<()> {
        let active = self.workers.gate.ctl.active_token().is_some();
        let cfg = &self.state.config;
        let now = now_ms();
        let threshold = f64::from(cfg.low_quota_threshold_pct);
        let mut status = TrayIconState::Offline;
        let mut lines = Vec::new();
        if let Some(f) = &self.frame {
            let hide_5h = is_pro_plan(f.overlay.codex_plan_type.as_deref());
            if cfg.codex.enabled && !f.quotas.is_empty() {
                let relevant: Vec<_> = f
                    .quotas
                    .iter()
                    .filter(|q| !(hide_5h && q.period_kind == QuotaPeriodKind::FiveHour))
                    .collect();
                for q in &relevant {
                    let period = match q.period_kind {
                        QuotaPeriodKind::FiveHour => "5h",
                        QuotaPeriodKind::Weekly => "7d",
                        QuotaPeriodKind::Unknown(_) => continue,
                    };
                    if let QuotaWindowState::Active {
                        remaining_pct,
                        resets_in_ms,
                        ..
                    } = q.window_state(now)
                    {
                        let reset_part = resets_in_ms
                            .filter(|ms| *ms > 0)
                            .map(|ms| {
                                let dur = format_reset_duration(ms, cfg.language);
                                if cfg.language == Language::ZhCn {
                                    format!(" · {dur}后重置")
                                } else {
                                    format!(" · {dur}")
                                }
                            })
                            .unwrap_or_default();
                        lines.push(format!("Codex {period}: {remaining_pct:.0}%{reset_part}"));
                    }
                }
                status = if relevant
                    .iter()
                    .any(|q| should_alert_low_quota(q, threshold, now))
                {
                    TrayIconState::LowQuota
                } else {
                    TrayIconState::Healthy
                };
            }
            if let Some(tokens) = f.overlay.today_tokens {
                let label = if cfg.language == Language::ZhCn {
                    "今日"
                } else {
                    "Today"
                };
                lines.push(format!("{label}: {tokens} tokens"));
                if status == TrayIconState::Offline {
                    status = TrayIconState::Healthy;
                }
            }
        }
        let tooltip = lines.join("\n");
        if !active {
            status = TrayIconState::Paused;
        }
        if self.tray.is_none() {
            self.tray = TrayHost::new(self.hwnd, TRAY_ID, status, cfg.language).ok();
        }
        if let Some(tray) = &mut self.tray {
            let _ = tray.update(status, cfg.language, &tooltip);
            if active && cfg.codex.enabled && !self.bench {
                if let Some(f) = &self.frame {
                    let hide_5h = is_pro_plan(f.overlay.codex_plan_type.as_deref());
                    for q in &f.quotas {
                        if hide_5h && q.period_kind == QuotaPeriodKind::FiveHour {
                            continue;
                        }
                        let _ = tray.notify_low_quota(
                            q,
                            threshold,
                            now,
                            i64::from(cfg.cooldown_minutes) * 60_000,
                        );
                    }
                }
            }
        }
        Ok(())
    }
    fn persist(&mut self) -> AppResult<()> {
        if self.bench {
            return Ok(());
        }
        if let Some(o) = &self.overlay {
            if self.state.config.display_placement != DisplayPlacement::Taskbar {
                self.state.overlay_pos = Some(o.position().map_err(err)?);
            }
        }
        save_state(&self.paths.config_path, &self.state)
    }
    fn apply(&mut self, mut next: PersistedState) -> AppResult<()> {
        next.config = next.config.sanitized();
        let before = self.state.clone();
        let startup_changed = next.config.start_on_login != before.config.start_on_login;
        let exe = if startup_changed {
            Some(std::env::current_exe().map_err(err)?)
        } else {
            None
        };
        if let Some(exe) = &exe {
            native::autostart::set_enabled(next.config.start_on_login, exe)?;
        }
        let providers_changed = next.config.codex != before.config.codex
            || next.config.antigravity != before.config.antigravity;
        self.state = next;
        if let Err(e) = self.setup_overlay().and_then(|()| self.persist()) {
            self.state = before;
            if let Some(exe) = &exe {
                native::autostart::set_enabled(self.state.config.start_on_login, exe)?;
            }
            self.setup_overlay()?;
            return Err(e);
        }
        if providers_changed {
            *self.workers.settings.lock().unwrap() = self.state.config.clone();
            self.workers.gate.interrupt(|c| c.invalidate());
            self.workers
                .tx
                .send(Work::Refresh { full: false })
                .map_err(err)?;
        }
        self.tray_status()?;
        self.read_detail(None)
    }
    fn read_detail(&self, format: Option<ExportFormat>) -> AppResult<()> {
        if self.ready {
            let flag = self.workers.gate.sql_cancel.lock().unwrap().clone();
            self.workers
                .reads
                .send(Read::Detail(self.view_revision, self.filter, format, flag))
                .map_err(err)?;
        }
        Ok(())
    }
    fn push(&self, mut d: DashboardSnapshotDto) -> AppResult<()> {
        if let Some(host) = self.detail.as_ref().filter(|_| self.ready) {
            for card in &mut d.providers {
                let enabled = match card.provider {
                    ProviderId::Codex => self.state.config.codex.enabled,
                    ProviderId::Antigravity => self.state.config.antigravity.enabled,
                };
                if !enabled {
                    card.availability = ProviderAvailability::Disabled;
                }
            }
            host.push_snapshot(&d, &self.state.config).map_err(err)?;
        }
        Ok(())
    }
    fn reap(&mut self) -> AppResult<()> {
        if self.detail.as_ref().is_some_and(|host| host.is_closed()) {
            let closed = self.detail.take().unwrap();
            self.ready = false;
            self.view_revision += 1;
            if let Some(e) = closed.take_close_error() {
                return Err(err(e));
            }
        }
        Ok(())
    }
    fn open_detail(&mut self) -> AppResult<()> {
        self.reap()?;
        if self.detail.is_some() {
            let hwnd = detail_hwnd()?;
            unsafe {
                if IsIconic(hwnd).as_bool() {
                    let _ = ShowWindow(hwnd, SW_RESTORE);
                }
                let _ = SetForegroundWindow(hwnd);
            }
            return Ok(());
        }
        self.detail_session += 1;
        self.view_revision += 1;
        self.filter = Filter::default();
        self.ready = false;
        let id = self.detail_session;
        let ui = self.ui.clone();
        match DetailWindowHost::open(&self.paths.webview_udf_dir, move |cmd| {
            ui.post(Event::Web(id, cmd))
        }) {
            Ok(host) => self.detail = Some(host),
            Err(DetailHostError::AlreadyClosed) => {}
            Err(DetailHostError::RuntimeMissing(_)) => {
                let text = if self.state.config.language == Language::ZhCn {
                    "详情面板需要 Microsoft Edge WebView2 Runtime。请从微软官方网站安装运行时，然后重新打开详情。托盘和悬浮窗仍可使用。"
                } else {
                    "Install the Microsoft Edge WebView2 Runtime from Microsoft's official website, then reopen Details. The tray and overlay remain available."
                };
                unsafe {
                    MessageBoxW(
                        self.hwnd,
                        &HSTRING::from(text),
                        w!("TokenPulse"),
                        MB_OK | MB_ICONINFORMATION,
                    );
                }
            }
            Err(e) => return Err(err(e)),
        }
        Ok(())
    }
    fn command(&mut self, command: TrayCommand) -> AppResult<bool> {
        let mut next = self.state.clone();
        match command {
            TrayCommand::Exit => return Ok(false),
            TrayCommand::OpenDetail => {
                self.open_detail()?;
                return Ok(true);
            }
            TrayCommand::RefreshNow => {
                self.workers
                    .tx
                    .send(Work::Refresh { full: false })
                    .map_err(err)?;
                return Ok(true);
            }
            TrayCommand::TogglePause => {
                self.paused = !self.paused;
                if self.paused {
                    self.workers.gate.interrupt(|c| {
                        c.pause(PauseReason::UserPaused);
                    });
                } else if self.workers.gate.ctl.resume(PauseReason::UserPaused)
                    == ResumeAction::FullReconcile
                {
                    self.workers
                        .tx
                        .send(Work::Refresh { full: true })
                        .map_err(err)?;
                }
                self.tray_status()?;
                return Ok(true);
            }
            TrayCommand::ToggleOverlay => next.overlay_visible = !next.overlay_visible,
            TrayCommand::SetPlacement(placement) => {
                next.config.display_placement = placement;
                next.overlay_visible = true;
                if placement == DisplayPlacement::DesktopWidget {
                    next.overlay_compact = false;
                }
            }
            TrayCommand::ToggleOverlayMode => next.overlay_compact = !next.overlay_compact,
            TrayCommand::ToggleClickThrough => {
                next.config.click_through = !next.config.click_through
            }
            TrayCommand::ToggleAutostart => {
                next.config.start_on_login = !next.config.start_on_login
            }
            TrayCommand::ToggleTheme => {
                next.config.theme = match next.config.theme {
                    Theme::Dark => Theme::Light,
                    Theme::Light => Theme::Dark,
                }
            }
            TrayCommand::ToggleLanguage => {
                next.config.language = match next.config.language {
                    Language::ZhCn => Language::EnUs,
                    Language::EnUs => Language::ZhCn,
                }
            }
        }
        self.apply(next)?;
        Ok(true)
    }
    fn menu_flags(&self) -> TrayMenuState {
        TrayMenuState {
            overlay_visible: self.state.overlay_visible,
            placement: self.state.config.display_placement,
            click_through: self.state.config.click_through,
            paused: self.paused,
            autostart: self.state.config.start_on_login,
        }
    }
    fn tray_click(&mut self, lp: isize) -> AppResult<bool> {
        if (lp >> 16) as u16 != TRAY_ID {
            return Ok(true);
        }
        let flags = self.menu_flags();
        let cmd = match lp as u16 as u32 {
            // NOTIFYICON_VERSION_4 delivers NIN_SELECT and NIN_KEYSELECT.
            NIN_SELECT | 0x401 | WM_LBUTTONUP | WM_LBUTTONDBLCLK => Some(TrayCommand::OpenDetail),
            WM_CONTEXTMENU => match &self.tray {
                Some(tray) => tray.handle_callback(LPARAM(lp), flags).map_err(err)?,
                None => None,
            },
            WM_RBUTTONUP => {
                let mut point = POINT::default();
                unsafe {
                    GetCursorPos(&mut point).map_err(err)?;
                }
                match &self.tray {
                    Some(tray) => tray.popup(point, flags).map_err(err)?,
                    None => None,
                }
            }
            _ => None,
        };
        if let Some(cmd) = cmd {
            return self.command(cmd);
        }
        Ok(true)
    }
    fn overlay_action(&mut self, action: usize) -> AppResult<bool> {
        match action {
            OVERLAY_ACTION_OPEN_DETAIL => {
                self.open_detail()?;
            }
            OVERLAY_ACTION_CONTEXT_MENU => {
                let mut point = POINT::default();
                unsafe {
                    GetCursorPos(&mut point).map_err(err)?;
                }
                let flags = self.menu_flags();
                let cmd = match &self.tray {
                    Some(tray) => tray.popup(point, flags).map_err(err)?,
                    None => None,
                };
                if let Some(cmd) = cmd {
                    return self.command(cmd);
                }
            }
            OVERLAY_ACTION_MODE_TOGGLED => {
                self.state.overlay_compact = !self.state.overlay_compact;
                self.persist()?;
            }
            _ => {}
        }
        Ok(true)
    }
    fn web(&mut self, cmd: HostCommand) -> AppResult<()> {
        match cmd {
            HostCommand::Ready => {
                self.ready = true;
                if let Some(d) = &self.latest {
                    self.push(d.clone())?;
                }
                self.read_detail(None)?;
            }
            HostCommand::Refresh => {
                self.command(TrayCommand::RefreshNow)?;
                self.read_detail(None)?;
            }
            HostCommand::Filter { provider, range } => {
                self.filter = Filter::new(&provider, &range)?;
                self.view_revision += 1;
                self.read_detail(None)?;
            }
            HostCommand::UpdateConfig(config) => {
                let mut next = self.state.clone();
                let placement_changed =
                    config.display_placement != next.config.display_placement;
                next.config = config;
                if placement_changed {
                    next.overlay_visible = true;
                    if next.config.display_placement == DisplayPlacement::DesktopWidget {
                        next.overlay_compact = false;
                    }
                }
                self.apply(next)?;
            }
            HostCommand::ToggleOverlay => {
                self.command(TrayCommand::ToggleOverlay)?;
            }
            HostCommand::ExportJson | HostCommand::ExportCsv => {
                let format = if matches!(cmd, HostCommand::ExportJson) {
                    ExportFormat::Json
                } else {
                    ExportFormat::Csv
                };
                self.read_detail(Some(format))?;
            }
        }
        Ok(())
    }
    fn event(&mut self, event: Event) -> AppResult<bool> {
        match event {
            Event::Cached(frame) => {
                self.frame = Some(*frame);
                self.setup_overlay()?;
                self.tray_status()?;
                self.workers
                    .tx
                    .send(Work::Refresh { full: false })
                    .map_err(err)?;
            }
            Event::Synced(token, pair) if !token.is_cancelled() => {
                let (frame, dashboard) = *pair;
                self.frame = Some(frame);
                self.paint_overlay()?;
                self.tray_status()?;
                if self.ready {
                    if self.filter.provider.is_none() && self.filter.days <= 1 {
                        self.push(dashboard.clone())?;
                    } else {
                        self.read_detail(None)?;
                    }
                }
                self.latest = Some(dashboard);
            }
            Event::Detail(id, flag, d)
                if id == self.view_revision && !flag.load(Ordering::Acquire) =>
            {
                self.push(*d)?
            }
            Event::Export(id, flag, format, range)
                if id == self.view_revision && self.ready && !flag.load(Ordering::Acquire) =>
            {
                if let Some(name) = save_export(detail_hwnd()?, &range, format)? {
                    self.reap()?;
                    if let Some(host) = &self.detail {
                        host.push_toast(&name).map_err(err)?;
                    }
                }
            }
            Event::Web(id, cmd) if id == self.detail_session && self.detail.is_some() => {
                self.web(cmd)?
            }
            Event::Tray(lp) if !self.bench => return self.tray_click(lp),
            Event::OverlayAction(action) if !self.bench => return self.overlay_action(action),
            Event::Open if !self.bench => self.open_detail()?,
            Event::Power => self.tray_status()?,
            Event::Exit => return Ok(false),
            Event::Error(e) | Event::Fatal(e) => return Err(e),
            _ => {}
        }
        Ok(true)
    }
    fn report_error(&self, e: &str) {
        eprintln!("TokenPulse: {e}");
        if let Some(detail) = self.detail.as_ref().filter(|_| self.ready) {
            let message = if self.state.config.language == Language::ZhCn {
                "操作未完成，请重试。"
            } else {
                "Operation failed. Please retry."
            };
            let _ = detail.push_toast(message);
        }
    }
}

fn bench_sample(
    app: &Desktop,
    startup: Duration,
    elapsed: Duration,
) -> AppResult<serde_json::Value> {
    let mut memory = PROCESS_MEMORY_COUNTERS_EX::default();
    let size = std::mem::size_of_val(&memory) as u32;
    memory.cb = size;
    unsafe {
        GetProcessMemoryInfo(
            GetCurrentProcess(),
            (&mut memory as *mut PROCESS_MEMORY_COUNTERS_EX).cast(),
            size,
        )
        .map_err(err)?;
    }
    let tray_id = NOTIFYICONIDENTIFIER {
        cbSize: std::mem::size_of::<NOTIFYICONIDENTIFIER>() as u32,
        hWnd: app.hwnd,
        uID: u32::from(TRAY_ID),
        ..Default::default()
    };
    let tray_active = unsafe { Shell_NotifyIconGetRect(&tray_id) }.is_ok();
    Ok(serde_json::json!({
        "startup_ms": startup.as_secs_f64() * 1000.0,
        "duration_ms": elapsed.as_millis() as u64,
        "private_bytes_mb": memory.PrivateUsage as f64 / 1_048_576.0,
        "working_set_mb": memory.WorkingSetSize as f64 / 1_048_576.0,
        "overlay_present_commits": app.overlay.as_ref().map_or(0, OverlayWindow::present_commit_count),
        "tray_active": tray_active,
        "overlay_active": app.overlay.as_ref().is_some_and(OverlayWindow::is_visible),
    }))
}

fn pump(
    app: &mut Desktop,
    events: Receiver<Event>,
    started: Instant,
    duration: Option<Duration>,
) -> AppResult<Option<serde_json::Value>> {
    let mut first_frame = None;
    let mut timer_armed = false;
    loop {
        if let Err(e) = app.reap() {
            app.report_error(&e);
        }
        if let Some(e) = app
            .overlay
            .as_ref()
            .and_then(OverlayWindow::take_event_error)
        {
            return Err(err(e));
        }
        for event in events.try_iter() {
            let initial = matches!(&event, Event::Cached(_));
            let fatal = initial || matches!(&event, Event::Fatal(_)) || app.bench;
            match app.event(event) {
                Ok(true) => {}
                Ok(false) => {
                    return if app.bench {
                        Err("Benchmark interrupted".into())
                    } else {
                        Ok(None)
                    }
                }
                Err(e) if fatal => return Err(e),
                Err(e) => app.report_error(&e),
            }
            if initial {
                first_frame = Some(Instant::now());
            }
        }
        if let (Some(first), Some(target)) = (first_frame, duration) {
            let elapsed = first.elapsed();
            if elapsed >= target {
                return bench_sample(app, first.duration_since(started), elapsed).map(Some);
            }
            if !timer_armed {
                let tick = (target - elapsed).as_millis().clamp(1, 0x7fff_ffff) as u32;
                if unsafe { SetTimer(app.hwnd, BENCH_TIMER, tick, None) } == 0 {
                    return Err(err(Error::from_win32()));
                }
                timer_armed = true;
            }
        }
        let mut msg = MSG::default();
        unsafe {
            match GetMessageW(&mut msg, None, 0, 0).0 {
                -1 => return Err(err(Error::from_win32())),
                0 if app.bench => return Err("Benchmark interrupted".into()),
                0 => return Ok(None),
                _ if msg.hwnd == app.hwnd
                    && msg.message == WM_TIMER
                    && msg.wParam.0 == BENCH_TIMER =>
                {
                    let _ = KillTimer(app.hwnd, BENCH_TIMER);
                    timer_armed = false;
                }
                _ => {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        }
    }
}

fn run(bench: Option<(bool, Duration)>) -> AppResult<()> {
    let started = Instant::now();
    native::enable_per_monitor_dpi_v2().map_err(err)?;
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED)
            .ok()
            .map_err(err)?;
    }
    let _apartment = Apartment;
    let _guard = if bench.is_none() {
        match InstanceGuard::acquire()? {
            InstanceGuard::AlreadyRunning => return Ok(()),
            primary => Some(primary),
        }
    } else {
        None
    };
    let host = HostWindow::create()?;
    let (sender, events) = mpsc::channel();
    let ui = Mailbox {
        hwnd: host.hwnd.0 as usize,
        sender,
    };
    let gate = Arc::new(Gate::default());
    let (tx, rx) = mpsc::channel();
    ROUTE.with(|r| {
        r.replace(Some(Route {
            ui: ui.clone(),
            gate: gate.clone(),
            work: tx.clone(),
            activate: if bench.is_none() {
                activate_message_id()
            } else {
                0
            },
        }))
    });
    let paths = AppPaths::discover()?;
    let mut state = load_state(&paths.config_path);
    if let Some((tray_only, _)) = bench {
        state.overlay_visible = !tray_only;
    } else {
        state.config.start_on_login = native::autostart::is_enabled()?;
    }
    let tray = TrayHost::new(
        host.hwnd,
        TRAY_ID,
        TrayIconState::Offline,
        state.config.language,
    )
    .ok();
    let workers = Workers::start(
        paths.db_path.clone(),
        state.config.clone(),
        ui.clone(),
        gate,
        tx,
        rx,
    )?;
    let mut app = Desktop {
        hwnd: host.hwnd,
        paths,
        state,
        tray,
        overlay: None,
        detail: None,
        detail_session: 0,
        view_revision: 0,
        ready: false,
        filter: Filter::default(),
        frame: None,
        latest: None,
        paused: false,
        bench: bench.is_some(),
        ui,
        workers,
    };
    let result = pump(
        &mut app,
        events,
        started,
        bench.map(|(_, duration)| duration),
    );
    if bench.is_some() {
        unsafe {
            let _ = KillTimer(host.hwnd, BENCH_TIMER);
        }
    }
    // Always join before destroying the HWND that receives cross-thread wakeups.
    let stopped = app.workers.stop();
    let closed = app
        .detail
        .take()
        .map(|d| d.close().map_err(err))
        .transpose();
    let saved = if result.is_ok() {
        app.persist()
    } else {
        Ok(())
    };
    drop(app);
    drop(host);
    let sample = result?;
    stopped?;
    closed?;
    saved?;
    if let Some(sample) = sample {
        println!("{sample}");
    }
    Ok(())
}

pub fn run_desktop() -> Result<(), String> {
    run(None)
}
pub fn run_bench(tray_only: bool, duration_ms: u64) -> Result<(), String> {
    run(Some((tray_only, Duration::from_millis(duration_ms))))
}

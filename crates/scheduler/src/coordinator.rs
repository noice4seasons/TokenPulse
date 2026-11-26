use crate::debounce::{DebounceConfig, DebounceQueue, DirtyKey};
use crate::power::{CancelToken, CancellationController, PauseReason, ResumeAction};
use crate::watcher::{WatchEvent, WatchRoot};
use std::collections::BTreeMap;
use std::io;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ScanKind {
    Incremental,
    FullReconcile,
}

enum Message<K> {
    Watch(WatchEvent),
    Finished(K),
    Reconcile,
    Stop,
}

impl<K> From<WatchEvent> for Message<K> {
    fn from(event: WatchEvent) -> Self {
        Self::Watch(event)
    }
}

/// A source stays occupied until its worker releases this move-only guard.
/// Release also wakes an idle driver, including on worker unwind.
pub struct InFlightGuard<K> {
    key: Option<K>,
    sender: Sender<Message<K>>,
}

impl<K> Drop for InFlightGuard<K> {
    fn drop(&mut self) {
        if let Some(key) = self.key.take() {
            let _ = self.sender.send(Message::Finished(key));
        }
    }
}

/// The driver's single owner serializes acquire, hint and completion.
/// A present entry is running. Its value is the strongest requested rerun.
pub struct SourceRunGate<K> {
    running: BTreeMap<K, Option<ScanKind>>,
}

impl<K: Ord + Clone> SourceRunGate<K> {
    pub fn is_running(&self, source: &K) -> bool {
        self.running.contains_key(source)
    }
    pub fn rerun_needed(&self, source: &K) -> bool {
        matches!(self.running.get(source), Some(Some(_)))
    }
    fn merge_if_running(&mut self, source: &K, kind: ScanKind) -> bool {
        if let Some(rerun_needed) = self.running.get_mut(source) {
            *rerun_needed = Some(rerun_needed.unwrap_or(kind).max(kind));
            true
        } else {
            false
        }
    }

    fn acquire(
        &mut self,
        source: &K,
        kind: ScanKind,
        sender: &Sender<Message<K>>,
    ) -> Option<InFlightGuard<K>> {
        if self.merge_if_running(source, kind) {
            return None;
        }
        self.running.insert(source.clone(), None);
        Some(InFlightGuard {
            key: Some(source.clone()),
            sender: sender.clone(),
        })
    }

    fn finish(&mut self, source: &K) -> Option<ScanKind> {
        self.running.remove(source).flatten()
    }
}

pub struct ScanRequest<K> {
    pub source_id: K,
    pub roots: Vec<WatchRoot>,
    pub kind: ScanKind,
    pub token: CancelToken,
    pub guard: InFlightGuard<K>,
}

/// Hand the entire request to a scan worker, retaining its guard through
/// publication. Use token.publish_if_current for the final short commit.
pub trait ScanExecutor<K> {
    fn start(&mut self, request: ScanRequest<K>);
}

#[derive(Clone)]
pub struct SchedulerHandle<K> {
    sender: Sender<Message<K>>,
    control: CancellationController,
}

impl<K> SchedulerHandle<K> {
    pub fn watch_event(&self, event: WatchEvent) {
        let _ = self.sender.send(Message::Watch(event));
    }
    pub fn pause(&self, reason: PauseReason) -> CancelToken {
        self.control.pause(reason)
    }
    pub fn disable(&self) -> CancelToken {
        self.control.disable()
    }
    pub fn resume(&self, reason: PauseReason) -> ResumeAction {
        self.forward_resume(self.control.resume(reason))
    }
    pub fn set_enabled(&self, enabled: bool) -> ResumeAction {
        self.forward_resume(self.control.set_enabled(enabled))
    }
    fn forward_resume(&self, action: ResumeAction) -> ResumeAction {
        if action == ResumeAction::FullReconcile {
            let _ = self.sender.send(Message::Reconcile);
        }
        action
    }
    pub fn stop(&self) {
        self.control.set_enabled(false);
        let _ = self.sender.send(Message::Stop);
    }
}

pub struct SchedulerDriver<K, E> {
    sources: [K; 3],
    executor: E,
    queue: DebounceQueue<DirtyKey<K>>,
    gate: SourceRunGate<K>,
    ready: BTreeMap<K, ScanKind>,
    handle: SchedulerHandle<K>,
    receiver: Receiver<Message<K>>,
    stopped: bool,
    #[cfg(windows)]
    watcher: Option<crate::watcher::DirectoryWatcher<Message<K>>>,
}

impl<K: Ord + Clone + Send + 'static, E: ScanExecutor<K>> SchedulerDriver<K, E> {
    /// Entries correspond to WatchRoot::ALL. Repeating a stable source ID
    /// groups those roots into one scan and one in-flight slot.
    pub fn new(sources: [K; 3], config: DebounceConfig, executor: E, now: Instant) -> Self {
        let (sender, receiver) = mpsc::channel();
        let mut queue = DebounceQueue::new(config);
        queue.reconcile_all(now);
        Self {
            sources,
            executor,
            queue,
            gate: SourceRunGate {
                running: BTreeMap::new(),
            },
            ready: BTreeMap::new(),
            handle: SchedulerHandle {
                sender,
                control: Default::default(),
            },
            receiver,
            stopped: false,
            #[cfg(windows)]
            watcher: None,
        }
    }

    pub fn handle(&self) -> SchedulerHandle<K> {
        self.handle.clone()
    }

    #[cfg(windows)]
    pub fn watch(&mut self, home: &std::path::Path) -> io::Result<()> {
        self.watcher = Some(crate::watcher::DirectoryWatcher::start(
            home,
            self.handle.sender.clone(),
        )?);
        self.queue.reconcile_all(Instant::now());
        Ok(())
    }

    pub fn next_deadline(&self) -> Option<Instant> {
        self.queue.next_deadline()
    }

    pub fn run_gate(&self) -> &SourceRunGate<K> {
        &self.gate
    }

    fn schedule(&mut self, source: K, kind: ScanKind) {
        self.ready
            .entry(source)
            .and_modify(|v| *v = (*v).max(kind))
            .or_insert(kind);
    }

    fn receive(&mut self, message: Message<K>, now: Instant) {
        match message {
            Message::Watch(WatchEvent::Dirty(root)) => {
                let key = &self.sources[root.index()];
                if !self.gate.merge_if_running(key, ScanKind::Incremental) {
                    self.queue.mark_dirty(DirtyKey::Source(key.clone()), now);
                }
            }
            Message::Reconcile | Message::Watch(WatchEvent::OverflowReconcile) => {
                self.queue.reconcile_all(now);
            }
            Message::Finished(key) => {
                if let Some(kind) = self.gate.finish(&key) {
                    self.schedule(key, kind);
                }
            }
            Message::Stop => self.stopped = true,
        }
    }

    /// One bounded turn with an injected monotonic time. Running executors
    /// may complete later and signal this driver from any thread.
    pub fn step(&mut self, now: Instant) -> io::Result<()> {
        for _ in 0..128 {
            let Ok(message) = self.receiver.try_recv() else {
                break;
            };
            self.receive(message, now);
        }
        if self.stopped || self.handle.control.active_token().is_none() {
            self.queue.clear();
            self.ready.clear();
            return Ok(());
        }
        #[cfg(windows)]
        let mut refresh = Ok(());
        for dirty in self.queue.drain_ready(now) {
            match dirty {
                DirtyKey::ReconcileAll => {
                    #[cfg(windows)]
                    if let Some(watcher) = &mut self.watcher {
                        refresh = watcher.refresh();
                    }
                    self.queue.clear();
                    for source in self.sources.clone() {
                        self.schedule(source, ScanKind::FullReconcile);
                    }
                }
                DirtyKey::Source(source) => self.schedule(source, ScanKind::Incremental),
            }
        }
        for (source, kind) in std::mem::take(&mut self.ready) {
            let Some(token) = self.handle.control.active_token() else {
                break;
            };
            if let Some(guard) = self.gate.acquire(&source, kind, &self.handle.sender) {
                let roots = WatchRoot::ALL
                    .into_iter()
                    .filter(|root| self.sources[root.index()] == source)
                    .collect();
                self.executor.start(ScanRequest {
                    source_id: source,
                    roots,
                    kind,
                    token,
                    guard,
                });
            }
        }
        #[cfg(windows)]
        refresh?;
        Ok(())
    }

    /// Run on a background scheduler thread. Idle waits have no polling tick.
    pub fn run(mut self) -> io::Result<()> {
        loop {
            self.step(Instant::now())?;
            if self.stopped {
                return Ok(());
            }
            let message = match self.next_deadline() {
                Some(at) => match self
                    .receiver
                    .recv_timeout(at.saturating_duration_since(Instant::now()))
                {
                    Ok(message) => message,
                    Err(RecvTimeoutError::Timeout) => continue,
                    Err(RecvTimeoutError::Disconnected) => return Ok(()),
                },
                None => match self.receiver.recv() {
                    Ok(message) => message,
                    Err(_) => return Ok(()),
                },
            };
            self.receive(message, Instant::now());
        }
    }
}

impl<K, E> Drop for SchedulerDriver<K, E> {
    fn drop(&mut self) {
        self.handle.control.set_enabled(false);
    }
}

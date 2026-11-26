use std::collections::BTreeMap;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DebounceConfig {
    pub quiet_window: Duration,
    pub max_wait: Duration,
}

impl Default for DebounceConfig {
    fn default() -> Self {
        Self {
            quiet_window: Duration::from_millis(300),
            max_wait: Duration::from_millis(1500),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum DirtyKey<K> {
    ReconcileAll,
    Source(K),
}

#[derive(Debug)]
struct Window {
    first_dirty: Instant,
    last_dirty: Instant,
    immediate: bool,
}

impl Window {
    fn deadline(&self, config: DebounceConfig) -> Option<Instant> {
        if self.immediate {
            return Some(self.first_dirty);
        }
        self.first_dirty
            .checked_add(config.max_wait)
            .into_iter()
            .chain(self.last_dirty.checked_add(config.quiet_window))
            .min()
    }
}

pub struct DebounceQueue<K> {
    config: DebounceConfig,
    pending: BTreeMap<K, Window>,
}

impl<K: Ord> DebounceQueue<K> {
    pub fn new(config: DebounceConfig) -> Self {
        Self {
            config,
            pending: BTreeMap::new(),
        }
    }

    pub fn mark_dirty(&mut self, key: K, now: Instant) {
        self.pending
            .entry(key)
            .and_modify(|window| {
                window.first_dirty = window.first_dirty.min(now);
                window.last_dirty = window.last_dirty.max(now);
            })
            .or_insert(Window {
                first_dirty: now,
                last_dirty: now,
                immediate: false,
            });
    }

    pub fn next_deadline(&self) -> Option<Instant> {
        self.pending
            .values()
            .filter_map(|w| w.deadline(self.config))
            .min()
    }

    pub fn drain_ready(&mut self, now: Instant) -> Vec<K>
    where
        K: Clone,
    {
        let config = self.config;
        let mut ready = Vec::new();
        self.pending.retain(|key, w| {
            if w.deadline(config).is_some_and(|at| now >= at) {
                ready.push(key.clone());
                false
            } else {
                true
            }
        });
        ready
    }
}

impl<K: Ord> DebounceQueue<DirtyKey<K>> {
    /// A full reconciliation covers every earlier pending source hint.
    pub fn reconcile_all(&mut self, now: Instant) {
        self.pending.clear();
        self.pending.insert(
            DirtyKey::ReconcileAll,
            Window {
                first_dirty: now,
                last_dirty: now,
                immediate: true,
            },
        );
    }
    pub(crate) fn clear(&mut self) {
        self.pending.clear();
    }
}

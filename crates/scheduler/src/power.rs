use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokenpulse_core_domain::config::AppConfig;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum PauseReason {
    UserPaused = 1,
    ScreenLocked = 2,
    SystemSuspended = 4,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResumeAction {
    None,
    FullReconcile,
}

#[derive(Default)]
struct State {
    reasons: u8,
    disabled: bool,
}

impl State {
    fn active(&self) -> bool {
        self.reasons == 0 && !self.disabled
    }
}

#[derive(Clone, Default)]
pub struct CancellationController {
    generation: Arc<AtomicU64>,
    state: Arc<Mutex<State>>,
}

#[derive(Clone)]
pub struct CancelToken {
    target_gen: u64,
    controller: CancellationController,
}

impl CancelToken {
    pub fn target_gen(&self) -> u64 {
        self.target_gen
    }
    pub fn is_cancelled(&self) -> bool {
        self.controller.generation.load(Ordering::Acquire) != self.target_gen
    }

    /// Serialize a short commit with pause/disable. The closure must not
    /// re-enter this controller or perform a blocking scan.
    pub fn publish_if_current<T>(&self, publish: impl FnOnce() -> T) -> Option<T> {
        let state = self.controller.state.lock().unwrap();
        (state.active() && !self.is_cancelled()).then(publish)
    }
}

impl CancellationController {
    fn token(&self) -> CancelToken {
        CancelToken {
            target_gen: self.generation.load(Ordering::Acquire),
            controller: self.clone(),
        }
    }

    fn advance(&self) {
        // All writers hold the same state lock. Readers only need one atomic load.
        let next = self
            .generation
            .load(Ordering::Relaxed)
            .checked_add(1)
            .expect("cancellation generation exhausted");
        self.generation.store(next, Ordering::Release);
    }

    pub fn active_token(&self) -> Option<CancelToken> {
        let state = self.state.lock().unwrap();
        state.active().then(|| self.token())
    }

    /// Returns a token for the generation invalidated by this transition.
    pub fn pause(&self, reason: PauseReason) -> CancelToken {
        let mut state = self.state.lock().unwrap();
        if state.reasons & reason as u8 == 0 {
            state.reasons |= reason as u8;
            self.advance();
        }
        self.invalidated_token()
    }

    // Called under the state lock after at least one cancellation.
    fn invalidated_token(&self) -> CancelToken {
        CancelToken {
            target_gen: self.generation.load(Ordering::Acquire) - 1,
            controller: self.clone(),
        }
    }

    pub fn resume(&self, reason: PauseReason) -> ResumeAction {
        let mut state = self.state.lock().unwrap();
        let was_active = state.active();
        state.reasons &= !(reason as u8);
        Self::resume_action(was_active, &state)
    }

    pub fn set_enabled(&self, enabled: bool) -> ResumeAction {
        if !enabled {
            self.disable();
            return ResumeAction::None;
        }
        let mut state = self.state.lock().unwrap();
        let was_active = state.active();
        state.disabled = false;
        Self::resume_action(was_active, &state)
    }

    pub fn disable(&self) -> CancelToken {
        let mut state = self.state.lock().unwrap();
        if !state.disabled {
            self.advance();
            state.disabled = true;
        }
        self.invalidated_token()
    }

    pub fn invalidate(&self) {
        let _state = self.state.lock().unwrap();
        self.advance();
    }

    fn resume_action(was_active: bool, state: &State) -> ResumeAction {
        if !was_active && state.active() {
            ResumeAction::FullReconcile
        } else {
            ResumeAction::None
        }
    }
}

#[derive(Debug, Clone)]
pub struct BackoffPolicy {
    base: Duration,
    current: Duration,
}

impl Default for BackoffPolicy {
    fn default() -> Self {
        Self::new(AppConfig::default().antigravity.poll_interval_secs)
    }
}

impl BackoffPolicy {
    pub fn new(base_secs: u32) -> Self {
        let base = Duration::from_secs(base_secs.clamp(60, 300).into());
        Self {
            base,
            current: base,
        }
    }

    pub fn interval(&self) -> Duration {
        self.current
    }
    pub fn on_failure(&mut self) -> Duration {
        self.current = (self.current * 2).min(Duration::from_secs(300));
        self.current
    }
    pub fn on_success(&mut self) -> Duration {
        self.current = self.base;
        self.current
    }
}

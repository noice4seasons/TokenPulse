//! Synchronous scheduling for the three Codex log roots. Watch events are
//! wakeup hints. The executor owns the incremental scan and its checkpoints.
pub mod coordinator;
pub mod debounce;
pub mod power;
pub mod watcher;

#[cfg(test)]
mod tests {
    use super::coordinator::*;
    use super::debounce::*;
    use super::power::*;
    use super::watcher::*;
    use std::sync::mpsc::{self, Receiver, Sender};
    use std::time::{Duration, Instant};

    fn ms(value: u64) -> Duration {
        Duration::from_millis(value)
    }

    #[test]
    fn quiet_deadline_moves_only_for_the_dirty_source() {
        let start = Instant::now();
        let mut q = DebounceQueue::new(Default::default());
        q.mark_dirty(1, start);
        q.mark_dirty(2, start);
        q.mark_dirty(2, start + ms(200));
        assert_eq!(q.next_deadline(), Some(start + ms(300)));
        assert!(q.drain_ready(start + ms(299)).is_empty());
        assert_eq!(q.drain_ready(start + ms(300)), [1]);
        assert_eq!(q.next_deadline(), Some(start + ms(500)));
        assert_eq!(q.drain_ready(start + ms(500)), [2]);
        assert_eq!(q.next_deadline(), None);
    }

    #[test]
    fn continuous_writes_flush_at_max_wait_and_start_a_new_window() {
        let start = Instant::now();
        let mut q = DebounceQueue::new(Default::default());
        for tick in 0..=15 {
            let now = start + ms(tick * 100);
            q.mark_dirty("log", now);
            if tick < 15 {
                assert!(q.drain_ready(now).is_empty());
            }
        }
        assert_eq!(q.next_deadline(), Some(start + ms(1500)));
        assert_eq!(q.drain_ready(start + ms(1500)), ["log"]);
        q.mark_dirty("log", start + ms(1600));
        assert_eq!(q.next_deadline(), Some(start + ms(1900)));
    }

    #[test]
    fn lock_and_suspend_cancel_even_across_overlapping_reasons() {
        for reason in [
            PauseReason::ScreenLocked,
            PauseReason::SystemSuspended,
            PauseReason::UserPaused,
        ] {
            let ctl = CancellationController::default();
            let token = ctl.active_token().unwrap();
            assert!(ctl.pause(reason).is_cancelled());
            assert!(token.is_cancelled());
            assert!(ctl.active_token().is_none());
            assert_eq!(ctl.resume(reason), ResumeAction::FullReconcile);
            assert_eq!(ctl.resume(reason), ResumeAction::None);
            assert_eq!(
                token.publish_if_current(|| panic!("stale response")),
                None::<()>
            );
            assert_eq!(
                ctl.active_token().unwrap().publish_if_current(|| 12),
                Some(12)
            );
        }
        let ctl = CancellationController::default();
        ctl.pause(PauseReason::ScreenLocked);
        ctl.pause(PauseReason::UserPaused);
        ctl.pause(PauseReason::SystemSuspended);
        assert!(ctl.disable().is_cancelled());
        for reason in [
            PauseReason::SystemSuspended,
            PauseReason::ScreenLocked,
            PauseReason::UserPaused,
        ] {
            assert_eq!(ctl.resume(reason), ResumeAction::None);
        }
        assert_eq!(ctl.set_enabled(true), ResumeAction::FullReconcile);
        let token = ctl.active_token().unwrap();
        ctl.set_enabled(false);
        assert!(token.is_cancelled());
    }

    struct Recorder(Sender<ScanRequest<u8>>);
    impl ScanExecutor<u8> for Recorder {
        fn start(&mut self, request: ScanRequest<u8>) {
            self.0.send(request).unwrap();
        }
    }
    fn driver(
        sources: [u8; 3],
        now: Instant,
    ) -> (SchedulerDriver<u8, Recorder>, Receiver<ScanRequest<u8>>) {
        let (tx, rx) = mpsc::channel();
        (
            SchedulerDriver::new(sources, Default::default(), Recorder(tx), now),
            rx,
        )
    }

    #[test]
    fn in_flight_dirtiness_coalesces_and_completion_runs_it_immediately() {
        let now = Instant::now();
        let (mut driver, rx) = driver([7; 3], now);
        let h = driver.handle();
        driver.step(now).unwrap();
        let first = rx.try_recv().unwrap();
        assert!(driver.run_gate().is_running(&7));
        assert_eq!(first.kind, ScanKind::FullReconcile);
        assert_eq!(first.roots, WatchRoot::ALL);
        std::thread::scope(|scope| {
            for root in WatchRoot::ALL {
                let handle = &h;
                scope.spawn(move || handle.watch_event(WatchEvent::Dirty(root)));
            }
        });
        driver.step(now).unwrap();
        assert!(rx.try_recv().is_err());
        assert!(driver.run_gate().rerun_needed(&7));
        assert_eq!(driver.next_deadline(), None);
        drop(first);
        driver.step(now).unwrap();
        let second = rx.try_recv().unwrap();
        assert_eq!((second.source_id, second.kind), (7, ScanKind::Incremental));
        drop(second);
        driver.step(now).unwrap();
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn startup_overflow_and_resume_reconcile_all_registered_roots() {
        let now = Instant::now();
        let (mut driver, rx) = driver([1, 2, 3], now);
        let h = driver.handle();
        driver.step(now).unwrap();
        let mut jobs: Vec<_> = rx.try_iter().collect();
        assert_eq!(jobs.len(), 3);
        let held = jobs.pop().unwrap();
        drop(jobs);
        h.pause(PauseReason::SystemSuspended);
        assert!(held.token.is_cancelled());
        h.watch_event(WatchEvent::OverflowReconcile);
        driver.step(now).unwrap();
        assert!(rx.try_recv().is_err());
        assert_eq!(
            h.resume(PauseReason::SystemSuspended),
            ResumeAction::FullReconcile
        );
        driver.step(now).unwrap();
        assert_eq!(
            rx.try_iter().map(|s| s.source_id).collect::<Vec<_>>(),
            [1, 2]
        );
        drop(held);
        driver.step(now).unwrap();
        let third = rx.try_recv().unwrap();
        assert_eq!((third.source_id, third.kind), (3, ScanKind::FullReconcile));
        drop(third);
        h.watch_event(WatchEvent::Dirty(WatchRoot::Sessions));
        driver.step(now).unwrap();
        assert_eq!(driver.next_deadline(), Some(now + ms(300)));
        h.watch_event(WatchEvent::OverflowReconcile);
        driver.step(now).unwrap();
        let batch: Vec<_> = rx.try_iter().map(|s| (s.source_id, s.kind)).collect();
        assert_eq!(
            batch,
            [
                (1, ScanKind::FullReconcile),
                (2, ScanKind::FullReconcile),
                (3, ScanKind::FullReconcile)
            ]
        );
        assert_eq!(driver.next_deadline(), None);
    }

    #[test]
    fn backoff_caps_and_resets_to_configured_baseline() {
        let secs = Duration::from_secs;
        let mut p = BackoffPolicy::default();
        assert_eq!(p.interval(), secs(90));
        assert_eq!(p.on_failure(), secs(180));
        assert_eq!(p.on_failure(), secs(300));
        assert_eq!(p.on_failure(), secs(300));
        assert_eq!(p.on_success(), secs(90));
        assert_eq!(BackoffPolicy::new(20).interval(), secs(60));
        assert_eq!(BackoffPolicy::new(650).interval(), secs(300));
        let mut custom = BackoffPolicy::new(130);
        assert_eq!(custom.on_failure(), secs(260));
        assert_eq!(custom.on_success(), secs(130));
    }

    #[cfg(windows)]
    #[test]
    fn real_recursive_notifications_filter_credentials_refresh_missing_roots_and_join() {
        use std::fs;
        use std::sync::mpsc::RecvTimeoutError;
        let home = tempfile::tempdir().unwrap();
        let sessions = home.path().join(WatchRoot::Sessions.name());
        let nested = sessions.join("2026");
        fs::create_dir_all(&nested).unwrap();
        let (tx, rx) = mpsc::channel::<WatchEvent>();
        let mut watcher = DirectoryWatcher::start(home.path(), tx).unwrap();
        fs::write(nested.join("auth.json"), b"fixture").unwrap();
        fs::write(nested.join("AUTH.JSON"), b"another fixture").unwrap();
        fs::write(nested.join("other"), b"fixture").unwrap();
        assert_eq!(rx.recv_timeout(ms(200)), Err(RecvTimeoutError::Timeout));
        fs::write(nested.join("rollout.jsonl"), b"{}\n").unwrap();
        assert_eq!(
            rx.recv_timeout(ms(3000)).unwrap(),
            WatchEvent::Dirty(WatchRoot::Sessions)
        );
        let archive = home.path().join(WatchRoot::ArchivedSessions.name());
        fs::create_dir(&archive).unwrap();
        fs::create_dir(home.path().join(WatchRoot::SessionCleanupArchive.name())).unwrap();
        watcher.refresh().unwrap();
        // A dotted directory demonstrates that directory identity does not
        // depend on name heuristics or a stat after a rename/delete.
        fs::create_dir(archive.join("history.v1")).unwrap();
        let until = Instant::now() + ms(3000);
        loop {
            let event = rx
                .recv_timeout(until.saturating_duration_since(Instant::now()))
                .unwrap();
            if event == WatchEvent::Dirty(WatchRoot::ArchivedSessions) {
                break;
            }
        }
        let start = Instant::now();
        watcher.stop().unwrap();
        let elapsed = start.elapsed();
        assert!(elapsed < ms(500), "stop took {elapsed:?}");
        eprintln!("joined three directory workers in {elapsed:?}");
        watcher.stop().unwrap();
    }
}

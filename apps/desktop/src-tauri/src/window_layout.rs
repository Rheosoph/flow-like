use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager, RunEvent, Runtime, WindowEvent};
use tauri_plugin_window_state::{DEFAULT_FILENAME, StateFlags};

const PENDING_FILENAME: &str = ".window-layout-pending";
const MIN_SESSION_DURATION: Duration = Duration::from_secs(10);

// A marker survives crashes and quick exits. The next launch discards only the
// saved window geometry, leaving application data and preferences intact.
struct RecoverySession {
    marker: PathBuf,
    ready_at: Option<Instant>,
    main_closed_after: Option<Duration>,
}

impl RecoverySession {
    fn start(config_dir: &Path, reset_requested: bool) -> io::Result<Self> {
        fs::create_dir_all(config_dir)?;
        let marker = config_dir.join(PENDING_FILENAME);
        if reset_requested || marker.try_exists()? {
            remove_if_present(&config_dir.join(DEFAULT_FILENAME))?;
        }
        fs::write(&marker, [])?;
        Ok(Self {
            marker,
            ready_at: None,
            main_closed_after: None,
        })
    }

    fn ready(&mut self, now: Instant) {
        self.ready_at = Some(now);
    }

    fn main_closed(&mut self, now: Instant) {
        self.main_closed_after = Some(self.elapsed(now));
    }

    fn elapsed(&self, now: Instant) -> Duration {
        self.ready_at
            .map(|ready| now.saturating_duration_since(ready))
            .unwrap_or_default()
    }

    fn finish(&self, now: Instant) -> io::Result<()> {
        let duration = self.main_closed_after.unwrap_or_else(|| self.elapsed(now));
        if duration >= MIN_SESSION_DURATION {
            remove_if_present(&self.marker)?;
        }
        Ok(())
    }
}

fn remove_if_present(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

pub(crate) fn register<R: Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    let tracking_enabled = Arc::new(AtomicBool::new(false));
    let setup_tracking = tracking_enabled.clone();

    // Run recovery before window-state loads its cache, and register both
    // plugins before Tauri creates the main window.
    builder
        .plugin(
            tauri::plugin::Builder::<R>::new("window-layout-recovery")
                .setup(move |app, _| {
                    let session = app
                        .path()
                        .app_config_dir()
                        .map_err(io::Error::other)
                        .and_then(|dir| {
                            let reset = std::env::args_os()
                                .any(|arg| arg == "--reset-window-layout");
                            RecoverySession::start(&dir, reset)
                        });
                    let session = match session {
                        Ok(session) => {
                            setup_tracking.store(true, Ordering::Release);
                            Some(session)
                        }
                        Err(error) => {
                            // Failed recovery must still allow a default window to open.
                            tracing::warn!(%error, "Window layout recovery unavailable; using defaults");
                            None
                        }
                    };
                    app.manage(Mutex::new(session));
                    Ok(())
                })
                .build(),
        )
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(
                    StateFlags::SIZE
                        | StateFlags::POSITION
                        | StateFlags::MAXIMIZED
                        | StateFlags::FULLSCREEN,
                )
                .with_filter(move |_| tracking_enabled.load(Ordering::Acquire))
                .build(),
        )
}

// The app callback runs after plugin events, so a clean exit clears the marker
// only after window-state has had the opportunity to save the current layout.
pub(crate) fn on_event<R: Runtime>(app: &AppHandle<R>, event: &RunEvent) {
    let state = app.state::<Mutex<Option<RecoverySession>>>();
    let Ok(mut session) = state.lock() else {
        return;
    };
    let Some(session) = session.as_mut() else {
        return;
    };
    let now = Instant::now();
    match event {
        RunEvent::Ready => session.ready(now),
        RunEvent::WindowEvent {
            label,
            event: WindowEvent::Destroyed,
            ..
        } if label == "main" => session.main_closed(now),
        RunEvent::Exit => {
            if let Err(error) = session.finish(now) {
                tracing::warn!(%error, "Failed to finish window layout recovery session");
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU64;

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            static NEXT_ID: AtomicU64 = AtomicU64::new(0);
            let dir = std::env::temp_dir().join(format!(
                "flow-like-window-layout-{}-{}",
                std::process::id(),
                NEXT_ID.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&dir).unwrap();
            Self(dir)
        }

        fn save_layout(&self) {
            fs::write(self.0.join(DEFAULT_FILENAME), b"saved layout").unwrap();
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn healthy_unchanged_layout_survives_relaunch() {
        let dir = TestDir::new();
        dir.save_layout();
        let mut session = RecoverySession::start(&dir.0, false).unwrap();
        let now = Instant::now();
        session.ready(now);
        session.finish(now + MIN_SESSION_DURATION).unwrap();

        assert!(!dir.0.join(PENDING_FILENAME).exists());
        RecoverySession::start(&dir.0, false).unwrap();
        assert_eq!(
            fs::read(dir.0.join(DEFAULT_FILENAME)).unwrap(),
            b"saved layout"
        );
    }

    #[test]
    fn quick_exit_resets_only_the_window_layout_on_next_launch() {
        let dir = TestDir::new();
        dir.save_layout();
        fs::write(dir.0.join("settings.json"), b"preferences").unwrap();
        let mut session = RecoverySession::start(&dir.0, false).unwrap();
        let now = Instant::now();
        session.ready(now);
        session.finish(now + Duration::from_secs(9)).unwrap();
        // The window-state plugin can still save during shutdown.
        dir.save_layout();

        RecoverySession::start(&dir.0, false).unwrap();
        assert!(!dir.0.join(DEFAULT_FILENAME).exists());
        assert_eq!(
            fs::read(dir.0.join("settings.json")).unwrap(),
            b"preferences"
        );
    }

    #[test]
    fn crash_resets_layout_even_after_a_long_session() {
        let dir = TestDir::new();
        dir.save_layout();
        let mut session = RecoverySession::start(&dir.0, false).unwrap();
        session.ready(Instant::now() - Duration::from_secs(60));
        drop(session);

        RecoverySession::start(&dir.0, false).unwrap();
        assert!(!dir.0.join(DEFAULT_FILENAME).exists());
    }

    #[test]
    fn quick_main_close_is_not_hidden_by_other_windows_remaining_open() {
        let dir = TestDir::new();
        dir.save_layout();
        let mut session = RecoverySession::start(&dir.0, false).unwrap();
        let now = Instant::now();
        session.ready(now);
        session.main_closed(now + Duration::from_secs(2));
        session.finish(now + Duration::from_secs(60)).unwrap();

        RecoverySession::start(&dir.0, false).unwrap();
        assert!(!dir.0.join(DEFAULT_FILENAME).exists());
    }

    #[test]
    fn exit_before_ready_keeps_recovery_pending() {
        let dir = TestDir::new();
        let session = RecoverySession::start(&dir.0, false).unwrap();
        session
            .finish(Instant::now() + Duration::from_secs(60))
            .unwrap();
        assert!(dir.0.join(PENDING_FILENAME).exists());
    }

    #[test]
    fn explicit_reset_discards_layout_without_a_previous_failure() {
        let dir = TestDir::new();
        dir.save_layout();
        RecoverySession::start(&dir.0, true).unwrap();
        assert!(!dir.0.join(DEFAULT_FILENAME).exists());
    }

    #[test]
    fn healthy_recovery_allows_the_new_layout_to_be_saved() {
        let dir = TestDir::new();
        RecoverySession::start(&dir.0, false).unwrap();
        let mut session = RecoverySession::start(&dir.0, false).unwrap();
        let now = Instant::now();
        session.ready(now);
        dir.save_layout();
        session.finish(now + MIN_SESSION_DURATION).unwrap();

        RecoverySession::start(&dir.0, false).unwrap();
        assert!(dir.0.join(DEFAULT_FILENAME).exists());
    }

    #[test]
    fn failed_reset_does_not_report_a_trackable_session() {
        let dir = TestDir::new();
        fs::create_dir(dir.0.join(DEFAULT_FILENAME)).unwrap();
        assert!(RecoverySession::start(&dir.0, true).is_err());
    }
}

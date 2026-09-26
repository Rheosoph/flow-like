//! Release logging: the level filter every platform's subscriber installs,
//! and the size-capped file desktop and Android builds write next to stderr.
//!
//! Without a filter every dependency logged down to TRACE. LanceDB, the IPC
//! bridge and Wasmtime alone wrote several gigabytes per hour into one file.

use tracing_subscriber::{EnvFilter, filter::LevelFilter};

/// Overrides the release log filter with a level (`debug`) or a full
/// EnvFilter directive string (`debug,lance_io=info`).
const LOG_LEVEL_ENV: &str = "FLOW_LIKE_LOG_LEVEL";
/// The conventional tracing variable, read when `FLOW_LIKE_LOG_LEVEL` is unset.
const RUST_LOG_ENV: &str = "RUST_LOG";

const DEFAULT_LEVEL: LevelFilter = LevelFilter::INFO;

/// Dependencies whose DEBUG and TRACE output buried the application's own
/// logs. EnvFilter matches targets by prefix, so `lance` also covers
/// `lance_io`, `lance_encoding` and `lancedb`, `wasmtime` covers
/// `wasmtime_internal_cranelift`, and `aws_` every AWS SDK crate.
const NOISY_TARGETS: [&str; 14] = [
    "aws_",
    "cranelift",
    "datafusion",
    "h2",
    "hyper",
    "lance",
    "object_store",
    "reqwest",
    "rustls",
    "tao",
    "tokio",
    "tower",
    "wasmtime",
    "wry",
];
const NOISY_LEVEL: LevelFilter = LevelFilter::WARN;

/// The filter release builds install in front of every log layer.
#[cfg(not(debug_assertions))]
pub(crate) fn release_filter() -> EnvFilter {
    filter_from_env(|name| std::env::var(name).ok())
}

fn filter_from_env(var: impl Fn(&str) -> Option<String>) -> EnvFilter {
    let overrides = [LOG_LEVEL_ENV, RUST_LOG_ENV]
        .into_iter()
        .filter_map(var)
        .find(|value| !value.trim().is_empty());
    build_filter(overrides.as_deref().unwrap_or_default())
}

/// INFO by default, with the noisy dependencies capped at WARN. A bare level
/// in `overrides` replaces the default, and a directive naming a target
/// replaces that target's cap: `trace,lance_io=debug` keeps the other noisy
/// crates at WARN.
fn build_filter(overrides: &str) -> EnvFilter {
    let parts: Vec<&str> = overrides
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect();
    let default_level = parts
        .iter()
        .rev()
        .find_map(|part| part.parse::<LevelFilter>().ok())
        .unwrap_or(DEFAULT_LEVEL);

    let mut directives = vec![default_level.to_string()];
    // A default at or below WARN already quiets the noisy crates.
    if default_level > NOISY_LEVEL {
        directives.extend(
            NOISY_TARGETS
                .iter()
                .map(|target| format!("{target}={NOISY_LEVEL}")),
        );
    }
    // EnvFilter replaces an earlier directive for the same target with a later one.
    directives.extend(
        parts
            .into_iter()
            .filter(|part| part.parse::<LevelFilter>().is_err())
            .map(str::to_owned),
    );

    EnvFilter::builder().parse_lossy(directives.join(","))
}

#[cfg(not(target_os = "ios"))]
pub(crate) mod log_file {
    use std::{
        fs::{File, OpenOptions},
        io::{self, BufRead, BufReader, Seek, SeekFrom, Write},
        path::{Path, PathBuf},
        sync::{Mutex, MutexGuard, PoisonError},
        time::{Duration, Instant},
    };
    use tracing_subscriber::fmt::MakeWriter;

    /// How long writes are dropped after a failed rotation before it is retried.
    const ROTATION_RETRY: Duration = Duration::from_secs(30);

    /// Log file that moves itself to `<name>.1` once it reaches `max_bytes`,
    /// replacing the previous backup. The file overshoots the cap by at most
    /// one event, so the file and its backup stay bounded however long the app
    /// runs. A file that cannot be moved has its newest lines copied to the
    /// backup and is emptied in place. Only while the file cannot be reopened
    /// are events dropped, and the file says how many once it is back.
    pub(crate) struct RotatingLogFile {
        state: Mutex<State>,
    }

    struct State {
        path: PathBuf,
        backup: PathBuf,
        max_bytes: u64,
        retry_after: Duration,
        /// `None` while a failed rotation drops writes.
        file: Option<File>,
        len: u64,
        /// Set exactly while `file` is `None`.
        failure: Option<Failure>,
    }

    /// A rotation that left no file to write to.
    struct Failure {
        at: Instant,
        error: io::Error,
        dropped: u64,
    }

    /// Holds the file lock for one event, so concurrent events never interleave.
    pub(crate) struct RotatingLogWriter<'a>(MutexGuard<'a, State>);

    impl RotatingLogFile {
        /// Opens `path` for appending, rotating it first when an earlier run
        /// left it full. `None` when the file cannot be opened.
        pub(crate) fn open(path: PathBuf, max_bytes: u64) -> Option<Self> {
            Self::open_with_retry(path, max_bytes, ROTATION_RETRY)
        }

        fn open_with_retry(path: PathBuf, max_bytes: u64, retry_after: Duration) -> Option<Self> {
            let file = open_append(&path).ok()?;
            let backup = with_suffix(&path, ".1");
            let mut state = State {
                len: file.metadata().map_or(0, |meta| meta.len()),
                file: Some(file),
                path,
                backup,
                max_bytes,
                retry_after,
                failure: None,
            };
            state.rotate_if_due();
            state.trim_backup();
            Some(Self {
                state: Mutex::new(state),
            })
        }
    }

    impl State {
        fn rotate_if_due(&mut self) {
            let due = match self.file {
                Some(_) => self.len >= self.max_bytes,
                None => self
                    .failure
                    .as_ref()
                    .is_none_or(|failure| failure.at.elapsed() >= self.retry_after),
            };
            if due {
                self.rotate();
            }
        }

        fn rotate(&mut self) {
            // Close the handle before renaming so Windows can move the file.
            self.file = None;
            let mut notice = None;
            let reopened = match std::fs::rename(&self.path, &self.backup) {
                // Another process holds the file or its backup open without
                // delete sharing (Windows), or the directory is read-only:
                // keep the newest lines in the backup and empty the file.
                Err(error) if error.kind() != io::ErrorKind::NotFound => {
                    let backup = match copy_tail(&self.path, &self.backup, self.max_bytes) {
                        Ok(()) => "copied its newest lines to the backup".to_owned(),
                        Err(copy_error) => format!("could not back it up ({copy_error})"),
                    };
                    notice = Some(format!(
                        "log rotation could not move the log file ({error}); {backup} and emptied it"
                    ));
                    reopen(&self.path, true)
                }
                // Moved, or deleted while running: there is nothing to keep.
                _ => reopen(&self.path, false),
            };
            match reopened {
                Ok(file) => {
                    self.len = file.metadata().map_or(0, |meta| meta.len());
                    self.file = Some(file);
                    if let Some(failure) = self.failure.take() {
                        self.write_notice(&format!(
                            "log rotation failed ({}); {} events dropped",
                            failure.error, failure.dropped
                        ));
                    }
                    if let Some(notice) = notice {
                        self.write_notice(&notice);
                    }
                }
                Err(error) => {
                    let dropped = self.failure.take().map_or(0, |failure| failure.dropped);
                    self.failure = Some(Failure {
                        at: Instant::now(),
                        error,
                        dropped,
                    });
                }
            }
        }

        /// Records a problem with the log itself as a line of its own.
        fn write_notice(&mut self, notice: &str) {
            let Some(file) = self.file.as_mut() else {
                return;
            };
            let line = format!("{notice}\n");
            if file.write_all(line.as_bytes()).is_ok() {
                self.len += line.len() as u64;
            }
        }

        /// Cuts a backup over the cap, left by a build that never rotated
        /// while running, down to its newest lines.
        fn trim_backup(&self) {
            let oversized = std::fs::metadata(&self.backup)
                .is_ok_and(|meta| meta.is_file() && meta.len() > self.max_bytes);
            if !oversized {
                return;
            }
            let trimmed = with_suffix(&self.backup, ".tmp");
            let result = copy_tail(&self.backup, &trimmed, self.max_bytes)
                .and_then(|()| std::fs::rename(&trimmed, &self.backup));
            if result.is_err() {
                let _ = std::fs::remove_file(&trimmed);
            }
        }
    }

    fn open_append(path: &Path) -> io::Result<File> {
        OpenOptions::new().create(true).append(true).open(path)
    }

    /// Opens `path` again after a rotation, recreating its directory if it
    /// was deleted. `truncate` empties a file that could not be moved.
    fn reopen(path: &Path, truncate: bool) -> io::Result<File> {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if truncate {
            OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(path)
        } else {
            open_append(path)
        }
    }

    /// Replaces `to` with the whole lines in the last `max_bytes` of `from`.
    fn copy_tail(from: &Path, to: &Path, max_bytes: u64) -> io::Result<()> {
        let mut reader = BufReader::new(File::open(from)?);
        let start = reader.get_ref().metadata()?.len().saturating_sub(max_bytes);
        if start > 0 {
            // Skip the rest of the line the cut falls into.
            reader.seek(SeekFrom::Start(start - 1))?;
            reader.skip_until(b'\n')?;
        }
        io::copy(&mut reader, &mut File::create(to)?)?;
        Ok(())
    }

    fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
        let mut name = path.as_os_str().to_owned();
        name.push(suffix);
        name.into()
    }

    impl<'a> MakeWriter<'a> for RotatingLogFile {
        type Writer = RotatingLogWriter<'a>;

        fn make_writer(&'a self) -> Self::Writer {
            // The state stays usable after a panic elsewhere held the lock.
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            state.rotate_if_due();
            // The fmt layer asks for one writer per event.
            if let Some(failure) = state.failure.as_mut() {
                failure.dropped += 1;
            }
            RotatingLogWriter(state)
        }
    }

    impl Write for RotatingLogWriter<'_> {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            let state = &mut *self.0;
            match state.file.as_mut() {
                Some(file) => {
                    let written = file.write(buf)?;
                    state.len += written as u64;
                    Ok(written)
                }
                // No file could be reopened: drop the event, counted in
                // `make_writer`, rather than fail the subscriber.
                None => Ok(buf.len()),
            }
        }

        fn flush(&mut self) -> io::Result<()> {
            self.0.file.as_mut().map_or(Ok(()), |file| file.flush())
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn temp_dir() -> PathBuf {
            let dir = std::env::temp_dir()
                .join(format!("flow-like-log-rotation-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            dir
        }

        fn write(log: &RotatingLogFile, bytes: &[u8]) {
            log.make_writer().write_all(bytes).unwrap();
        }

        /// Ten bytes per line.
        fn lines(numbers: std::ops::Range<usize>) -> String {
            numbers.map(|number| format!("line {number:04}\n")).collect()
        }

        #[test]
        fn rotating_log_file_keeps_one_backup_once_full() {
            let dir = temp_dir();
            let path = dir.join("flow-like.log");
            let backup = dir.join("flow-like.log.1");
            let log = RotatingLogFile::open(path.clone(), 100).unwrap();

            write(&log, &[b'a'; 60]);
            write(&log, &[b'b'; 60]);
            assert_eq!(std::fs::metadata(&path).unwrap().len(), 120);
            assert!(!backup.exists());

            write(&log, &[b'c'; 60]);
            assert_eq!(
                std::fs::read(&backup).unwrap(),
                [[b'a'; 60], [b'b'; 60]].concat()
            );
            assert_eq!(std::fs::read(&path).unwrap(), [b'c'; 60]);

            write(&log, &[b'd'; 60]);
            write(&log, &[b'e'; 60]);
            assert_eq!(
                std::fs::read(&backup).unwrap(),
                [[b'c'; 60], [b'd'; 60]].concat()
            );
            assert_eq!(std::fs::read(&path).unwrap(), [b'e'; 60]);
            assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);

            std::fs::remove_dir_all(dir).unwrap();
        }

        #[test]
        fn rotating_log_file_rotates_a_full_file_from_an_earlier_run() {
            let dir = temp_dir();
            let path = dir.join("flow-like.log");
            let backup = dir.join("flow-like.log.1");
            std::fs::write(&path, lines(0..15)).unwrap();
            std::fs::write(&backup, b"older run").unwrap();

            let log = RotatingLogFile::open(path.clone(), 100).unwrap();
            // The backup keeps the newest lines that fit the cap.
            assert_eq!(std::fs::read_to_string(&backup).unwrap(), lines(5..15));
            write(&log, b"this run");
            assert_eq!(std::fs::read(&path).unwrap(), b"this run");

            std::fs::remove_dir_all(dir).unwrap();
        }

        #[test]
        fn rotating_log_file_trims_an_oversized_backup_from_an_earlier_run() {
            let dir = temp_dir();
            let path = dir.join("flow-like.log");
            let backup = dir.join("flow-like.log.1");
            std::fs::write(&path, b"short run\n").unwrap();
            std::fs::write(&backup, lines(0..25)).unwrap();

            let log = RotatingLogFile::open(path.clone(), 95).unwrap();
            // The cut falls inside line 15, so the backup starts at line 16.
            assert_eq!(std::fs::read_to_string(&backup).unwrap(), lines(16..25));
            write(&log, b"this run\n");
            assert_eq!(
                std::fs::read_to_string(&path).unwrap(),
                "short run\nthis run\n"
            );
            assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);

            std::fs::remove_dir_all(dir).unwrap();
        }

        #[test]
        fn rotating_log_file_empties_a_full_file_it_cannot_move() {
            let dir = temp_dir();
            let path = dir.join("flow-like.log");
            let backup = dir.join("flow-like.log.1");
            std::fs::write(&path, lines(0..150)).unwrap();
            // A directory where the backup belongs makes the rename and the copy fail.
            std::fs::create_dir_all(backup.join("blocker")).unwrap();

            let log = RotatingLogFile::open(path.clone(), 1000).unwrap();
            write(&log, b"this run\n");
            let contents = std::fs::read_to_string(&path).unwrap();
            let (notice, rest) = contents.split_once('\n').unwrap();
            assert!(
                notice.starts_with("log rotation could not move the log file ("),
                "{notice}"
            );
            assert_eq!(rest, "this run\n");

            // The next rotation moves the file again once it can.
            std::fs::remove_dir_all(&backup).unwrap();
            write(&log, lines(0..100).as_bytes());
            write(&log, b"next\n");
            assert_eq!(
                std::fs::read_to_string(&backup).unwrap(),
                format!("{contents}{}", lines(0..100))
            );
            assert_eq!(std::fs::read_to_string(&path).unwrap(), "next\n");

            std::fs::remove_dir_all(dir).unwrap();
        }

        /// Unix only: Windows cannot delete the directory of a file held open.
        #[cfg(unix)]
        #[test]
        fn rotating_log_file_reports_the_events_it_dropped_while_it_could_not_reopen() {
            let dir = temp_dir();
            let logs = dir.join("logs");
            std::fs::create_dir_all(&logs).unwrap();
            let path = logs.join("flow-like.log");
            let log = RotatingLogFile::open_with_retry(path.clone(), 100, Duration::ZERO).unwrap();
            write(&log, lines(0..10).as_bytes());

            // A file where the log directory was makes the rename, the copy
            // and every reopen fail.
            std::fs::remove_dir_all(&logs).unwrap();
            std::fs::write(&logs, b"").unwrap();
            write(&log, b"dropped\n");
            write(&log, b"dropped\n");

            // A deleted log directory is created again.
            std::fs::remove_file(&logs).unwrap();
            write(&log, b"kept\n");
            let contents = std::fs::read_to_string(&path).unwrap();
            let (notice, rest) = contents.split_once('\n').unwrap();
            assert!(notice.starts_with("log rotation failed ("), "{notice}");
            assert!(notice.ends_with("); 2 events dropped"), "{notice}");
            assert_eq!(rest, "kept\n");

            std::fs::remove_dir_all(dir).unwrap();
        }

        #[test]
        fn rotating_log_file_keeps_concurrent_events_whole() {
            const LINE: usize = 32;
            let dir = temp_dir();
            let path = dir.join("flow-like.log");
            let log = RotatingLogFile::open(path.clone(), 40 * LINE as u64).unwrap();

            std::thread::scope(|scope| {
                for thread in 0..8 {
                    let log = &log;
                    scope.spawn(move || {
                        for line in 0..100 {
                            write(log, format!("{thread}:{line:<29}\n").as_bytes());
                        }
                    });
                }
            });

            for file in [path.clone(), dir.join("flow-like.log.1")] {
                let contents = std::fs::read_to_string(file).unwrap();
                assert!(contents.len() <= 41 * LINE);
                assert!(contents.lines().all(|line| line.len() == LINE - 1));
            }

            std::fs::remove_dir_all(dir).unwrap();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing::{
        Level, Metadata,
        callsite::{Callsite, Identifier},
        field::FieldSet,
        metadata::Kind,
        subscriber::Interest,
    };
    use tracing_subscriber::{Layer, Registry};

    struct SampleCallsite;
    static SAMPLE_CALLSITE: SampleCallsite = SampleCallsite;

    impl Callsite for SampleCallsite {
        fn set_interest(&self, _: Interest) {}

        fn metadata(&self) -> &Metadata<'_> {
            unreachable!("filters only read the metadata they are given")
        }
    }

    /// Whether `filter` lets an event from `target` at `level` through.
    fn enabled(filter: &EnvFilter, target: &'static str, level: Level) -> bool {
        let metadata = Box::leak(Box::new(Metadata::new(
            "sample",
            target,
            level,
            None,
            None,
            None,
            FieldSet::new(&[], Identifier(&SAMPLE_CALLSITE)),
            Kind::EVENT,
        )));
        <EnvFilter as Layer<Registry>>::register_callsite(filter, metadata).is_always()
    }

    fn filter_with(env: &[(&str, &str)]) -> EnvFilter {
        filter_from_env(|name| {
            env.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.to_string())
        })
    }

    const APP: &str = "flow_like_desktop::event_sink::cron";

    #[test]
    fn release_filter_keeps_app_logs_at_info_and_noisy_crates_at_warn() {
        let filter = filter_with(&[]);

        for target in [
            APP,
            "flow_like",
            "flow_like_types::intercom",
            "tauri::app",
            "tauri_plugin_updater",
            "panic",
        ] {
            assert!(enabled(&filter, target, Level::INFO), "{target} INFO");
            assert!(!enabled(&filter, target, Level::DEBUG), "{target} DEBUG");
        }

        for target in [
            "lance::dataset",
            "lance_io::scheduler",
            "lance_encoding::decoder",
            "lance_file",
            "lance_index",
            "lance_table",
            "lancedb::table",
            "wasmtime::runtime::type_registry",
            "wasmtime_internal_cranelift",
            "cranelift_codegen::machinst",
            "cranelift_frontend",
            "datafusion_physical_plan",
            "hyper::proto",
            "hyper_util::client",
            "h2::codec",
            "reqwest::connect",
            "rustls::client",
            "tao::platform_impl",
            "wry::ipc",
            "tokio::task",
            "tower::buffer",
            "object_store::local",
            "aws_config::imds",
        ] {
            assert!(enabled(&filter, target, Level::WARN), "{target} WARN");
            assert!(!enabled(&filter, target, Level::INFO), "{target} INFO");
            assert!(!enabled(&filter, target, Level::TRACE), "{target} TRACE");
        }
    }

    #[test]
    fn release_filter_honours_log_level_env_before_rust_log() {
        let debug = filter_with(&[("FLOW_LIKE_LOG_LEVEL", "debug")]);
        assert!(enabled(&debug, APP, Level::DEBUG));
        assert!(!enabled(&debug, APP, Level::TRACE));
        assert!(!enabled(&debug, "lance_io::scheduler", Level::INFO));

        let both = filter_with(&[("FLOW_LIKE_LOG_LEVEL", "warn"), ("RUST_LOG", "trace")]);
        assert!(enabled(&both, APP, Level::WARN));
        assert!(!enabled(&both, APP, Level::INFO));

        for env in [
            &[("RUST_LOG", "trace")][..],
            &[("FLOW_LIKE_LOG_LEVEL", " "), ("RUST_LOG", "trace")],
        ] {
            let rust_log = filter_with(env);
            assert!(enabled(&rust_log, APP, Level::TRACE));
            assert!(!enabled(&rust_log, "wry::ipc", Level::INFO));
        }

        let error = filter_with(&[("FLOW_LIKE_LOG_LEVEL", "ERROR")]);
        assert!(!enabled(&error, APP, Level::WARN));
        assert!(!enabled(&error, "lance_io::scheduler", Level::WARN));
        assert!(enabled(&error, "lance_io::scheduler", Level::ERROR));
    }

    #[test]
    fn release_filter_directives_lift_only_the_named_targets() {
        let directives = filter_with(&[("FLOW_LIKE_LOG_LEVEL", "trace, lance_io=debug")]);
        assert!(enabled(&directives, APP, Level::TRACE));
        assert!(enabled(&directives, "lance_io::scheduler", Level::DEBUG));
        assert!(!enabled(&directives, "lance_io::scheduler", Level::TRACE));
        assert!(!enabled(
            &directives,
            "lance_encoding::decoder",
            Level::INFO
        ));

        // Without a bare level everything else stays at the default.
        let scoped = filter_with(&[("FLOW_LIKE_LOG_LEVEL", "flow_like_desktop::event_sink=trace")]);
        assert!(enabled(&scoped, APP, Level::TRACE));
        assert!(enabled(&scoped, "flow_like_catalog", Level::INFO));
        assert!(!enabled(&scoped, "flow_like_catalog", Level::DEBUG));
        assert!(!enabled(&scoped, "wasmtime", Level::INFO));

        // An invalid directive is ignored instead of silencing the log.
        let invalid = filter_with(&[("FLOW_LIKE_LOG_LEVEL", "lance=loud")]);
        assert!(enabled(&invalid, APP, Level::INFO));
        assert!(!enabled(&invalid, "lance_io::scheduler", Level::INFO));
    }
}

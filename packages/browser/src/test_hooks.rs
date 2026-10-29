use crate::browser::ConnectionKind;

pub use crate::launch::process::{BrowserProcess, LaunchedProcess, Profile, spawn};

pub struct TestSetup {
    pub kind: ConnectionKind,
    pub headless: bool,
    pub page_load_timeout: std::time::Duration,
    pub process: Option<BrowserProcess>,
    pub staging: Option<std::path::PathBuf>,
    pub run_owned_setup: bool,
}

#[cfg(unix)]
pub fn adopt_process(
    child: tokio::process::Child,
    profile: Profile,
    staging: Option<std::path::PathBuf>,
) -> BrowserProcess {
    BrowserProcess::adopt(child, profile, staging)
}

static ASSEMBLED: std::sync::OnceLock<fn(&crate::Browser)> = std::sync::OnceLock::new();

/// Calls `observer` with every browser before it attaches its first tab, so a test that only
/// reaches the browser through nodes still sees the events and stderr of a launch that fails.
/// The first observer of a process stays.
pub fn observe_browsers(observer: fn(&crate::Browser)) {
    let _ = ASSEMBLED.set(observer);
}

pub(crate) fn assembled(browser: &crate::Browser) {
    if let Some(observer) = ASSEMBLED.get() {
        observer(browser);
    }
}

/// Reads the stderr tail even after the browser is gone, without keeping its process alive.
pub fn stderr_reader(
    browser: &crate::Browser,
) -> Option<impl Fn() -> String + Send + Sync + use<>> {
    browser
        .inner
        .process
        .as_ref()
        .map(BrowserProcess::stderr_reader)
}

pub fn stderr_tail(browser: &crate::Browser) -> Option<String> {
    browser
        .inner
        .process
        .as_ref()
        .map(BrowserProcess::stderr_tail)
}

pub fn staging_dir(browser: &crate::Browser) -> Option<std::path::PathBuf> {
    let process = browser.inner.process.as_ref()?;
    process.staging_dir().map(std::path::Path::to_path_buf)
}

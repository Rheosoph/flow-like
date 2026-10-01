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

pub mod args;
pub mod cft;
pub mod discovery;
pub(crate) mod process;
pub mod profile;
pub(crate) mod sandbox;
#[cfg(windows)]
pub(crate) mod windows_process;

pub(crate) use process::{BrowserProcess, LaunchedProcess, spawn};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrowserKind {
    Chrome,
    Edge,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flavor {
    Chrome,
    Edge,
    Chromium,
    SnapChromium,
    ChromeForTesting,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecutableSource {
    Installed,
    CachedCft,
    Explicit,
}

#[derive(Clone, Debug)]
pub struct Executable {
    pub path: std::path::PathBuf,
    pub flavor: Flavor,
    pub version: Option<String>,
    pub source: ExecutableSource,
}

#[derive(Clone, Debug)]
pub struct ProxyConfig {
    pub server: String,
    pub bypass: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct LaunchOptions {
    pub kind: BrowserKind,
    pub executable: Option<Executable>,
    pub headless: bool,
    pub window_size: (u32, u32),
    pub user_agent: Option<String>,
    pub user_data_dir: Option<std::path::PathBuf>,
    pub proxy: Option<ProxyConfig>,
    pub locale: Option<String>,
    pub ignore_https_errors: bool,
    pub cache_dir: std::path::PathBuf,
    pub page_load_timeout: std::time::Duration,
    pub launch_timeout: std::time::Duration,
}

const CACHE_DIR_ENV: &str = "FLOW_LIKE_BROWSER_CACHE_DIR";

pub fn default_cache_dir() -> std::path::PathBuf {
    if let Some(dir) = std::env::var_os(CACHE_DIR_ENV).filter(|dir| !dir.is_empty()) {
        return std::path::PathBuf::from(dir);
    }
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("flow-like")
        .join("browsers")
}

pub fn find(kind: BrowserKind, cache_dir: &std::path::Path) -> crate::Result<Executable> {
    let (snap, installed): (Vec<_>, Vec<_>) = discovery::candidates(kind)
        .into_iter()
        .filter(|candidate| discovery::is_executable_file(&candidate.path))
        .partition(|candidate| candidate.flavor == Flavor::SnapChromium);
    if let Some(candidate) = installed.into_iter().next() {
        return Ok(installed_executable(candidate));
    }
    if kind == BrowserKind::Chrome
        && let Some(executable) = cft::find_cached(cache_dir)
    {
        return Ok(executable);
    }
    if let Some(candidate) = snap.into_iter().next() {
        return Ok(installed_executable(candidate));
    }
    Err(crate::BrowserError::Launch {
        message: discovery::missing_browser_message(
            kind,
            &discovery::searched_locations(kind, cache_dir),
            discovery::edge_hint_applies(kind),
        ),
    })
}

fn installed_executable(candidate: discovery::Candidate) -> Executable {
    let version = discovery::executable_version(&candidate.path, candidate.flavor);
    Executable {
        path: candidate.path,
        flavor: candidate.flavor,
        version,
        source: ExecutableSource::Installed,
    }
}

pub(crate) fn browser_name(flavor: Flavor) -> &'static str {
    match flavor {
        Flavor::Chrome => "Google Chrome",
        Flavor::Edge => "Microsoft Edge",
        Flavor::Chromium | Flavor::SnapChromium => "Chromium",
        Flavor::ChromeForTesting => "Chrome for Testing",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_names_follow_the_flavor() {
        assert_eq!(browser_name(Flavor::Chrome), "Google Chrome");
        assert_eq!(browser_name(Flavor::Edge), "Microsoft Edge");
        assert_eq!(browser_name(Flavor::SnapChromium), "Chromium");
        assert_eq!(browser_name(Flavor::ChromeForTesting), "Chrome for Testing");
    }

    #[test]
    fn default_cache_dir_ends_in_the_browsers_folder() {
        if std::env::var_os(CACHE_DIR_ENV).is_none() {
            let dir = default_cache_dir();
            assert!(dir.ends_with("flow-like/browsers"), "{}", dir.display());
        }
    }
}

// Derived from Chromium chrome/test/chromedriver @154.0.8037.92, Copyright The Chromium Authors, BSD-3-Clause; modified by Rheosoph GmbH. See NOTICE.
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::BrowserError;
use crate::launch::discovery::{executable_version, major_version};
use crate::launch::{Executable, Flavor, browser_name};

const PREFERENCES_TEMPLATE: &str = include_str!("../templates/preferences.txt");
const LOCAL_STATE_TEMPLATE: &str = include_str!("../templates/local_state.txt");
pub(crate) const TEMP_PROFILE_PREFIX: &str = "flow-like-browser-";
pub(crate) const STAGING_PREFIX: &str = "flow-like-downloads-";
const DEVTOOLS_ACTIVE_PORT: &str = "DevToolsActivePort";

pub fn ensure_profile_available(
    dir: &std::path::Path,
    executable: &Executable,
) -> crate::Result<()> {
    let canonical = canonical_dir(dir);
    refuse_everyday_profile(
        dir,
        &canonical,
        executable.flavor,
        &everyday_profile_dirs(executable.flavor),
    )?;
    if executable.flavor == Flavor::SnapChromium {
        refuse_outside_home(&canonical)?;
    }
    if let LockState::InUse(pid) = lock_state(&canonical) {
        return Err(in_use_error(dir, pid));
    }
    match std::fs::remove_file(canonical.join(DEVTOOLS_ACTIVE_PORT)) {
        Err(error) if error.kind() != ErrorKind::NotFound => {
            return Err(BrowserError::io(
                format!(
                    "Deleting the stale {DEVTOOLS_ACTIVE_PORT} in Profile Directory {}",
                    dir.display()
                ),
                &error,
            ));
        }
        _ => {}
    }
    refuse_downgrade(dir, &canonical, executable)
}

pub fn prepare_user_data_dir(dir: &std::path::Path, locale: Option<&str>) -> crate::Result<()> {
    let default_dir = dir.join("Default");
    std::fs::create_dir_all(&default_dir).map_err(|error| {
        BrowserError::io(
            format!("Creating the profile folder {}", default_dir.display()),
            &error,
        )
    })?;
    write_preferences(&default_dir.join("Preferences"), locale)?;
    write_if_missing(&dir.join("Local State"), LOCAL_STATE_TEMPLATE)?;
    write_if_missing(&dir.join("First Run"), "")
}

pub fn create_temp_profile(executable: &Executable) -> crate::Result<std::path::PathBuf> {
    let root = scratch_root(executable.flavor)?;
    let dir = root.join(format!(
        "{TEMP_PROFILE_PREFIX}{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4().simple()
    ));
    create_private_dir(&dir).map_err(|error| {
        BrowserError::io(
            format!("Creating the temporary browser profile {}", dir.display()),
            &error,
        )
    })?;
    Ok(dir)
}

pub fn sweep_stale_profiles(root: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let Some(owner) = entry.file_name().to_str().and_then(sweep_owner_pid) else {
            continue;
        };
        let path = entry.path();
        let is_real_dir = std::fs::symlink_metadata(&path).is_ok_and(|meta| meta.is_dir());
        if !is_real_dir || process_alive(owner) || profile_locked(&path) {
            continue;
        }
        if let Err(error) = std::fs::remove_dir_all(&path) {
            tracing::debug!(
                target: "flow_like_browser::launch",
                "could not remove the stale browser folder {}: {error}",
                path.display()
            );
        }
    }
}

pub fn default_user_data_dir(flavor: Flavor) -> Option<std::path::PathBuf> {
    everyday_profile_dirs(flavor).into_iter().next()
}

pub(crate) fn in_use_error(dir: &Path, pid: Option<u32>) -> BrowserError {
    let owner = match pid {
        Some(pid) => format!("browser process {pid}"),
        None => "another browser process".to_owned(),
    };
    BrowserError::Launch {
        message: format!(
            "Profile Directory {} is in use by {owner}; close it or choose another directory",
            dir.display()
        ),
    }
}

pub(crate) fn lock_owner(dir: &Path) -> Option<u32> {
    #[cfg(unix)]
    {
        let target = std::fs::read_link(dir.join("SingletonLock")).ok()?;
        parse_singleton_lock(target.to_str()?).map(|(_, pid)| pid)
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
        None
    }
}

pub(crate) fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new().mode(0o700).create(dir)
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir(dir)
    }
}

#[cfg(unix)]
pub(crate) fn process_alive(pid: u32) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    if pid <= 0 {
        return false;
    }
    // SAFETY: signal 0 performs only the existence and permission check.
    let result = unsafe { libc::kill(pid, 0) };
    result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(windows)]
pub(crate) fn process_alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_ACCESS_DENIED, GetLastError, STILL_ACTIVE,
    };
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    // SAFETY: OpenProcess has no memory preconditions; the handle is closed below.
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        // SAFETY: reads the calling thread's last error.
        return unsafe { GetLastError() } == ERROR_ACCESS_DENIED;
    }
    let mut code = 0u32;
    // SAFETY: handle is a live process handle and code is a valid out pointer.
    let queried = unsafe { GetExitCodeProcess(handle, &mut code) } != 0;
    // SAFETY: handle was opened above and is closed exactly once.
    unsafe { CloseHandle(handle) };
    queried && code == STILL_ACTIVE as u32
}

enum LockState {
    Free,
    InUse(Option<u32>),
}

fn lock_state(dir: &Path) -> LockState {
    #[cfg(unix)]
    {
        let Some(target) = std::fs::read_link(dir.join("SingletonLock")).ok() else {
            return LockState::Free;
        };
        let Some((host, pid)) = target.to_str().and_then(parse_singleton_lock) else {
            return LockState::Free;
        };
        let same_host = local_hostname().is_none_or(|local| local == host);
        if same_host && process_alive(pid) {
            LockState::InUse(Some(pid))
        } else {
            LockState::Free
        }
    }
    #[cfg(windows)]
    {
        if windows_lockfile_held(dir) {
            LockState::InUse(None)
        } else {
            LockState::Free
        }
    }
}

fn profile_locked(dir: &Path) -> bool {
    #[cfg(unix)]
    {
        lock_owner(dir).is_some_and(process_alive)
    }
    #[cfg(windows)]
    {
        windows_lockfile_held(dir)
    }
}

#[cfg(windows)]
fn windows_lockfile_held(dir: &Path) -> bool {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Foundation::ERROR_SHARING_VIOLATION;

    match std::fs::OpenOptions::new()
        .write(true)
        .share_mode(0)
        .open(dir.join("lockfile"))
    {
        Ok(_) => false,
        Err(error) => error.raw_os_error() == Some(ERROR_SHARING_VIOLATION as i32),
    }
}

#[cfg(unix)]
fn local_hostname() -> Option<String> {
    let mut buffer = [0u8; 256];
    // SAFETY: the buffer is writable for its full length.
    let result = unsafe { libc::gethostname(buffer.as_mut_ptr().cast(), buffer.len()) };
    if result != 0 {
        return None;
    }
    let len = buffer
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(buffer.len());
    String::from_utf8(buffer[..len].to_vec()).ok()
}

#[cfg(any(unix, test))]
fn parse_singleton_lock(target: &str) -> Option<(&str, u32)> {
    let (host, pid) = target.rsplit_once('-')?;
    if host.is_empty() {
        return None;
    }
    Some((host, pid.parse().ok()?))
}

fn sweep_owner_pid(name: &str) -> Option<u32> {
    let rest = name
        .strip_prefix(TEMP_PROFILE_PREFIX)
        .or_else(|| name.strip_prefix(STAGING_PREFIX))?;
    let (pid, _) = rest.split_once('-')?;
    pid.parse().ok()
}

fn canonical_dir(dir: &Path) -> PathBuf {
    std::fs::canonicalize(dir)
        .or_else(|_| std::path::absolute(dir))
        .unwrap_or_else(|_| dir.to_path_buf())
}

fn same_dir(left: &Path, right: &Path) -> bool {
    if cfg!(any(target_os = "macos", windows)) {
        left.to_string_lossy().to_lowercase() == right.to_string_lossy().to_lowercase()
    } else {
        left == right
    }
}

fn refuse_everyday_profile(
    dir: &Path,
    canonical: &Path,
    flavor: Flavor,
    everyday: &[PathBuf],
) -> crate::Result<()> {
    let is_everyday = everyday
        .iter()
        .any(|everyday| same_dir(canonical, &canonical_dir(everyday)));
    if !is_everyday {
        return Ok(());
    }
    Err(BrowserError::Launch {
        message: format!(
            "Profile Directory {} is {}'s everyday profile; Chrome 136+ blocks automation there. Use Attach to Browser, or pick another directory.",
            dir.display(),
            browser_name(flavor)
        ),
    })
}

fn refuse_outside_home(canonical: &Path) -> crate::Result<()> {
    let inside_home =
        dirs::home_dir().is_some_and(|home| canonical.starts_with(canonical_dir(&home)));
    if inside_home {
        Ok(())
    } else {
        Err(BrowserError::Launch {
            message: "Chromium from Snap can only use profile directories inside your home folder"
                .to_owned(),
        })
    }
}

fn refuse_downgrade(dir: &Path, canonical: &Path, executable: &Executable) -> crate::Result<()> {
    let Ok(last) = std::fs::read_to_string(canonical.join("Last Version")) else {
        return Ok(());
    };
    let last = last.trim();
    let Some(last_major) = major_version(last) else {
        return Ok(());
    };
    let Some(current) = executable
        .version
        .clone()
        .or_else(|| executable_version(&executable.path, executable.flavor))
    else {
        return Ok(());
    };
    match major_version(&current) {
        Some(current_major) if last_major > current_major => {
            let name = browser_name(executable.flavor);
            Err(BrowserError::Launch {
                message: format!(
                    "Profile Directory {} was last used by {name} {last}; the selected browser is {name} {current}. Update the browser or use another directory.",
                    dir.display()
                ),
            })
        }
        _ => Ok(()),
    }
}

fn write_preferences(path: &Path, locale: Option<&str>) -> crate::Result<()> {
    let (mut preferences, mut changed) = match std::fs::read_to_string(path) {
        Ok(text) => (parse_preferences(path, &text)?, false),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            (parse_preferences(path, PREFERENCES_TEMPLATE)?, true)
        }
        Err(error) => {
            return Err(BrowserError::io(
                format!("Reading the browser preferences {}", path.display()),
                &error,
            ));
        }
    };
    if let Some(locale) = locale.map(str::trim).filter(|locale| !locale.is_empty()) {
        changed |= set_dotted(
            &mut preferences,
            "intl.accept_languages",
            Value::String(locale.to_owned()),
        );
    }
    if !changed {
        return Ok(());
    }
    let bytes = serde_json::to_vec(&preferences).map_err(|error| BrowserError::Launch {
        message: format!(
            "Could not encode the browser preferences {}: {error}",
            path.display()
        ),
    })?;
    std::fs::write(path, bytes).map_err(|error| {
        BrowserError::io(
            format!("Writing the browser preferences {}", path.display()),
            &error,
        )
    })
}

fn parse_preferences(path: &Path, text: &str) -> crate::Result<Value> {
    match serde_json::from_str::<Value>(text) {
        Ok(value) if value.is_object() => Ok(value),
        Ok(_) => Err(corrupt_preferences(
            path,
            "the top level is not a JSON object",
        )),
        Err(error) => Err(corrupt_preferences(path, &error.to_string())),
    }
}

fn corrupt_preferences(path: &Path, reason: &str) -> BrowserError {
    BrowserError::Launch {
        message: format!(
            "The browser preferences file {} could not be read ({reason}); fix or delete it, or choose another Profile Directory",
            path.display()
        ),
    }
}

fn set_dotted(root: &mut Value, dotted: &str, value: Value) -> bool {
    let mut node = root;
    for key in dotted.split('.') {
        if !node.is_object() {
            *node = Value::Null;
        }
        node = &mut node[key];
    }
    if *node == value {
        return false;
    }
    *node = value;
    true
}

fn write_if_missing(path: &Path, contents: &str) -> crate::Result<()> {
    use std::io::Write;
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path);
    let result = match file {
        Ok(mut file) => file.write_all(contents.as_bytes()),
        Err(error) if error.kind() == ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error),
    };
    result.map_err(|error| BrowserError::io(format!("Writing {}", path.display()), &error))
}

pub(crate) fn scratch_root(flavor: Flavor) -> crate::Result<PathBuf> {
    if flavor != Flavor::SnapChromium {
        return Ok(std::env::temp_dir());
    }
    let home = dirs::home_dir().ok_or_else(|| BrowserError::Launch {
        message:
            "Chromium from Snap keeps its profiles in your home folder, but no home folder is set"
                .to_owned(),
    })?;
    let root = snap_root_in(&home);
    std::fs::create_dir_all(&root).map_err(|error| {
        BrowserError::io(
            format!("Creating the Snap Chromium profile root {}", root.display()),
            &error,
        )
    })?;
    Ok(root)
}

fn snap_root_in(home: &Path) -> PathBuf {
    home.join("snap").join("chromium").join("common")
}

#[cfg(not(any(target_os = "macos", windows)))]
pub(crate) fn snap_profile_root() -> Option<PathBuf> {
    dirs::home_dir().map(|home| snap_root_in(&home))
}

fn everyday_profile_dirs(flavor: Flavor) -> Vec<PathBuf> {
    let Some(base) = everyday_base(flavor) else {
        return Vec::new();
    };
    everyday_names(flavor)
        .iter()
        .map(|name| base.join(name))
        .collect()
}

#[cfg(target_os = "macos")]
fn everyday_base(flavor: Flavor) -> Option<PathBuf> {
    (flavor != Flavor::SnapChromium)
        .then(|| dirs::home_dir().map(|home| home.join("Library").join("Application Support")))
        .flatten()
}

#[cfg(target_os = "macos")]
fn everyday_names(flavor: Flavor) -> &'static [&'static str] {
    match flavor {
        Flavor::Chrome => &[
            "Google/Chrome",
            "Google/Chrome Beta",
            "Google/Chrome Dev",
            "Google/Chrome Canary",
        ],
        Flavor::Edge => &[
            "Microsoft Edge",
            "Microsoft Edge Beta",
            "Microsoft Edge Dev",
            "Microsoft Edge Canary",
        ],
        Flavor::Chromium => &["Chromium"],
        Flavor::SnapChromium | Flavor::ChromeForTesting => &[],
    }
}

#[cfg(windows)]
fn everyday_base(flavor: Flavor) -> Option<PathBuf> {
    (flavor != Flavor::SnapChromium)
        .then(|| std::env::var_os("LOCALAPPDATA").filter(|dir| !dir.is_empty()))
        .flatten()
        .map(PathBuf::from)
}

#[cfg(windows)]
fn everyday_names(flavor: Flavor) -> &'static [&'static str] {
    match flavor {
        Flavor::Chrome => &[
            r"Google\Chrome\User Data",
            r"Google\Chrome Beta\User Data",
            r"Google\Chrome Dev\User Data",
            r"Google\Chrome SxS\User Data",
        ],
        Flavor::Edge => &[
            r"Microsoft\Edge\User Data",
            r"Microsoft\Edge Beta\User Data",
            r"Microsoft\Edge Dev\User Data",
            r"Microsoft\Edge SxS\User Data",
        ],
        Flavor::Chromium => &[r"Chromium\User Data"],
        Flavor::SnapChromium | Flavor::ChromeForTesting => &[],
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
fn everyday_base(flavor: Flavor) -> Option<PathBuf> {
    if flavor == Flavor::SnapChromium {
        return snap_profile_root();
    }
    ["CHROME_CONFIG_HOME", "XDG_CONFIG_HOME"]
        .iter()
        .filter_map(std::env::var_os)
        .find(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".config")))
}

#[cfg(not(any(target_os = "macos", windows)))]
fn everyday_names(flavor: Flavor) -> &'static [&'static str] {
    match flavor {
        Flavor::Chrome => &[
            "google-chrome",
            "google-chrome-beta",
            "google-chrome-unstable",
            "google-chrome-canary",
        ],
        Flavor::Edge => &[
            "microsoft-edge",
            "microsoft-edge-beta",
            "microsoft-edge-dev",
            "microsoft-edge-canary",
        ],
        Flavor::Chromium | Flavor::SnapChromium => &["chromium"],
        Flavor::ChromeForTesting => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::launch::ExecutableSource;

    fn executable(flavor: Flavor, version: Option<&str>) -> Executable {
        Executable {
            path: PathBuf::from("/nonexistent/browser"),
            flavor,
            version: version.map(str::to_owned),
            source: ExecutableSource::Explicit,
        }
    }

    fn read_json(path: &Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(path).expect("file exists"))
            .expect("valid json")
    }

    #[test]
    fn a_fresh_profile_gets_the_chromedriver_templates() {
        let root = tempfile::tempdir().expect("tempdir");
        prepare_user_data_dir(root.path(), None).expect("prepared");
        let template: Value = serde_json::from_str(PREFERENCES_TEMPLATE).expect("template");
        assert_eq!(
            read_json(&root.path().join("Default/Preferences")),
            template
        );
        let local_state: Value = serde_json::from_str(LOCAL_STATE_TEMPLATE).expect("template");
        assert_eq!(read_json(&root.path().join("Local State")), local_state);
        assert_eq!(
            std::fs::read(root.path().join("First Run")).expect("first run"),
            b""
        );
    }

    #[test]
    fn locale_is_merged_by_dotted_path_and_unrelated_keys_survive() {
        let root = tempfile::tempdir().expect("tempdir");
        let preferences = root.path().join("Default/Preferences");
        std::fs::create_dir_all(preferences.parent().expect("parent")).expect("mkdir");
        std::fs::write(
            &preferences,
            r#"{"custom":{"kept":1},"intl":{"selected_languages":"fr-FR"}}"#,
        )
        .expect("write");
        std::fs::write(root.path().join("Local State"), r#"{"mine":true}"#).expect("write");
        std::fs::write(root.path().join("First Run"), "keep").expect("write");

        prepare_user_data_dir(root.path(), Some("de-DE")).expect("prepared");
        let merged = read_json(&preferences);
        assert_eq!(merged["custom"]["kept"], 1);
        assert_eq!(merged["intl"]["selected_languages"], "fr-FR");
        assert_eq!(merged["intl"]["accept_languages"], "de-DE");
        assert_eq!(
            read_json(&root.path().join("Local State")),
            serde_json::json!({"mine": true})
        );
        assert_eq!(
            std::fs::read(root.path().join("First Run")).expect("read"),
            b"keep"
        );

        let before = std::fs::read(&preferences).expect("read");
        std::fs::write(
            &preferences,
            format!("{}\n", String::from_utf8_lossy(&before)),
        )
        .expect("rewrite with a trailing newline");
        prepare_user_data_dir(root.path(), Some("de-DE")).expect("prepared again");
        assert!(
            std::fs::read(&preferences).expect("read").ends_with(b"\n"),
            "an unchanged preference file is not rewritten"
        );
    }

    #[test]
    fn corrupt_preferences_are_reported_and_left_untouched() {
        let root = tempfile::tempdir().expect("tempdir");
        let preferences = root.path().join("Default/Preferences");
        std::fs::create_dir_all(preferences.parent().expect("parent")).expect("mkdir");
        std::fs::write(&preferences, "{not json").expect("write");
        let error = prepare_user_data_dir(root.path(), Some("de-DE"))
            .expect_err("corrupt file")
            .to_string();
        assert!(
            error.contains(&preferences.display().to_string()),
            "{error}"
        );
        assert_eq!(std::fs::read(&preferences).expect("read"), b"{not json");
    }

    #[test]
    fn dotted_paths_replace_non_objects_on_the_way() {
        let mut root = serde_json::json!({"intl": "legacy"});
        assert!(set_dotted(
            &mut root,
            "intl.accept_languages",
            Value::from("en-US")
        ));
        assert_eq!(
            root,
            serde_json::json!({"intl": {"accept_languages": "en-US"}})
        );
        assert!(!set_dotted(
            &mut root,
            "intl.accept_languages",
            Value::from("en-US")
        ));
    }

    #[test]
    fn the_everyday_profile_is_refused_before_any_write() {
        let root = tempfile::tempdir().expect("tempdir");
        let everyday = root.path().join("Google/Chrome");
        std::fs::create_dir_all(&everyday).expect("mkdir");
        let error = refuse_everyday_profile(
            &everyday,
            &canonical_dir(&everyday),
            Flavor::Chrome,
            std::slice::from_ref(&everyday),
        )
        .expect_err("refused")
        .to_string();
        assert_eq!(
            error,
            format!(
                "Profile Directory {} is Google Chrome's everyday profile; Chrome 136+ blocks automation there. Use Attach to Browser, or pick another directory.",
                everyday.display()
            )
        );
        let other = root.path().join("automation");
        assert!(
            refuse_everyday_profile(&other, &canonical_dir(&other), Flavor::Chrome, &[everyday])
                .is_ok()
        );
        assert_eq!(default_user_data_dir(Flavor::ChromeForTesting), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_to_the_everyday_profile_is_refused() {
        let root = tempfile::tempdir().expect("tempdir");
        let everyday = root.path().join("Chrome");
        std::fs::create_dir_all(&everyday).expect("mkdir");
        let link = root.path().join("link");
        std::os::unix::fs::symlink(&everyday, &link).expect("symlink");
        assert!(
            refuse_everyday_profile(&link, &canonical_dir(&link), Flavor::Edge, &[everyday])
                .is_err()
        );
    }

    #[test]
    fn singleton_lock_targets_are_parsed() {
        assert_eq!(parse_singleton_lock("Mac-34645"), Some(("Mac", 34645)));
        assert_eq!(
            parse_singleton_lock("build-host-01-12"),
            Some(("build-host-01", 12))
        );
        assert_eq!(parse_singleton_lock("nohyphen"), None);
        assert_eq!(parse_singleton_lock("host-"), None);
        assert_eq!(parse_singleton_lock("-12"), None);
        assert_eq!(parse_singleton_lock("host-12x"), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_live_singleton_lock_blocks_and_a_stale_one_does_not() {
        let root = tempfile::tempdir().expect("tempdir");
        let host = local_hostname().expect("hostname");
        let lock = root.path().join("SingletonLock");
        std::os::unix::fs::symlink(format!("{host}-{}", std::process::id()), &lock)
            .expect("symlink");
        std::fs::write(
            root.path().join(DEVTOOLS_ACTIVE_PORT),
            "1\n/devtools/browser/x",
        )
        .expect("write");
        let error = ensure_profile_available(root.path(), &executable(Flavor::Chrome, None))
            .expect_err("in use")
            .to_string();
        assert_eq!(
            error,
            format!(
                "Profile Directory {} is in use by browser process {}; close it or choose another directory",
                root.path().display(),
                std::process::id()
            )
        );
        assert!(root.path().join(DEVTOOLS_ACTIVE_PORT).exists());

        std::fs::remove_file(&lock).expect("remove");
        std::os::unix::fs::symlink(format!("{host}-{}", dead_pid()), &lock).expect("symlink");
        ensure_profile_available(root.path(), &executable(Flavor::Chrome, None))
            .expect("a dead owner does not block");
        assert!(!root.path().join(DEVTOOLS_ACTIVE_PORT).exists());
    }

    #[test]
    fn a_newer_profile_is_not_downgraded() {
        let root = tempfile::tempdir().expect("tempdir");
        std::fs::write(root.path().join("Last Version"), "156.0.7000.1\n").expect("write");
        let error = ensure_profile_available(
            root.path(),
            &executable(Flavor::ChromeForTesting, Some("154.0.8037.92")),
        )
        .expect_err("downgrade")
        .to_string();
        assert_eq!(
            error,
            format!(
                "Profile Directory {} was last used by Chrome for Testing 156.0.7000.1; the selected browser is Chrome for Testing 154.0.8037.92. Update the browser or use another directory.",
                root.path().display()
            )
        );
        for version in ["156.0.1.1", "157.0.0.1"] {
            ensure_profile_available(root.path(), &executable(Flavor::Chrome, Some(version)))
                .expect("same or newer browser");
        }
        ensure_profile_available(root.path(), &executable(Flavor::Chrome, None))
            .expect("unknown version is not guarded");
    }

    #[test]
    fn temporary_profiles_are_private_and_named_after_this_process() {
        let dir = create_temp_profile(&executable(Flavor::Chrome, None)).expect("created");
        let name = dir
            .file_name()
            .and_then(|name| name.to_str())
            .expect("name");
        assert_eq!(sweep_owner_pid(name), Some(std::process::id()));
        assert!(dir.starts_with(std::env::temp_dir()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&dir)
                .expect("metadata")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o700);
        }
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[cfg(unix)]
    fn dead_pid() -> u32 {
        let mut child = std::process::Command::new("true")
            .spawn()
            .expect("spawn true");
        let pid = child.id();
        child.wait().expect("wait");
        pid
    }

    #[cfg(unix)]
    #[test]
    fn sweep_removes_only_folders_of_dead_owners() {
        let root = tempfile::tempdir().expect("tempdir");
        let dead = dead_pid();
        let live = std::process::id();
        let make = |name: String| {
            let dir = root.path().join(name);
            std::fs::create_dir_all(dir.join("Default")).expect("mkdir");
            dir
        };
        let dead_profile = make(format!("{TEMP_PROFILE_PREFIX}{dead}-a"));
        let dead_staging = make(format!("{STAGING_PREFIX}{dead}-b"));
        let live_profile = make(format!("{TEMP_PROFILE_PREFIX}{live}-c"));
        let locked = make(format!("{TEMP_PROFILE_PREFIX}{dead}-d"));
        std::os::unix::fs::symlink(format!("elsewhere-{live}"), locked.join("SingletonLock"))
            .expect("symlink");
        let unrelated = make(format!("other-{dead}-e"));
        let target = make("target".to_owned());
        std::os::unix::fs::symlink(
            &target,
            root.path().join(format!("{TEMP_PROFILE_PREFIX}{dead}-f")),
        )
        .expect("symlink");

        sweep_stale_profiles(root.path());

        assert!(!dead_profile.exists());
        assert!(!dead_staging.exists());
        assert!(live_profile.exists());
        assert!(locked.exists());
        assert!(unrelated.exists());
        assert!(target.join("Default").exists());
    }

    #[cfg(unix)]
    #[test]
    fn process_liveness() {
        assert!(process_alive(std::process::id()));
        assert!(!process_alive(dead_pid()));
        assert!(!process_alive(0));
        assert!(!process_alive(u32::MAX));
    }
}

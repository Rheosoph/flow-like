// Derived from rustwright src/lib.rs @fca1438, Copyright (c) 2026 Ikonomos Inc (dba Skyvern), MIT; modified by Rheosoph GmbH. See NOTICE.
use std::path::{Path, PathBuf};

use crate::launch::cft::CACHE_SUBDIR;
use crate::launch::{BrowserKind, Flavor};

const SNAP_LAUNCHER: &str = "snap";

pub fn candidates(kind: BrowserKind) -> Vec<Candidate> {
    locations(kind).into_iter().filter_map(resolve).collect()
}

pub struct Candidate {
    pub path: std::path::PathBuf,
    pub flavor: Flavor,
}

pub fn executable_version(path: &std::path::Path, flavor: Flavor) -> Option<String> {
    if flavor == Flavor::ChromeForTesting
        && let Some(version) = cft_install_dir(path).and_then(cft_dir_version)
    {
        return Some(version);
    }
    platform_version(path)
}

pub(crate) fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        metadata.is_file()
    }
}

pub(crate) fn cft_install_dir(path: &Path) -> Option<&Path> {
    path.ancestors().find(|dir| {
        dir.parent()
            .and_then(Path::file_name)
            .is_some_and(|name| name == CACHE_SUBDIR)
    })
}

pub(crate) fn major_version(version: &str) -> Option<u32> {
    version.trim().split('.').next()?.parse().ok()
}

pub(crate) fn searched_locations(kind: BrowserKind, cache_dir: &Path) -> Vec<String> {
    let locations = locations(kind);
    let mut searched: Vec<String> = locations.iter().map(Location::describe).collect();
    if kind == BrowserKind::Chrome {
        let snap_position = locations
            .iter()
            .position(|location| location.flavor() == Flavor::SnapChromium)
            .unwrap_or(searched.len());
        searched.insert(
            snap_position,
            format!(
                "Chrome for Testing in {}",
                cache_dir.join(CACHE_SUBDIR).display()
            ),
        );
    }
    searched
}

pub(crate) fn edge_hint_applies(kind: BrowserKind) -> bool {
    cfg!(windows)
        && kind == BrowserKind::Chrome
        && candidates(BrowserKind::Edge)
            .iter()
            .any(|candidate| is_executable_file(&candidate.path))
}

pub(crate) fn missing_browser_message(
    kind: BrowserKind,
    searched: &[String],
    edge_installed: bool,
) -> String {
    compose_missing_message(
        kind,
        searched,
        edge_installed,
        !cfg!(all(windows, target_arch = "aarch64")),
    )
}

fn compose_missing_message(
    kind: BrowserKind,
    searched: &[String],
    edge_installed: bool,
    cft_available: bool,
) -> String {
    let name = match kind {
        BrowserKind::Chrome => "Chrome",
        BrowserKind::Edge => "Microsoft Edge",
    };
    let mut message = format!(
        "No {name} browser was found. Searched: {}. Install Google Chrome or Microsoft Edge",
        searched.join(", ")
    );
    if cft_available {
        message.push_str(
            ", or download Chrome for Testing (about 200 MB from storage.googleapis.com) in Settings > Automation.",
        );
    } else {
        message.push('.');
    }
    if edge_installed {
        message.push_str(" Microsoft Edge is installed: set Browser Type to Edge.");
    }
    message
}

enum Location {
    File(PathBuf, Flavor),
    #[cfg_attr(any(target_os = "macos", windows), allow(dead_code))]
    OnPath(&'static str, Flavor),
    #[cfg(windows)]
    AppPaths(&'static str, Flavor),
}

impl Location {
    fn flavor(&self) -> Flavor {
        match self {
            Self::File(_, flavor) | Self::OnPath(_, flavor) => *flavor,
            #[cfg(windows)]
            Self::AppPaths(_, flavor) => *flavor,
        }
    }

    fn describe(&self) -> String {
        match self {
            Self::File(path, _) => path.display().to_string(),
            Self::OnPath(name, _) => format!("{name} on PATH"),
            #[cfg(windows)]
            Self::AppPaths(exe, _) => format!("the App Paths registry entry for {exe}"),
        }
    }
}

fn resolve(location: Location) -> Option<Candidate> {
    match location {
        Location::File(path, flavor) => Some(Candidate { path, flavor }),
        Location::OnPath(name, flavor) => resolve_path_hit(which(name)?, flavor),
        #[cfg(windows)]
        Location::AppPaths(exe, flavor) => {
            app_paths_entry(exe).map(|path| Candidate { path, flavor })
        }
    }
}

fn resolve_path_hit(found: PathBuf, flavor: Flavor) -> Option<Candidate> {
    if flavor != Flavor::Chromium {
        return Some(Candidate {
            path: found,
            flavor,
        });
    }
    let resolved = std::fs::canonicalize(&found).ok()?;
    (!is_packaged_chromium(&found, &resolved, &file_head(&resolved))).then_some(Candidate {
        path: resolved,
        flavor,
    })
}

/// snapd links every `/snap/bin/<app>` to its multi-call launcher, which picks the app from
/// argv[0], so a PATH hit is packaged when the hit itself, its target or the launcher name says so.
fn is_packaged_chromium(found: &Path, resolved: &Path, head: &[u8]) -> bool {
    is_snap_or_flatpak(found, &[])
        || is_snap_or_flatpak(resolved, head)
        || resolved
            .file_name()
            .is_some_and(|name| name == SNAP_LAUNCHER)
}

fn which(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(name))
        .find(|path| is_executable_file(path))
}

fn file_head(path: &Path) -> Vec<u8> {
    use std::io::Read;
    let mut head = Vec::new();
    if let Ok(file) = std::fs::File::open(path) {
        let _ = file.take(4096).read_to_end(&mut head);
    }
    head
}

fn is_snap_or_flatpak(executable: &Path, head: &[u8]) -> bool {
    let path = executable.to_string_lossy();
    if path.starts_with("/snap/")
        || path.starts_with("/var/lib/snapd/snap/")
        || path.starts_with("/var/lib/flatpak/")
        || path.contains("/.local/share/flatpak/")
    {
        return true;
    }
    let mentions = |needle: &[u8]| head.windows(needle.len()).any(|window| window == needle);
    head.starts_with(b"#!")
        && (mentions(b"/snap/") || mentions(b"snap run") || mentions(b"flatpak"))
}

#[cfg(target_os = "macos")]
fn locations(kind: BrowserKind) -> Vec<Location> {
    let (names, flavor): (&[&str], Flavor) = match kind {
        BrowserKind::Chrome => (
            &[
                "Google Chrome",
                "Google Chrome Beta",
                "Google Chrome Dev",
                "Google Chrome Canary",
            ],
            Flavor::Chrome,
        ),
        BrowserKind::Edge => (
            &[
                "Microsoft Edge",
                "Microsoft Edge Beta",
                "Microsoft Edge Dev",
                "Microsoft Edge Canary",
            ],
            Flavor::Edge,
        ),
    };
    let mut roots = vec![PathBuf::from("/Applications")];
    roots.extend(dirs::home_dir().map(|home| home.join("Applications")));
    let mut locations: Vec<Location> = roots
        .iter()
        .flat_map(|root| {
            names
                .iter()
                .map(move |name| Location::File(mac_bundle_executable(root, name), flavor))
        })
        .collect();
    if kind == BrowserKind::Chrome {
        locations.push(Location::File(
            mac_bundle_executable(Path::new("/Applications"), "Chromium"),
            Flavor::Chromium,
        ));
    }
    locations
}

#[cfg(target_os = "macos")]
fn mac_bundle_executable(root: &Path, name: &str) -> PathBuf {
    root.join(format!("{name}.app"))
        .join("Contents")
        .join("MacOS")
        .join(name)
}

#[cfg(windows)]
fn locations(kind: BrowserKind) -> Vec<Location> {
    let (channels, exe, flavor) = match kind {
        BrowserKind::Chrome => (
            [
                r"Google\Chrome",
                r"Google\Chrome Beta",
                r"Google\Chrome Dev",
                r"Google\Chrome SxS",
            ],
            "chrome.exe",
            Flavor::Chrome,
        ),
        BrowserKind::Edge => (
            [
                r"Microsoft\Edge",
                r"Microsoft\Edge Beta",
                r"Microsoft\Edge Dev",
                r"Microsoft\Edge SxS",
            ],
            "msedge.exe",
            Flavor::Edge,
        ),
    };
    let roots: Vec<PathBuf> = ["LOCALAPPDATA", "PROGRAMFILES", "PROGRAMFILES(X86)"]
        .iter()
        .filter_map(std::env::var_os)
        .filter(|root| !root.is_empty())
        .map(PathBuf::from)
        .collect();
    let mut locations: Vec<Location> = channels
        .iter()
        .flat_map(|channel| {
            roots.iter().map(move |root| {
                Location::File(root.join(channel).join("Application").join(exe), flavor)
            })
        })
        .collect();
    locations.push(Location::AppPaths(exe, flavor));
    locations
}

#[cfg(windows)]
fn app_paths_entry(exe: &str) -> Option<PathBuf> {
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};

    let subkey: Vec<u16> = format!(r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\{exe}")
        .encode_utf16()
        .chain(Some(0))
        .collect();
    [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER]
        .into_iter()
        .filter_map(|root| registry_default_value(root, &subkey))
        .map(|value| value.trim().trim_matches('"').to_owned())
        .find(|value| !value.is_empty())
        .map(PathBuf::from)
}

#[cfg(windows)]
fn registry_default_value(
    root: windows_sys::Win32::System::Registry::HKEY,
    subkey: &[u16],
) -> Option<String> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{RRF_RT_REG_SZ, RegGetValueW};

    let mut size = 0u32;
    // SAFETY: subkey is NUL-terminated; a null data pointer asks only for the size.
    let status = unsafe {
        RegGetValueW(
            root,
            subkey.as_ptr(),
            std::ptr::null(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut size,
        )
    };
    if status != ERROR_SUCCESS || size < 2 {
        return None;
    }
    let mut buffer = vec![0u16; (size as usize).div_ceil(2)];
    // SAFETY: buffer holds `size` bytes, as reported by the previous call.
    let status = unsafe {
        RegGetValueW(
            root,
            subkey.as_ptr(),
            std::ptr::null(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buffer.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if status != ERROR_SUCCESS {
        return None;
    }
    let len = buffer
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(buffer.len());
    Some(
        std::ffi::OsString::from_wide(&buffer[..len])
            .to_string_lossy()
            .into_owned(),
    )
}

#[cfg(not(any(target_os = "macos", windows)))]
fn files(paths: &[&str], flavor: Flavor) -> Vec<Location> {
    paths
        .iter()
        .map(|&path| Location::File(PathBuf::from(path), flavor))
        .collect()
}

#[cfg(not(any(target_os = "macos", windows)))]
fn on_path(names: &'static [&'static str], flavor: Flavor) -> Vec<Location> {
    names
        .iter()
        .map(|&name| Location::OnPath(name, flavor))
        .collect()
}

#[cfg(not(any(target_os = "macos", windows)))]
fn locations(kind: BrowserKind) -> Vec<Location> {
    match kind {
        BrowserKind::Chrome => [
            files(
                &[
                    "/opt/google/chrome/chrome",
                    "/opt/google/chrome-beta/chrome",
                    "/opt/google/chrome-unstable/chrome",
                    "/opt/google/chrome-canary/chrome",
                ],
                Flavor::Chrome,
            ),
            on_path(&["google-chrome-stable", "google-chrome"], Flavor::Chrome),
            on_path(&["chromium", "chromium-browser"], Flavor::Chromium),
            files(&["/snap/bin/chromium"], Flavor::SnapChromium),
        ]
        .into_iter()
        .flatten()
        .collect(),
        BrowserKind::Edge => [
            files(
                &[
                    "/opt/microsoft/msedge/msedge",
                    "/opt/microsoft/msedge-beta/msedge",
                    "/opt/microsoft/msedge-dev/msedge",
                ],
                Flavor::Edge,
            ),
            on_path(
                &[
                    "microsoft-edge-stable",
                    "microsoft-edge-beta",
                    "microsoft-edge-dev",
                    "microsoft-edge",
                ],
                Flavor::Edge,
            ),
        ]
        .into_iter()
        .flatten()
        .collect(),
    }
}

#[cfg(target_os = "macos")]
fn platform_version(path: &Path) -> Option<String> {
    let plist = path.parent()?.parent()?.join("Info.plist");
    plist_short_version(&std::fs::read_to_string(plist).ok()?)
}

#[cfg(windows)]
fn platform_version(path: &Path) -> Option<String> {
    let names = std::fs::read_dir(path.parent()?)
        .ok()?
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| entry.file_name().into_string().ok());
    newest_version(names)
}

#[cfg(not(any(target_os = "macos", windows)))]
fn platform_version(path: &Path) -> Option<String> {
    version_from_output(&run_version_command(path)?)
}

#[cfg(not(any(target_os = "macos", windows)))]
fn run_version_command(path: &Path) -> Option<String> {
    use std::io::Read;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    let mut child = Command::new(path)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut output = String::new();
    child.stdout.take()?.read_to_string(&mut output).ok()?;
    Some(output)
}

#[cfg(any(test, target_os = "macos"))]
fn plist_short_version(plist: &str) -> Option<String> {
    let after_key = plist
        .split("<key>CFBundleShortVersionString</key>")
        .nth(1)?;
    let value = after_key.trim_start().strip_prefix("<string>")?;
    let version = value.split("</string>").next()?.trim();
    is_dotted_version(version).then(|| version.to_owned())
}

#[cfg(any(test, not(any(target_os = "macos", windows))))]
fn version_from_output(output: &str) -> Option<String> {
    output
        .split_whitespace()
        .find(|token| is_dotted_version(token))
        .map(str::to_owned)
}

#[cfg(any(test, windows))]
fn newest_version(names: impl Iterator<Item = String>) -> Option<String> {
    names
        .filter(|name| is_dotted_version(name))
        .max_by_key(|name| version_key(name))
}

#[cfg(any(test, windows))]
fn version_key(version: &str) -> Vec<u64> {
    version
        .split('.')
        .map(|part| part.parse().unwrap_or(0))
        .collect()
}

fn cft_dir_version(dir: &Path) -> Option<String> {
    let name = dir.file_name()?.to_str()?;
    let (_, version) = name.rsplit_once('-')?;
    is_dotted_version(version).then(|| version.to_owned())
}

fn is_dotted_version(text: &str) -> bool {
    let mut parts = 0;
    for part in text.split('.') {
        if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
            return false;
        }
        parts += 1;
    }
    parts >= 2
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_chrome_message_is_the_documented_text() {
        let searched = vec![
            "/opt/google/chrome/chrome".to_owned(),
            "google-chrome on PATH".to_owned(),
        ];
        assert_eq!(
            compose_missing_message(BrowserKind::Chrome, &searched, false, true),
            "No Chrome browser was found. Searched: /opt/google/chrome/chrome, google-chrome on PATH. \
             Install Google Chrome or Microsoft Edge, or download Chrome for Testing (about 200 MB from \
             storage.googleapis.com) in Settings > Automation."
        );
    }

    #[test]
    fn missing_message_names_edge_and_the_windows_hint() {
        let searched = vec![r"C:\Program Files\Google\Chrome\Application\chrome.exe".to_owned()];
        let message = compose_missing_message(BrowserKind::Chrome, &searched, true, true);
        assert!(message.ends_with(
            "in Settings > Automation. Microsoft Edge is installed: set Browser Type to Edge."
        ));
        let edge = compose_missing_message(BrowserKind::Edge, &searched, false, true);
        assert!(edge.starts_with("No Microsoft Edge browser was found. Searched: "));
    }

    #[test]
    fn missing_message_omits_chrome_for_testing_on_windows_arm() {
        let message = compose_missing_message(BrowserKind::Chrome, &["x".to_owned()], false, false);
        assert!(message.ends_with("Install Google Chrome or Microsoft Edge."));
        assert!(!message.contains("Chrome for Testing"));
    }

    #[test]
    fn chrome_searches_chrome_for_testing_before_snap_chromium() {
        let searched = searched_locations(BrowserKind::Chrome, Path::new("/cache"));
        let cft = searched
            .iter()
            .position(|entry| entry.starts_with("Chrome for Testing in "))
            .expect("the Chrome for Testing cache is searched");
        if let Some(snap) = searched
            .iter()
            .position(|entry| entry == "/snap/bin/chromium")
        {
            assert_eq!(snap, cft + 1);
        } else {
            assert_eq!(cft, searched.len() - 1);
        }
        assert!(
            !searched_locations(BrowserKind::Edge, Path::new("/cache"))
                .iter()
                .any(|entry| entry.contains("Chrome for Testing"))
        );
    }

    #[test]
    fn stable_chrome_is_the_first_candidate() {
        let first = candidates(BrowserKind::Chrome)
            .into_iter()
            .next()
            .expect("at least one candidate");
        assert_eq!(first.flavor, Flavor::Chrome);
        let path = first.path.to_string_lossy().into_owned();
        #[cfg(target_os = "macos")]
        assert_eq!(
            path,
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
        );
        #[cfg(target_os = "linux")]
        assert_eq!(path, "/opt/google/chrome/chrome");
        #[cfg(windows)]
        assert!(path.ends_with(r"Google\Chrome\Application\chrome.exe"));
    }

    #[test]
    fn brave_is_never_a_candidate() {
        for kind in [BrowserKind::Chrome, BrowserKind::Edge] {
            assert!(
                candidates(kind)
                    .iter()
                    .all(|candidate| !candidate.path.to_string_lossy().contains("Brave"))
            );
        }
    }

    #[test]
    fn snap_and_flatpak_chromium_are_recognised() {
        assert!(is_snap_or_flatpak(
            Path::new("/snap/chromium/3000/usr/lib/chromium-browser/chrome"),
            b""
        ));
        assert!(is_snap_or_flatpak(
            Path::new("/var/lib/flatpak/exports/bin/org.chromium.Chromium"),
            b""
        ));
        assert!(is_snap_or_flatpak(
            Path::new("/home/me/.local/share/flatpak/exports/bin/org.chromium.Chromium"),
            b""
        ));
        assert!(is_snap_or_flatpak(
            Path::new("/usr/bin/chromium-browser"),
            b"#! /bin/sh\nexec /snap/bin/chromium \"$@\"\n"
        ));
        assert!(!is_snap_or_flatpak(
            Path::new("/usr/lib/chromium/chromium"),
            b"\x7fELF\x02\x01\x01"
        ));
        assert!(!is_snap_or_flatpak(
            Path::new("/usr/bin/chromium"),
            b"#!/bin/sh\nexec /usr/lib/chromium/chromium \"$@\"\n"
        ));
    }

    #[test]
    fn a_path_chromium_behind_the_snap_launcher_is_packaged() {
        let elf = b"\x7fELF\x02\x01\x01";
        for (found, resolved) in [
            ("/snap/bin/chromium", "/usr/bin/snap"),
            ("/usr/local/bin/chromium", "/usr/bin/snap"),
            ("/var/lib/snapd/snap/bin/chromium", "/usr/bin/snap"),
            (
                "/usr/local/bin/chromium",
                "/var/lib/snapd/snap/chromium/3000/usr/lib/chromium-browser/chrome",
            ),
        ] {
            assert!(
                is_packaged_chromium(Path::new(found), Path::new(resolved), elf),
                "{found} -> {resolved}"
            );
        }
        assert!(is_snap_or_flatpak(
            Path::new("/var/lib/snapd/snap/bin/chromium"),
            b""
        ));
        assert!(!is_packaged_chromium(
            Path::new("/usr/bin/chromium"),
            Path::new("/usr/lib/chromium/chromium"),
            elf
        ));
        assert!(!is_packaged_chromium(
            Path::new("/usr/bin/chromium"),
            Path::new("/usr/bin/chromium"),
            b"#!/bin/sh\nexec /usr/lib/chromium/chromium \"$@\"\n"
        ));
    }

    #[cfg(unix)]
    #[test]
    fn a_path_chromium_linked_to_the_snap_launcher_is_skipped() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        let root = tempfile::tempdir().expect("tempdir");
        let executable = |path: &Path| {
            std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
            std::fs::write(path, b"\x7fELF\x02\x01\x01").expect("write");
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        };
        let launcher = root.path().join("usr/bin/snap");
        let distro = root.path().join("usr/lib/chromium/chromium");
        executable(&launcher);
        executable(&distro);
        let bin = root.path().join("snap/bin");
        std::fs::create_dir_all(&bin).expect("mkdir");
        symlink(&launcher, bin.join("chromium")).expect("symlink");
        symlink(&distro, bin.join("chromium-browser")).expect("symlink");

        assert!(resolve_path_hit(bin.join("chromium"), Flavor::Chromium).is_none());
        let accepted = resolve_path_hit(bin.join("chromium-browser"), Flavor::Chromium)
            .expect("a distro Chromium stays a candidate");
        assert_eq!(
            accepted.path,
            std::fs::canonicalize(&distro).expect("canonical")
        );
        assert_eq!(accepted.flavor, Flavor::Chromium);
    }

    #[test]
    fn versions_are_read_from_every_source() {
        let plist = "<dict>\n\t<key>CFBundleShortVersionString</key>\n\t<string>154.0.8037.92</string>\n</dict>";
        assert_eq!(plist_short_version(plist).as_deref(), Some("154.0.8037.92"));
        assert_eq!(plist_short_version("<dict></dict>"), None);
        assert_eq!(
            version_from_output("Google Chrome 154.0.8037.92 \n").as_deref(),
            Some("154.0.8037.92")
        );
        assert_eq!(
            version_from_output("Chromium 153.0.7000.1 snap").as_deref(),
            Some("153.0.7000.1")
        );
        assert_eq!(version_from_output("Chromium dev build"), None);
        let names = ["SetupMetrics", "99.0.1.2", "154.0.8037.92", "154.0.800.1"];
        assert_eq!(
            newest_version(names.iter().map(|name| (*name).to_owned())).as_deref(),
            Some("154.0.8037.92")
        );
    }

    #[test]
    fn chrome_for_testing_version_comes_from_the_install_directory() {
        let executable = Path::new(
            "/cache/chrome-for-testing/mac-arm64-154.0.8037.92/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing",
        );
        assert_eq!(
            cft_install_dir(executable),
            Some(Path::new(
                "/cache/chrome-for-testing/mac-arm64-154.0.8037.92"
            ))
        );
        assert_eq!(
            executable_version(executable, Flavor::ChromeForTesting).as_deref(),
            Some("154.0.8037.92")
        );
        assert_eq!(cft_install_dir(Path::new("/usr/bin/chromium")), None);
    }

    #[test]
    fn major_version_reads_the_first_component() {
        assert_eq!(major_version("154.0.8037.92"), Some(154));
        assert_eq!(major_version(" 155.1\n"), Some(155));
        assert_eq!(major_version("unknown"), None);
    }
}

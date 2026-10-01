// Derived from Chromium chrome/test/chromedriver @154.0.8037.92, Copyright The Chromium Authors, BSD-3-Clause; modified by Rheosoph GmbH. See NOTICE.
use std::ffi::OsString;
use std::path::Path;

use crate::launch::{Executable, Flavor, LaunchOptions};

const BASE_SWITCHES: &[&str] = &[
    "--allow-pre-commit-input",
    "--disable-background-networking",
    "--disable-background-timer-throttling",
    "--disable-backgrounding-occluded-windows",
    "--disable-client-side-phishing-detection",
    "--disable-default-apps",
    "--disable-features=IgnoreDuplicateNavs,Prewarm",
    "--disable-hang-monitor",
    "--disable-popup-blocking",
    "--disable-prompt-on-repost",
    "--disable-search-engine-choice-screen",
    "--disable-sync",
    "--enable-automation",
    "--no-first-run",
    "--no-service-autorun",
    "--password-store=basic",
    "--test-type=webdriver",
    "--use-mock-keychain",
    "--remote-debugging-port=0",
];

const LIFELINE_SWITCH: &str = "--remote-debugging-pipe";
const PROXY_RULE_SELECTORS: &[&str] = &["http", "https", "ftp", "socks"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HostOs {
    Unix,
    Windows,
}

impl HostOs {
    fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else {
            Self::Unix
        }
    }
}

pub fn launch_args(
    executable: &Executable,
    options: &LaunchOptions,
    profile_dir: &std::path::Path,
    temporary: bool,
) -> Vec<std::ffi::OsString> {
    args_for(
        executable.flavor,
        options,
        profile_dir,
        temporary,
        HostOs::current(),
    )
}

pub fn validate_proxy(server: &str) -> crate::Result<()> {
    match redact_userinfo(server) {
        Some(redacted) => Err(crate::BrowserError::Launch {
            message: format!(
                "Proxy credentials are not supported; remove user:password from '{redacted}'"
            ),
        }),
        None => Ok(()),
    }
}

#[cfg(any(unix, test))]
pub(crate) fn uses_lifeline(flavor: Flavor) -> bool {
    lifeline_for(flavor, HostOs::current())
}

fn lifeline_for(flavor: Flavor, os: HostOs) -> bool {
    os == HostOs::Unix && flavor != Flavor::SnapChromium
}

fn args_for(
    flavor: Flavor,
    options: &LaunchOptions,
    profile_dir: &Path,
    temporary: bool,
    os: HostOs,
) -> Vec<OsString> {
    let mut args: Vec<OsString> = BASE_SWITCHES.iter().map(OsString::from).collect();
    let mut user_data_dir = OsString::from("--user-data-dir=");
    user_data_dir.push(profile_dir.as_os_str());
    args.push(user_data_dir);
    args.extend(option_args(options));
    if flavor == Flavor::Edge && os == HostOs::Windows {
        args.push("--edge-skip-compat-layer-relaunch".into());
    }
    if lifeline_for(flavor, os) {
        args.push(LIFELINE_SWITCH.into());
    }
    if temporary {
        args.push("data:,".into());
    }
    args
}

fn option_args(options: &LaunchOptions) -> Vec<OsString> {
    let mut args = Vec::new();
    if options.headless {
        args.push("--headless".into());
    }
    let (width, height) = options.window_size;
    args.push(format!("--window-size={width},{height}").into());
    if let Some(user_agent) = non_empty(options.user_agent.as_deref()) {
        args.push(format!("--user-agent={user_agent}").into());
    }
    args.extend(proxy_args(options));
    if let Some(locale) = non_empty(options.locale.as_deref()) {
        args.push(format!("--lang={locale}").into());
    }
    if options.ignore_https_errors {
        args.push("--ignore-certificate-errors".into());
    }
    args
}

fn proxy_args(options: &LaunchOptions) -> Vec<OsString> {
    let Some(proxy) = &options.proxy else {
        return Vec::new();
    };
    let Some(server) = non_empty(Some(&proxy.server)) else {
        return Vec::new();
    };
    let mut args = vec![OsString::from(format!("--proxy-server={server}"))];
    let bypass: Vec<&str> = proxy
        .bypass
        .iter()
        .map(|host| host.trim())
        .filter(|host| !host.is_empty())
        .collect();
    if !bypass.is_empty() {
        args.push(format!("--proxy-bypass-list={}", bypass.join(";")).into());
    }
    args
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn redact_userinfo(server: &str) -> Option<String> {
    server.contains('@').then(|| {
        server
            .split(';')
            .map(redact_rule)
            .collect::<Vec<_>>()
            .join(";")
    })
}

fn redact_rule(rule: &str) -> String {
    let Some((before, host)) = rule.rsplit_once('@') else {
        return rule.to_owned();
    };
    format!("{}***@{host}", &before[..scheme_prefix_len(before)])
}

fn scheme_prefix_len(rule: &str) -> usize {
    let selector = rule
        .split_once('=')
        .filter(|(scheme, _)| PROXY_RULE_SELECTORS.contains(scheme))
        .map_or(0, |(scheme, _)| scheme.len() + 1);
    let uri_scheme = rule[selector..]
        .split_once("://")
        .filter(|(scheme, _)| is_uri_scheme(scheme))
        .map_or(0, |(scheme, _)| scheme.len() + 3);
    selector + uri_scheme
}

fn is_uri_scheme(token: &str) -> bool {
    token.starts_with(|c: char| c.is_ascii_alphabetic())
        && token
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Duration;

    use super::*;
    use crate::launch::{BrowserKind, ProxyConfig};

    fn options() -> LaunchOptions {
        LaunchOptions {
            kind: BrowserKind::Chrome,
            executable: None,
            headless: true,
            window_size: (1920, 1080),
            user_agent: None,
            user_data_dir: None,
            proxy: None,
            locale: None,
            ignore_https_errors: false,
            cache_dir: PathBuf::from("/cache"),
            page_load_timeout: Duration::from_secs(300),
            launch_timeout: Duration::from_secs(30),
        }
    }

    fn rendered(args: &[OsString]) -> Vec<String> {
        args.iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    fn with_base(extra: &[&str]) -> Vec<String> {
        BASE_SWITCHES
            .iter()
            .chain(extra)
            .map(|arg| (*arg).to_owned())
            .collect()
    }

    #[test]
    fn chrome_with_a_temporary_profile_gets_the_lifeline_and_a_blank_page() {
        let args = args_for(
            Flavor::Chrome,
            &options(),
            Path::new("/tmp/flow-like-browser-1-a"),
            true,
            HostOs::Unix,
        );
        assert_eq!(
            rendered(&args),
            with_base(&[
                "--user-data-dir=/tmp/flow-like-browser-1-a",
                "--headless",
                "--window-size=1920,1080",
                "--remote-debugging-pipe",
                "data:,",
            ])
        );
    }

    #[test]
    fn edge_on_windows_with_every_option() {
        let mut options = options();
        options.kind = BrowserKind::Edge;
        options.headless = false;
        options.window_size = (1280, 720);
        options.user_agent = Some("Agent/1.0".into());
        options.proxy = Some(ProxyConfig {
            server: "http://proxy:8080".into(),
            bypass: vec!["localhost".into(), " ".into(), "*.internal".into()],
        });
        options.locale = Some("de-DE".into());
        options.ignore_https_errors = true;
        let args = args_for(
            Flavor::Edge,
            &options,
            Path::new(r"C:\Profiles\edge"),
            false,
            HostOs::Windows,
        );
        assert_eq!(
            rendered(&args),
            with_base(&[
                r"--user-data-dir=C:\Profiles\edge",
                "--window-size=1280,720",
                "--user-agent=Agent/1.0",
                "--proxy-server=http://proxy:8080",
                "--proxy-bypass-list=localhost;*.internal",
                "--lang=de-DE",
                "--ignore-certificate-errors",
                "--edge-skip-compat-layer-relaunch",
            ])
        );
    }

    #[test]
    fn chrome_for_testing_matches_chrome() {
        let profile = Path::new("/profiles/cft");
        let cft = args_for(
            Flavor::ChromeForTesting,
            &options(),
            profile,
            false,
            HostOs::Unix,
        );
        let chrome = args_for(Flavor::Chrome, &options(), profile, false, HostOs::Unix);
        assert_eq!(cft, chrome);
        assert_eq!(
            rendered(&cft),
            with_base(&[
                "--user-data-dir=/profiles/cft",
                "--headless",
                "--window-size=1920,1080",
                "--remote-debugging-pipe",
            ])
        );
    }

    #[test]
    fn snap_chromium_has_no_lifeline() {
        let args = args_for(
            Flavor::SnapChromium,
            &options(),
            Path::new("/home/me/snap/chromium/common/flow-like-browser-1-a"),
            true,
            HostOs::Unix,
        );
        assert_eq!(
            rendered(&args),
            with_base(&[
                "--user-data-dir=/home/me/snap/chromium/common/flow-like-browser-1-a",
                "--headless",
                "--window-size=1920,1080",
                "data:,",
            ])
        );
    }

    #[test]
    fn features_are_disabled_in_one_switch_and_forbidden_switches_never_appear() {
        for flavor in [
            Flavor::Chrome,
            Flavor::Edge,
            Flavor::Chromium,
            Flavor::SnapChromium,
            Flavor::ChromeForTesting,
        ] {
            for os in [HostOs::Unix, HostOs::Windows] {
                let args = rendered(&args_for(flavor, &options(), Path::new("/p"), false, os));
                let features = args
                    .iter()
                    .filter(|arg| arg.starts_with("--disable-features"))
                    .count();
                assert_eq!(features, 1, "{flavor:?} {os:?}");
                for forbidden in [
                    "--no-sandbox",
                    "--disable-extensions",
                    "--disable-infobars",
                    "--disable-dev-shm-usage",
                    "--enable-logging",
                    "--disable-blink-features=AutomationControlled",
                ] {
                    assert!(!args.iter().any(|arg| arg.starts_with(forbidden)));
                }
                assert!(!args.iter().any(|arg| arg == "data:,"));
            }
        }
        assert!(uses_lifeline(Flavor::Chrome) == cfg!(unix));
    }

    #[test]
    fn proxy_credentials_are_rejected_without_echoing_the_password() {
        assert!(validate_proxy("http://proxy.corp:3128").is_ok());
        assert!(validate_proxy("socks5://127.0.0.1:1080").is_ok());
        assert!(validate_proxy("http=a:1;https=b:2").is_ok());
        let error = validate_proxy("http://alice:s3cret@proxy.corp:3128")
            .expect_err("credentials are rejected")
            .to_string();
        assert_eq!(
            error,
            "Proxy credentials are not supported; remove user:password from 'http://***@proxy.corp:3128'"
        );
        for (server, shown) in [
            ("http=bob:pw@a:1;https=b:2", "http=***@a:1;https=b:2"),
            ("http://ghp_abc123@proxy:3128", "http://***@proxy:3128"),
            ("u:p=w@host:1", "***@host:1"),
            ("socks5://u:a://b@h:1", "socks5://***@h:1"),
            ("https=socks5://u:p@s@h:1", "https=socks5://***@h:1"),
            ("HTTP=u:p@h:1", "***@h:1"),
        ] {
            let error = validate_proxy(server)
                .expect_err("credentials are rejected")
                .to_string();
            assert!(error.ends_with(&format!("'{shown}'")), "{server}: {error}");
        }
    }
}

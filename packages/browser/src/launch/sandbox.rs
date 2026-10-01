use crate::launch::{Executable, ExecutableSource};

const SETUID_HELPER: &str = "/opt/google/chrome/chrome-sandbox";
const APPARMOR_MESSAGE: &str = "This Linux restricts the browser sandbox (AppArmor). Install Google Chrome or Microsoft Edge, or run: sudo sysctl kernel.apparmor_restrict_unprivileged_userns=0";
const ROOT_MESSAGE: &str = "The browser refuses to run its sandbox as the root user (crbug.com/638180); run Flow-Like as a regular user";

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) enum SandboxDecision {
    Proceed,
    UseSetuidHelper(std::path::PathBuf),
    Refuse,
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn sandbox_decision(
    source: ExecutableSource,
    restrict_userns: Option<bool>,
    setuid_helper_exists: bool,
) -> SandboxDecision {
    match (source, restrict_userns) {
        (ExecutableSource::CachedCft, Some(true)) if setuid_helper_exists => {
            SandboxDecision::UseSetuidHelper(std::path::PathBuf::from(SETUID_HELPER))
        }
        (ExecutableSource::CachedCft, Some(true)) => SandboxDecision::Refuse,
        _ => SandboxDecision::Proceed,
    }
}

pub(crate) fn preflight(
    executable: &Executable,
) -> crate::Result<Vec<(std::ffi::OsString, std::ffi::OsString)>> {
    #[cfg(target_os = "linux")]
    {
        if executable.source != ExecutableSource::CachedCft {
            return Ok(Vec::new());
        }
        let restrict_userns =
            std::fs::read_to_string("/proc/sys/kernel/apparmor_restrict_unprivileged_userns")
                .ok()
                .and_then(|value| value.trim().parse::<u8>().ok())
                .map(|value| value != 0);
        let helper_exists = std::path::Path::new(SETUID_HELPER).exists();
        sandbox_env(sandbox_decision(
            executable.source,
            restrict_userns,
            helper_exists,
        ))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = executable;
        Ok(Vec::new())
    }
}

pub(crate) fn classify_stderr(stderr: &str) -> Option<crate::BrowserError> {
    let message = if stderr.contains("No usable sandbox!") {
        APPARMOR_MESSAGE
    } else if stderr.contains("crbug.com/638180") {
        ROOT_MESSAGE
    } else {
        return None;
    };
    Some(crate::BrowserError::Launch {
        message: message.to_owned(),
    })
}

#[cfg(any(test, target_os = "linux"))]
fn sandbox_env(
    decision: SandboxDecision,
) -> crate::Result<Vec<(std::ffi::OsString, std::ffi::OsString)>> {
    match decision {
        SandboxDecision::Proceed => Ok(Vec::new()),
        SandboxDecision::UseSetuidHelper(helper) => Ok(vec![(
            "CHROME_DEVEL_SANDBOX".into(),
            helper.into_os_string(),
        )]),
        SandboxDecision::Refuse => Err(crate::BrowserError::Launch {
            message: APPARMOR_MESSAGE.to_owned(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_downloaded_builds_are_checked() {
        use ExecutableSource::{CachedCft, Explicit, Installed};
        use SandboxDecision::{Proceed, Refuse, UseSetuidHelper};
        let helper = || UseSetuidHelper(std::path::PathBuf::from(SETUID_HELPER));
        let table = [
            (Installed, Some(true), false, Proceed),
            (Installed, Some(true), true, Proceed),
            (Explicit, Some(true), false, Proceed),
            (CachedCft, None, false, Proceed),
            (CachedCft, Some(false), false, Proceed),
            (CachedCft, Some(false), true, Proceed),
            (CachedCft, Some(true), true, helper()),
            (CachedCft, Some(true), false, Refuse),
        ];
        for (source, restrict, helper_exists, expected) in table {
            assert_eq!(
                sandbox_decision(source, restrict, helper_exists),
                expected,
                "{source:?} {restrict:?} {helper_exists}"
            );
        }
    }

    #[test]
    fn decisions_become_env_or_the_apparmor_error() {
        assert!(
            sandbox_env(SandboxDecision::Proceed)
                .expect("proceed")
                .is_empty()
        );
        let env =
            sandbox_env(SandboxDecision::UseSetuidHelper(SETUID_HELPER.into())).expect("helper");
        assert_eq!(
            env,
            vec![("CHROME_DEVEL_SANDBOX".into(), SETUID_HELPER.into())]
        );
        let error = sandbox_env(SandboxDecision::Refuse)
            .expect_err("refused")
            .to_string();
        assert_eq!(error, APPARMOR_MESSAGE);
        assert!(!error.contains("--no-sandbox"));
    }

    #[test]
    fn sandbox_failures_in_stderr_are_recognised() {
        let apparmor = classify_stderr(
            "[123:123:FATAL:zygote_host_impl_linux.cc(128)] No usable sandbox! Update your kernel",
        )
        .expect("sandbox failure");
        assert_eq!(apparmor.to_string(), APPARMOR_MESSAGE);
        let root = classify_stderr(
            "Running as root without --no-sandbox is not supported. See https://crbug.com/638180.",
        )
        .expect("root failure");
        assert_eq!(root.to_string(), ROOT_MESSAGE);
        assert!(classify_stderr("Fontconfig warning").is_none());
    }

    #[test]
    fn preflight_needs_no_env_outside_linux_or_for_installed_browsers() {
        let installed = Executable {
            path: "/opt/google/chrome/chrome".into(),
            flavor: crate::launch::Flavor::Chrome,
            version: None,
            source: ExecutableSource::Installed,
        };
        assert!(preflight(&installed).expect("preflight").is_empty());
    }
}

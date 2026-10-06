//! Build-time compatibility contract shared by registry and runtime crates.

/// Wasmtime's serialization compatibility version.
///
/// This is generated from the workspace's `wasmtime` dependency, so registry
/// platform keys cannot drift from the runtime that consumes AOT artifacts.
pub const WASMTIME_MAJOR_VERSION: &str = env!("FLOW_LIKE_WASMTIME_MAJOR_VERSION");

/// Backwards-compatible name for the supported Wasmtime major version.
pub const WASMTIME_VERSION: &str = WASMTIME_MAJOR_VERSION;

/// Serialized artifacts depend on the exact Wasmtime release and our engine
/// configuration. Bump the revision whenever that configuration changes.
pub const WASM_ARTIFACT_VERSION: &str =
    concat!(env!("FLOW_LIKE_WASMTIME_SERIALIZATION_VERSION"), "-p1");

pub fn artifact_platform_key(os: &str, arch: &str) -> String {
    let arch = if os == "ios" { "pulley64" } else { arch };
    format!("{os}-{arch}-wt{WASM_ARTIFACT_VERSION}")
}

/// An explicit target disables Wasmtime's compiler-host CPU feature inference.
/// Use the same baseline for the compiler and the consuming runtime.
pub fn portable_target(os: &str, arch: &str) -> Option<&'static str> {
    match (os, arch) {
        ("linux", "x86_64") => Some("x86_64-unknown-linux-gnu"),
        ("linux", "aarch64") => Some("aarch64-unknown-linux-gnu"),
        ("macos", "x86_64") => Some("x86_64-apple-darwin"),
        ("macos", "aarch64") => Some("aarch64-apple-darwin"),
        ("windows", "x86_64") => Some("x86_64-pc-windows-msvc"),
        ("windows", "aarch64") => Some("aarch64-pc-windows-msvc"),
        ("ios", "aarch64" | "x86_64" | "pulley64") => Some("pulley64"),
        ("android", "aarch64") => Some("aarch64-linux-android"),
        ("android", "x86_64") => Some("x86_64-linux-android"),
        _ => None,
    }
}

pub fn artifact_target(platform: &str) -> Option<&'static str> {
    let (host, version) = platform.rsplit_once("-wt")?;
    if version != WASM_ARTIFACT_VERSION {
        return None;
    }
    let (os, arch) = host.split_once('-')?;
    portable_target(os, arch)
}

/// Resolve a deployment's OS/architecture using this build's artifact version.
/// Legacy settings with the current major version also use the current artifact.
/// Explicit release/revision keys and other major versions remain unchanged.
pub fn current_artifact_platform(platform: &str) -> String {
    let host = match platform.rsplit_once("-wt") {
        Some((host, version)) if version == WASMTIME_MAJOR_VERSION => host,
        Some(_) => return platform.to_owned(),
        None => platform,
    };
    let Some((os, arch)) = host.split_once('-') else {
        return platform.to_owned();
    };
    if portable_target(os, arch).is_none() {
        return platform.to_owned();
    }
    artifact_platform_key(os, arch)
}

pub fn is_previous_artifact_platform(candidate: &str, current: &str) -> bool {
    if candidate == current || artifact_target(current).is_none() {
        return false;
    }
    let Some((host, version)) = candidate.rsplit_once("-wt") else {
        return false;
    };
    let Some((current_host, _)) = current.rsplit_once("-wt") else {
        return false;
    };
    host == current_host && version.split(['.', '-']).next() == Some(WASMTIME_MAJOR_VERSION)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deployment_targets_derive_the_artifact_version_from_this_build() {
        for (os, arch) in [
            ("linux", "x86_64"),
            ("linux", "aarch64"),
            ("macos", "x86_64"),
            ("macos", "aarch64"),
            ("windows", "x86_64"),
            ("windows", "aarch64"),
            ("android", "x86_64"),
            ("android", "aarch64"),
            ("ios", "aarch64"),
            ("ios", "pulley64"),
        ] {
            assert_eq!(
                current_artifact_platform(&format!("{os}-{arch}")),
                artifact_platform_key(os, arch)
            );
        }
    }

    #[test]
    fn explicit_versions_and_unknown_targets_are_not_reinterpreted() {
        for platform in [
            artifact_platform_key("linux", "x86_64"),
            format!("linux-aarch64-wt{WASMTIME_MAJOR_VERSION}.0.0-p0"),
            "linux-x86_64-wt1".to_owned(),
            "linux-x86_64-wt".to_owned(),
            "linux-x86_64-wtinvalid".to_owned(),
            "linux-s390x".to_owned(),
            "freebsd-x86_64".to_owned(),
            "linux-x86_64-extra".to_owned(),
            "aarch64".to_owned(),
            "arm64".to_owned(),
            "amd64".to_owned(),
            "x86_64".to_owned(),
            " linux-aarch64".to_owned(),
            "".to_owned(),
        ] {
            assert_eq!(current_artifact_platform(&platform), platform);
        }
    }

    #[test]
    fn artifact_identity_rejects_native_and_other_compiler_versions() {
        let platform = artifact_platform_key("linux", "x86_64");
        let legacy = format!("linux-x86_64-wt{WASMTIME_MAJOR_VERSION}");
        assert_eq!(artifact_target(&platform), Some("x86_64-unknown-linux-gnu"));
        assert_eq!(artifact_target(&legacy), None);
        assert_eq!(current_artifact_platform(&legacy), platform);
        assert!(is_previous_artifact_platform(
            &format!("linux-x86_64-wt{WASMTIME_MAJOR_VERSION}"),
            &platform
        ));
        assert!(!is_previous_artifact_platform(&platform, &platform));
        assert!(!is_previous_artifact_platform(
            &format!("linux-aarch64-wt{WASMTIME_MAJOR_VERSION}"),
            &platform
        ));
        assert_eq!(
            current_artifact_platform("linux-x86_64-wt1"),
            "linux-x86_64-wt1"
        );
        assert_eq!(artifact_target("linux-x86_64-wt1-p1"), None);
    }

    #[test]
    fn every_supported_platform_has_an_explicit_target_including_the_host() {
        for (os, arch) in [
            ("linux", "x86_64"),
            ("linux", "aarch64"),
            ("macos", "x86_64"),
            ("macos", "aarch64"),
            ("windows", "x86_64"),
            ("windows", "aarch64"),
            ("android", "x86_64"),
            ("android", "aarch64"),
            ("ios", "aarch64"),
        ] {
            assert!(artifact_target(&artifact_platform_key(os, arch)).is_some());
        }
    }
}

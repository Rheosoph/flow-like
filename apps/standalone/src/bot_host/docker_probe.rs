pub use flow_like_types::device_bot::DeviceDockerSupport;
use std::{
    future::Future,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::process::Command;

const PROBE_TIMEOUT: Duration = Duration::from_secs(4);
const NOT_INSTALLED: &str = "docker_not_installed";
const UNREACHABLE: &str = "docker_unreachable";
const REMOTE_ENDPOINT: &str = "docker_remote_endpoint";
const LINUX_REQUIRED: &str = "docker_linux_required";
const IMAGE_UNAVAILABLE: &str = "docker_image_unavailable";
const HUB_UNREACHABLE: &str = "docker_hub_unreachable";

fn docker_executable() -> PathBuf {
    let name = if cfg!(windows) {
        "docker.exe"
    } else {
        "docker"
    };
    let mut candidates: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|path| {
            std::env::split_paths(&path)
                .map(|directory| directory.join(name))
                .collect()
        })
        .unwrap_or_default();
    if let Some(home) = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }) {
        candidates.push(PathBuf::from(home).join(".docker/bin").join(name));
    }
    #[cfg(unix)]
    candidates.extend(
        [
            "/usr/local/bin/docker",
            "/opt/homebrew/bin/docker",
            "/usr/bin/docker",
            "/bin/docker",
            "/Applications/Docker.app/Contents/Resources/bin/docker",
        ]
        .into_iter()
        .map(PathBuf::from),
    );
    #[cfg(windows)]
    candidates.extend(windows_install_locations(
        std::env::var_os("ProgramFiles").map(PathBuf::from),
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from),
    ));
    resolve_executable(candidates).unwrap_or_else(|| PathBuf::from(name))
}

#[cfg(any(windows, test))]
fn windows_install_locations(
    program_files: Option<PathBuf>,
    local_app_data: Option<PathBuf>,
) -> impl Iterator<Item = PathBuf> {
    [
        program_files.map(|root| root.join("Docker/Docker/resources/bin/docker.exe")),
        local_app_data.map(|root| root.join("Programs/DockerDesktop/resources/bin/docker.exe")),
    ]
    .into_iter()
    .flatten()
}

fn resolve_executable(candidates: impl IntoIterator<Item = PathBuf>) -> Option<PathBuf> {
    candidates.into_iter().find(|path| {
        let Ok(metadata) = path.metadata() else {
            return false;
        };
        if !metadata.is_file() {
            return false;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            metadata.permissions().mode() & 0o111 != 0
        }
        #[cfg(not(unix))]
        {
            true
        }
    })
}

fn unavailable(reason: &'static str) -> DeviceDockerSupport {
    DeviceDockerSupport {
        installed: reason != NOT_INSTALLED,
        available: false,
        reason: Some(reason.into()),
    }
}

/// Reports the selected device's Docker service without starting or downloading anything.
pub async fn probe(state_dir: &Path, base_url: &str) -> DeviceDockerSupport {
    let result = async {
        let endpoint = local_endpoint().await?;
        let mut check = command(&endpoint);
        check.args(["info", "--format", "{{.OSType}}"]);
        let os = output(check).await?;
        if os.trim() != "linux" {
            return Err(LINUX_REQUIRED);
        }
        if !container_hub(base_url) {
            return Err(HUB_UNREACHABLE);
        }
        crate::release::installed_playground_container(state_dir).map_err(|_| IMAGE_UNAVAILABLE)?;
        Ok(())
    }
    .await;
    match result {
        Ok(()) => DeviceDockerSupport {
            installed: true,
            available: true,
            reason: None,
        },
        Err(reason) => unavailable(reason),
    }
}

fn container_hub(base_url: &str) -> bool {
    let Ok(url) = url::Url::parse(base_url) else {
        return false;
    };
    match url.host() {
        Some(url::Host::Domain(name)) => {
            let name = name.trim_end_matches('.');
            name != "localhost" && !name.ends_with(".localhost")
        }
        Some(url::Host::Ipv4(address)) => !address.is_loopback() && !address.is_unspecified(),
        Some(url::Host::Ipv6(address)) => {
            !address.is_loopback()
                && !address.is_unspecified()
                && !address
                    .to_ipv4_mapped()
                    .is_some_and(|ipv4| ipv4.is_loopback() || ipv4.is_unspecified())
        }
        None => false,
    }
}

/// The coordinator pins every command to this local socket rather than a mutable CLI context.
pub(crate) async fn local_endpoint() -> Result<String, &'static str> {
    let host = std::env::var("DOCKER_HOST").ok();
    let context = std::env::var("DOCKER_CONTEXT").ok();
    let mut inspect = Command::new(docker_executable());
    inspect.args([
        "context",
        "inspect",
        "--format",
        "{{json .Endpoints.docker.Host}}",
    ]);
    if let Some(context) = context.as_deref().filter(|value| !value.is_empty()) {
        inspect.arg(context);
    }
    let inspected = output(inspect).await;
    if inspected == Err(NOT_INSTALLED) {
        return Err(NOT_INSTALLED);
    }
    endpoint_with(
        host.as_deref(),
        context.as_deref(),
        std::future::ready(inspected),
    )
    .await
}

async fn endpoint_with(
    host: Option<&str>,
    context: Option<&str>,
    inspect: impl Future<Output = Result<String, &'static str>>,
) -> Result<String, &'static str> {
    let endpoint = if context.filter(|value| !value.is_empty()).is_none()
        && let Some(host) = host.filter(|value| !value.is_empty())
    {
        host.to_owned()
    } else {
        serde_json::from_str::<String>(&inspect.await?).map_err(|_| UNREACHABLE)?
    };
    if !local_socket(&endpoint) {
        return Err(REMOTE_ENDPOINT);
    }
    Ok(endpoint)
}

fn local_socket(endpoint: &str) -> bool {
    !endpoint.chars().any(char::is_control)
        && (endpoint
            .strip_prefix("unix://")
            .is_some_and(|path| path.starts_with('/') && path.len() > 1)
            || (cfg!(windows)
                && endpoint
                    .strip_prefix("npipe:////./pipe/")
                    .is_some_and(|pipe| !pipe.is_empty())))
}

pub(crate) fn command(endpoint: &str) -> Command {
    let mut command = Command::new(docker_executable());
    command
        .args(["--host", endpoint])
        .env_remove("DOCKER_CONTEXT")
        .env_remove("DOCKER_HOST")
        .kill_on_drop(true);
    command
}

async fn output(mut command: Command) -> Result<String, &'static str> {
    command.kill_on_drop(true);
    let result = tokio::time::timeout(PROBE_TIMEOUT, command.output())
        .await
        .map_err(|_| UNREACHABLE)?
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                NOT_INSTALLED
            } else {
                UNREACHABLE
            }
        })?;
    if !result.status.success() || result.stdout.len() > 4096 {
        return Err(UNREACHABLE);
    }
    String::from_utf8(result.stdout).map_err(|_| UNREACHABLE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::ready;

    #[test]
    fn windows_resolution_includes_system_and_per_user_docker_desktop_installs() {
        let system = PathBuf::from("system");
        let per_user = PathBuf::from("per-user");
        assert_eq!(
            windows_install_locations(Some(system.clone()), Some(per_user.clone()))
                .collect::<Vec<_>>(),
            [
                system.join("Docker/Docker/resources/bin/docker.exe"),
                per_user.join("Programs/DockerDesktop/resources/bin/docker.exe"),
            ]
        );
        assert_eq!(
            windows_install_locations(None, Some(per_user.clone())).collect::<Vec<_>>(),
            [per_user.join("Programs/DockerDesktop/resources/bin/docker.exe")]
        );
    }

    #[test]
    fn docker_resolution_prefers_path_and_uses_install_locations_when_path_is_incomplete() {
        let directory = tempfile::tempdir().unwrap();
        let path_cli = directory.path().join("path-docker");
        let installed_cli = directory.path().join("installed-docker");
        for path in [&path_cli, &installed_cli] {
            std::fs::write(path, b"test executable").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
            }
        }
        assert_eq!(
            resolve_executable([path_cli.clone(), installed_cli.clone()]),
            Some(path_cli)
        );
        assert_eq!(
            resolve_executable([
                directory.path().join("missing"),
                directory.path().to_path_buf(),
                installed_cli.clone()
            ]),
            Some(installed_cli)
        );
        assert_eq!(resolve_executable([directory.path().join("missing")]), None);
    }

    #[test]
    fn a_loopback_hub_cannot_be_reached_from_the_containers_own_network() {
        for url in [
            "http://localhost:8080/api/v1",
            "https://api.localhost/api/v1",
            "https://LOCALHOST./api/v1",
            "http://127.9.1.2/api/v1",
            "http://127.1/api/v1",
            "http://0.0.0.0/api/v1",
            "http://[::1]/api/v1",
            "http://[::]/api/v1",
            "http://[::ffff:127.0.0.1]/api/v1",
            "invalid hub URL",
        ] {
            assert!(!container_hub(url), "{url}");
        }
        for url in [
            "https://hub.example/api/v1",
            "https://10.0.0.4/api/v1",
            "http://host.docker.internal:8080/api/v1",
        ] {
            assert!(container_hub(url), "{url}");
        }
    }

    #[tokio::test]
    async fn only_local_sockets_can_run_a_selected_devices_bot() {
        for endpoint in [
            "tcp://localhost:2375",
            "ssh://device",
            "unix://relative",
            "unix:///",
            "unix:///tmp/docker\nsock",
        ] {
            assert_eq!(
                endpoint_with(Some(endpoint), None, ready(Err(NOT_INSTALLED))).await,
                Err(REMOTE_ENDPOINT)
            );
        }
        assert_eq!(
            endpoint_with(
                Some("unix:///run/user/1000/docker.sock"),
                None,
                ready(Err(NOT_INSTALLED))
            )
            .await,
            Ok("unix:///run/user/1000/docker.sock".into())
        );
    }

    #[tokio::test]
    async fn a_named_context_overrides_docker_host_and_is_checked() {
        assert_eq!(
            endpoint_with(
                Some("unix:///var/run/docker.sock"),
                Some("remote"),
                ready(Ok("\"ssh://other-device\"".into()))
            )
            .await,
            Err(REMOTE_ENDPOINT)
        );
        assert_eq!(
            endpoint_with(
                Some("ssh://unused"),
                Some("desktop-linux"),
                ready(Ok(
                    "\"unix:///home/person/.docker/desktop/docker.sock\"".into()
                ))
            )
            .await,
            Ok("unix:///home/person/.docker/desktop/docker.sock".into())
        );
        assert_eq!(
            endpoint_with(None, None, ready(Err(NOT_INSTALLED))).await,
            Err(NOT_INSTALLED)
        );
        assert_eq!(
            endpoint_with(None, None, ready(Ok("invalid metadata".into()))).await,
            Err(UNREACHABLE)
        );
    }

    #[test]
    fn coordinator_commands_cannot_follow_a_changed_context() {
        let command = command("unix:///var/run/docker.sock");
        let command = command.as_std();
        let args: Vec<_> = command.get_args().collect();
        assert_eq!(args, ["--host", "unix:///var/run/docker.sock"]);
        let removed: Vec<_> = command
            .get_envs()
            .filter_map(|(key, value)| value.is_none().then_some(key))
            .collect();
        assert!(removed.contains(&std::ffi::OsStr::new("DOCKER_CONTEXT")));
        assert!(removed.contains(&std::ffi::OsStr::new("DOCKER_HOST")));
    }

    #[tokio::test]
    async fn missing_cli_is_distinct_from_a_service_that_is_not_running() {
        let reason = output(Command::new(
            "/path-that-does-not-exist/flow-like-docker-probe",
        ))
        .await
        .unwrap_err();
        assert_eq!(
            unavailable(reason),
            DeviceDockerSupport {
                installed: false,
                available: false,
                reason: Some(NOT_INSTALLED.into()),
            }
        );
        assert!(unavailable(UNREACHABLE).installed);
    }
}

//! Tests for the language-neutral sandbox. Tests that need the CPython interpreter run when
//! `SANDBOX_TEST_PYTHON_WASM` points at the pinned `python.wasm` (or `SANDBOX_TEST_DOWNLOAD=1`
//! fetches it from the release URLs); otherwise they print SKIPPED.

#![cfg(feature = "execute")]

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use flow_like_catalog_code_interpreter::pyodide::network::GuestNetwork;
use flow_like_catalog_code_interpreter::sandbox::artifact::sha256_hex;
use flow_like_catalog_code_interpreter::sandbox::{
    ArtifactSource, ArtifactStore, Language, NOT_AVAILABLE_ON_DEVICE, NetworkPolicy,
    RuntimeArtifact, SandboxConfig, SandboxFile, SandboxLimits, SandboxRequest, SandboxRuntime,
    SandboxStatus, validate_relative_path,
};
use serde_json::json;

struct LocalFileSource(Vec<u8>);

#[async_trait]
impl ArtifactSource for LocalFileSource {
    async fn fetch(&self, _url: &str, _max_bytes: u64) -> Result<Vec<u8>> {
        Ok(self.0.clone())
    }
}

struct FailingSource;

#[async_trait]
impl ArtifactSource for FailingSource {
    async fn fetch(&self, url: &str, _max_bytes: u64) -> Result<Vec<u8>> {
        Err(anyhow!("offline: {url}"))
    }
}

fn leak_artifact(payload: &[u8]) -> RuntimeArtifact {
    RuntimeArtifact {
        id: "test",
        version: "1",
        file_name: "test.bin",
        sha256: Box::leak(sha256_hex(payload).into_boxed_str()),
        size: payload.len() as u64,
        urls: &["https://example.invalid/test.bin"],
    }
}

mod download_integrity {
    use super::*;

    #[tokio::test]
    async fn corrupted_download_is_refused_and_nothing_is_installed() {
        let good = b"pinned runtime bytes".to_vec();
        let artifact = leak_artifact(&good);
        let mut corrupted = good.clone();
        corrupted[3] ^= 0xff;

        let dir = tempfile::tempdir().unwrap();
        let store = ArtifactStore::new(dir.path(), Arc::new(LocalFileSource(corrupted)));
        let err = store.ensure(&artifact).await.unwrap_err();

        assert!(format!("{err:#}").contains("integrity check"), "{err:#}");
        assert!(!store.path_of(&artifact).exists());
        assert!(!store.is_installed(&artifact));
    }

    #[tokio::test]
    async fn truncated_download_is_refused() {
        let good = b"pinned runtime bytes".to_vec();
        let artifact = leak_artifact(&good);
        let dir = tempfile::tempdir().unwrap();
        let store = ArtifactStore::new(dir.path(), Arc::new(LocalFileSource(good[..5].to_vec())));
        assert!(store.ensure(&artifact).await.is_err());
        assert!(!store.path_of(&artifact).exists());
    }

    #[tokio::test]
    async fn good_download_is_installed_and_reused_offline() {
        let good = b"pinned runtime bytes".to_vec();
        let artifact = leak_artifact(&good);
        let dir = tempfile::tempdir().unwrap();

        let store = ArtifactStore::new(dir.path(), Arc::new(LocalFileSource(good.clone())));
        let path = store.ensure(&artifact).await.unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), good);

        let offline = ArtifactStore::new(dir.path(), Arc::new(FailingSource));
        assert_eq!(offline.ensure(&artifact).await.unwrap(), path);
    }

    #[tokio::test]
    async fn tampered_cache_is_discarded_and_redownloaded() {
        let good = b"pinned runtime bytes".to_vec();
        let artifact = leak_artifact(&good);
        let dir = tempfile::tempdir().unwrap();
        let store = ArtifactStore::new(dir.path(), Arc::new(LocalFileSource(good.clone())));
        let path = store.path_of(&artifact);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"pinned runtime bytez").unwrap();

        store.ensure(&artifact).await.unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), good);
    }

    #[tokio::test]
    async fn tampered_cache_without_network_fails_instead_of_running_it() {
        let good = b"pinned runtime bytes".to_vec();
        let artifact = leak_artifact(&good);
        let dir = tempfile::tempdir().unwrap();
        let store = ArtifactStore::new(dir.path(), Arc::new(FailingSource));
        let path = store.path_of(&artifact);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"pinned runtime bytez").unwrap();

        assert!(store.ensure(&artifact).await.is_err());
    }

    #[tokio::test]
    async fn run_reports_unavailable_when_the_interpreter_is_corrupt() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = SandboxConfig::new(dir.path());
        config.source = Arc::new(LocalFileSource(vec![0u8; 16]));
        let runtime = SandboxRuntime::new(config);

        let response = runtime
            .run(SandboxRequest::new(Language::Python, "print(1)"))
            .await;
        assert_eq!(response.status, SandboxStatus::Unavailable);
        assert!(response.error.unwrap().contains("not ready"));
    }
}

mod unsupported_device {
    use super::*;

    fn ios_runtime() -> SandboxRuntime {
        let dir = tempfile::tempdir().unwrap();
        let mut config = SandboxConfig::new(dir.keep());
        config.platform = "ios";
        config.source = Arc::new(FailingSource);
        SandboxRuntime::new(config)
    }

    #[tokio::test]
    async fn ios_advertises_no_language_and_refuses_cleanly() {
        let runtime = ios_runtime();
        assert!(!runtime.is_supported());
        assert!(runtime.languages().is_empty());

        let response = runtime
            .run(SandboxRequest::new(Language::Python, "print(1)"))
            .await;
        assert_eq!(response.status, SandboxStatus::Unavailable);
        assert_eq!(response.error.as_deref(), Some(NOT_AVAILABLE_ON_DEVICE));

        let prepared = runtime.prepare(Language::Python).await;
        assert_eq!(prepared.unwrap_err().to_string(), NOT_AVAILABLE_ON_DEVICE);
    }

    #[test]
    fn desktop_platforms_advertise_python_and_javascript() {
        for platform in ["macos", "windows", "linux", "android"] {
            let dir = tempfile::tempdir().unwrap();
            let mut config = SandboxConfig::new(dir.path());
            config.platform = platform;
            let languages = SandboxRuntime::new(config).languages();
            let ids: Vec<&str> = languages.iter().map(|l| l.id).collect();
            assert_eq!(ids, ["python", "javascript"], "{platform}");
            assert!(languages.iter().all(|l| !l.installed), "{platform}");
            assert!(languages[0].packages.contains(&"numpy"));
            assert!(languages[1].packages.is_empty());
            assert_eq!(languages[1].download_bytes, 1_571_399);
        }
    }
}

mod network_policy {
    use super::*;
    use std::net::IpAddr;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn requests_are_offline_by_default() {
        let request = SandboxRequest::new(Language::Python, "");
        assert_eq!(request.network, NetworkPolicy::Off);
        assert_eq!(NetworkPolicy::default(), NetworkPolicy::Off);
        assert_eq!(GuestNetwork::default(), GuestNetwork::Off);
    }

    #[test]
    fn off_permits_nothing() {
        for addr in ["93.184.216.34", "8.8.8.8", "127.0.0.1"] {
            assert!(!GuestNetwork::Off.permits(ip(addr)));
        }
    }

    #[tokio::test]
    async fn allow_hosts_permits_only_the_listed_hosts() {
        let policy =
            GuestNetwork::resolve_hosts(&["93.184.216.34".into(), "[2606:2800::1]:443".into()])
                .await;
        assert!(policy.permits(ip("93.184.216.34")));
        assert!(policy.permits(ip("2606:2800::1")));
        assert!(!policy.permits(ip("8.8.8.8")));
    }

    #[tokio::test]
    async fn allow_hosts_never_reaches_the_host_plane() {
        let policy = GuestNetwork::resolve_hosts(&[
            "127.0.0.1".into(),
            "169.254.169.254".into(),
            "93.184.216.34:443".into(),
        ])
        .await;
        assert!(!policy.permits(ip("127.0.0.1")));
        assert!(!policy.permits(ip("169.254.169.254")));
        assert!(policy.permits(ip("93.184.216.34")));
    }

    #[tokio::test]
    async fn an_unresolvable_allowlist_permits_nothing() {
        let policy = GuestNetwork::resolve_hosts(&["no-such-host.invalid".into()]).await;
        assert!(!policy.permits(ip("93.184.216.34")));
    }
}

mod paths {
    use super::*;

    #[test]
    fn relative_paths_only() {
        assert_eq!(validate_relative_path("a/b.csv").unwrap(), "a/b.csv");
        for bad in [
            "",
            "/etc/passwd",
            "../x",
            "a/../b",
            "a//b",
            "a\\b",
            "./a",
            "a/\0",
        ] {
            assert!(validate_relative_path(bad).is_err(), "{bad:?}");
        }
    }
}

mod execution {
    use super::*;

    fn shared_runtime() -> Option<Arc<SandboxRuntime>> {
        static RUNTIME: OnceLock<Option<Arc<SandboxRuntime>>> = OnceLock::new();
        RUNTIME
            .get_or_init(|| {
                let source: Arc<dyn ArtifactSource> =
                    if let Ok(path) = std::env::var("SANDBOX_TEST_PYTHON_WASM") {
                        Arc::new(LocalFileSource(std::fs::read(path).ok()?))
                    } else if std::env::var("SANDBOX_TEST_DOWNLOAD").is_ok() {
                        Arc::new(
                        flow_like_catalog_code_interpreter::sandbox::HttpArtifactSource::default(),
                    )
                    } else {
                        return None;
                    };
                let dir = std::env::temp_dir().join("flow-sandbox-tests");
                let mut config = SandboxConfig::new(dir);
                config.source = source;
                config.max_concurrent_runs = 4;
                Some(Arc::new(SandboxRuntime::new(config)))
            })
            .clone()
    }

    macro_rules! runtime_or_skip {
        () => {
            match shared_runtime() {
                Some(runtime) => runtime,
                None => {
                    eprintln!("SKIPPED: set SANDBOX_TEST_PYTHON_WASM or SANDBOX_TEST_DOWNLOAD=1");
                    return;
                }
            }
        };
    }

    fn python(code: &str) -> SandboxRequest {
        SandboxRequest::new(Language::Python, code)
    }

    #[tokio::test]
    async fn hello_world() {
        let runtime = runtime_or_skip!();
        let mut request = python("print('hello'); outputs['sum'] = sum(inputs['xs'])");
        request.inputs = json!({"xs": [1, 2, 3]});
        let response = runtime.run(request).await;

        assert_eq!(response.status, SandboxStatus::Ok, "{:?}", response.error);
        assert_eq!(response.stdout.trim(), "hello");
        assert_eq!(response.outputs["sum"], 6);
        assert!(response.error.is_none());
        assert!(runtime.languages()[0].installed);
    }

    #[tokio::test]
    async fn uncaught_exception_is_an_error_with_traceback() {
        let runtime = runtime_or_skip!();
        let response = runtime.run(python("raise ValueError('boom')")).await;
        assert_eq!(response.status, SandboxStatus::Error);
        assert!(response.error.unwrap().contains("ValueError: boom"));
    }

    #[tokio::test]
    async fn files_go_in_and_new_or_changed_files_come_out() {
        let runtime = runtime_or_skip!();
        let mut request = python(
            r#"
rows = open('/flow/in/data.csv').read().strip().splitlines()
open('result.txt', 'w').write(str(len(rows)))
import os
os.makedirs('nested', exist_ok=True)
open('nested/deep.bin', 'wb').write(bytes([0, 1, 2]))
with open('/flow/in/edit.txt', 'a') as f:
    f.write('!')
"#,
        );
        request.files = vec![
            SandboxFile::new("data.csv", "a,b\n1,2\n3,4\n"),
            SandboxFile::new("edit.txt", "keep"),
            SandboxFile::new("untouched.txt", "same"),
        ];
        let response = runtime.run(request).await;

        assert_eq!(response.status, SandboxStatus::Ok, "{:?}", response.error);
        let file = |p: &str| response.files.iter().find(|f| f.path == p);
        assert_eq!(file("out/result.txt").unwrap().bytes, b"3");
        assert_eq!(file("out/nested/deep.bin").unwrap().bytes, vec![0, 1, 2]);
        assert_eq!(file("in/edit.txt").unwrap().bytes, b"keep!");
        assert!(file("in/untouched.txt").is_none());
        assert!(file("in/data.csv").is_none());
    }

    #[tokio::test]
    async fn invalid_input_paths_are_rejected_before_running() {
        let runtime = runtime_or_skip!();
        let mut request = python("print(1)");
        request.files = vec![SandboxFile::new("../escape.txt", "x")];
        let response = runtime.run(request).await;
        assert_eq!(response.status, SandboxStatus::Error);
        assert!(response.error.unwrap().contains("invalid file path"));
    }

    #[tokio::test]
    async fn only_flow_is_reachable() {
        let runtime = runtime_or_skip!();
        let response = runtime
            .run(python(
                "import os\noutputs['etc'] = os.path.exists('/etc/passwd')\noutputs['flow'] = sorted(os.listdir('/flow'))\noutputs['cwd'] = os.getcwd()",
            ))
            .await;
        assert_eq!(response.status, SandboxStatus::Ok, "{:?}", response.error);
        assert_eq!(response.outputs["etc"], false);
        assert_eq!(response.outputs["cwd"], "/flow/out");
        assert!(
            response.outputs["flow"]
                .as_array()
                .unwrap()
                .contains(&json!("in"))
        );
    }

    #[tokio::test]
    async fn wall_time_limit_stops_a_busy_loop() {
        let runtime = runtime_or_skip!();
        let mut request = python("while True:\n    pass");
        request.limits.wall_time = Duration::from_secs(1);
        let started = std::time::Instant::now();
        let response = runtime.run(request).await;

        assert_eq!(
            response.status,
            SandboxStatus::Timeout,
            "{:?}",
            response.error
        );
        assert!(started.elapsed() < Duration::from_secs(15));
    }

    #[tokio::test]
    async fn memory_limit_turns_a_large_allocation_into_an_error() {
        let runtime = runtime_or_skip!();
        let mut request = python("x = bytearray(400 * 1024 * 1024)\nprint(len(x))");
        request.limits.memory_bytes = 128 * 1024 * 1024;
        let response = runtime.run(request).await;

        assert_eq!(
            response.status,
            SandboxStatus::OutOfMemory,
            "{:?}",
            response.error
        );
        assert!(response.stdout.is_empty());
    }

    #[tokio::test]
    async fn fuel_budget_stops_the_program() {
        let runtime = runtime_or_skip!();
        let mut request = python("n = 0\nwhile True:\n    n += 1");
        request.limits.fuel = Some(200_000_000);
        request.limits.wall_time = Duration::from_secs(60);
        let response = runtime.run(request).await;
        assert_eq!(
            response.status,
            SandboxStatus::FuelExhausted,
            "{:?}",
            response.error
        );
    }

    #[tokio::test]
    async fn output_is_capped_and_flagged() {
        let runtime = runtime_or_skip!();
        let mut request =
            python("print('x' * 5_000_000)\nimport sys\nsys.stderr.write('e' * 5_000_000)");
        request.limits.max_output_bytes = 1000;
        let response = runtime.run(request).await;

        assert_eq!(response.status, SandboxStatus::Ok, "{:?}", response.error);
        assert_eq!(response.stdout.len(), 1000);
        assert_eq!(response.stderr.len(), 1000);
        assert!(response.truncated.stdout && response.truncated.stderr);
    }

    #[tokio::test]
    async fn result_files_are_capped() {
        let runtime = runtime_or_skip!();
        let mut request = python("for i in range(5):\n    open(f'f{i}.txt', 'w').write('x' * 100)");
        request.limits = SandboxLimits {
            max_result_files: 3,
            ..SandboxLimits::default()
        };
        let response = runtime.run(request).await;
        assert_eq!(response.files.len(), 3);
        assert!(response.truncated.files);
    }

    #[tokio::test]
    async fn sockets_are_unavailable_without_a_network_policy() {
        let runtime = runtime_or_skip!();
        let response = runtime
            .run(python(
                "import socket\ntry:\n    s = socket.socket()\n    s.connect(('93.184.216.34', 80))\n    outputs['connected'] = True\nexcept Exception as e:\n    outputs['connected'] = False",
            ))
            .await;
        assert_eq!(response.outputs["connected"], false, "{:?}", response.error);
    }
}

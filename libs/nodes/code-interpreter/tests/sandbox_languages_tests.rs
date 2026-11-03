//! JavaScript, the scientific Python build and the bundle integrity checks.
//!
//! The real-interpreter tests need `SANDBOX_TEST_DOWNLOAD=1` (QuickJS, ~1.5 MB) and print
//! SKIPPED otherwise. The scientific tests also need `SANDBOX_TEST_SCIENTIFIC=1` because the
//! first run downloads ~98 MB and compiles ~100 MB of WebAssembly.

#![cfg(feature = "execute")]

use std::io::Write;
use std::sync::{Arc, OnceLock};

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use flow_like_catalog_code_interpreter::sandbox::artifact::{extract_members, sha256_hex};
use flow_like_catalog_code_interpreter::sandbox::{
    ArtifactSource, ArtifactStore, BundleMember, HttpArtifactSource, Language, QUICKJS_ARTIFACT,
    RuntimeArtifact, RuntimeBundle, SCIENTIFIC_PYTHON_BUNDLE, SandboxConfig, SandboxFile,
    SandboxLimits, SandboxRequest, SandboxRuntime, SandboxStatus, canonical_package,
};
use serde_json::json;

struct Fixed(Vec<u8>);

#[async_trait]
impl ArtifactSource for Fixed {
    async fn fetch(&self, _url: &str, _max_bytes: u64) -> Result<Vec<u8>> {
        Ok(self.0.clone())
    }
}

struct Offline;

#[async_trait]
impl ArtifactSource for Offline {
    async fn fetch(&self, url: &str, _max_bytes: u64) -> Result<Vec<u8>> {
        Err(anyhow!("offline: {url}"))
    }
}

mod pins {
    use super::*;

    #[test]
    fn quickjs_pin_is_a_wellformed_sha256_with_a_cdn_then_upstream_url() {
        assert_eq!(QUICKJS_ARTIFACT.sha256.len(), 64);
        assert!(QUICKJS_ARTIFACT.sha256.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(QUICKJS_ARTIFACT.urls[0].starts_with("https://cdn.flow-like.com/runtimes/quickjs/"));
        assert!(QUICKJS_ARTIFACT.urls[1].starts_with("https://github.com/quickjs-ng/quickjs/"));
    }

    #[test]
    fn scientific_bundle_pins_the_wheel_and_both_members() {
        let bundle = &SCIENTIFIC_PYTHON_BUNDLE;
        assert_eq!(bundle.archive.sha256.len(), 64);
        assert_eq!(bundle.members.len(), 2);
        for member in bundle.members {
            assert_eq!(member.sha256.len(), 64, "{}", member.path_in_archive);
        }
        assert!(bundle.archive.urls[1].starts_with("https://files.pythonhosted.org/"));
    }

    #[test]
    fn package_names_map_to_the_scientific_set() {
        assert_eq!(canonical_package("NumPy"), Some("numpy"));
        assert_eq!(canonical_package("sklearn"), Some("scikit-learn"));
        assert_eq!(canonical_package("scikit_learn"), Some("scikit-learn"));
        assert_eq!(canonical_package("PIL"), Some("pillow"));
        assert_eq!(canonical_package("requests"), None);
    }
}

mod bundle_integrity {
    use super::*;

    fn zip_with(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buffer = std::io::Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut buffer);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            for (name, bytes) in entries {
                writer.start_file(*name, options).unwrap();
                writer.write_all(bytes).unwrap();
            }
            writer.finish().unwrap();
        }
        buffer.into_inner()
    }

    struct Fixture {
        bundle: RuntimeBundle,
        archive: Vec<u8>,
    }

    fn fixture() -> Fixture {
        let engine = b"pretend interpreter".to_vec();
        let stdlib = b"pretend stdlib".to_vec();
        let archive = zip_with(&[("pkg/engine.bin", &engine), ("pkg/stdlib.zip", &stdlib)]);
        let leak = |s: String| -> &'static str { Box::leak(s.into_boxed_str()) };
        let members: &'static [BundleMember] = Box::leak(Box::new([
            BundleMember {
                path_in_archive: "pkg/engine.bin",
                file_name: "engine.bin",
                sha256: leak(sha256_hex(&engine)),
                size: engine.len() as u64,
            },
            BundleMember {
                path_in_archive: "pkg/stdlib.zip",
                file_name: "lib/stdlib.zip",
                sha256: leak(sha256_hex(&stdlib)),
                size: stdlib.len() as u64,
            },
        ]));
        let bundle = RuntimeBundle {
            id: "fixture",
            version: "1",
            archive: RuntimeArtifact {
                id: "fixture-archive",
                version: "1",
                file_name: "fixture.zip",
                sha256: leak(sha256_hex(&archive)),
                size: archive.len() as u64,
                urls: &["https://example.invalid/fixture.zip"],
            },
            members,
        };
        Fixture { bundle, archive }
    }

    #[tokio::test]
    async fn a_good_archive_is_extracted_and_reused_offline() {
        let f = fixture();
        let dir = tempfile::tempdir().unwrap();
        let store = ArtifactStore::new(dir.path(), Arc::new(Fixed(f.archive.clone())));
        assert!(!store.is_bundle_installed(&f.bundle));
        let root = store.ensure_bundle(&f.bundle).await.unwrap();
        assert_eq!(std::fs::read(root.join("engine.bin")).unwrap(), b"pretend interpreter");
        assert_eq!(std::fs::read(root.join("lib/stdlib.zip")).unwrap(), b"pretend stdlib");
        assert!(store.is_bundle_installed(&f.bundle));

        let offline = ArtifactStore::new(dir.path(), Arc::new(Offline));
        assert_eq!(offline.ensure_bundle(&f.bundle).await.unwrap(), root);
    }

    #[tokio::test]
    async fn a_tampered_archive_is_refused_and_nothing_is_installed() {
        let f = fixture();
        let mut bad = f.archive.clone();
        let middle = bad.len() / 2;
        bad[middle] ^= 0xff;
        let dir = tempfile::tempdir().unwrap();
        let store = ArtifactStore::new(dir.path(), Arc::new(Fixed(bad)));
        let err = store.ensure_bundle(&f.bundle).await.unwrap_err();
        assert!(format!("{err:#}").contains("integrity check"), "{err:#}");
        assert!(!store.is_bundle_installed(&f.bundle));
    }

    #[tokio::test]
    async fn a_member_that_differs_from_its_pin_is_refused_even_in_a_pinned_archive() {
        let f = fixture();
        let swapped = zip_with(&[
            ("pkg/engine.bin", b"EVIL interpreter xx"),
            ("pkg/stdlib.zip", b"pretend stdlib"),
        ]);
        let leak = |s: String| -> &'static str { Box::leak(s.into_boxed_str()) };
        let bundle = RuntimeBundle {
            archive: RuntimeArtifact {
                sha256: leak(sha256_hex(&swapped)),
                size: swapped.len() as u64,
                ..f.bundle.archive.clone()
            },
            ..f.bundle.clone()
        };
        let dir = tempfile::tempdir().unwrap();
        let store = ArtifactStore::new(dir.path(), Arc::new(Fixed(swapped)));
        let err = store.ensure_bundle(&bundle).await.unwrap_err();
        assert!(format!("{err:#}").contains("integrity check"), "{err:#}");
        assert!(!store.bundle_dir(&bundle).join("engine.bin").exists());
    }

    #[tokio::test]
    async fn a_tampered_cached_member_is_discarded_and_redownloaded() {
        let f = fixture();
        let dir = tempfile::tempdir().unwrap();
        let store = ArtifactStore::new(dir.path(), Arc::new(Fixed(f.archive.clone())));
        let root = store.ensure_bundle(&f.bundle).await.unwrap();
        std::fs::write(root.join("engine.bin"), b"pretend interpreteR").unwrap();

        store.ensure_bundle(&f.bundle).await.unwrap();
        assert_eq!(std::fs::read(root.join("engine.bin")).unwrap(), b"pretend interpreter");

        std::fs::write(root.join("engine.bin"), b"pretend interpreteR").unwrap();
        let offline = ArtifactStore::new(dir.path(), Arc::new(Offline));
        assert!(offline.ensure_bundle(&f.bundle).await.is_err());
    }

    #[test]
    fn a_missing_member_is_an_error() {
        let f = fixture();
        let archive = zip_with(&[("pkg/engine.bin", b"pretend interpreter")]);
        let err = extract_members(&archive, f.bundle.members).unwrap_err();
        assert!(format!("{err:#}").contains("not in the archive"), "{err:#}");
    }

    #[tokio::test]
    async fn quickjs_with_a_flipped_byte_is_refused() {
        let mut bytes = vec![0u8; QUICKJS_ARTIFACT.size as usize];
        bytes[100] = 1;
        let dir = tempfile::tempdir().unwrap();
        let store = ArtifactStore::new(dir.path(), Arc::new(Fixed(bytes)));
        let err = store.ensure(&QUICKJS_ARTIFACT).await.unwrap_err();
        assert!(format!("{err:#}").contains("integrity check"), "{err:#}");
        assert!(!store.is_installed(&QUICKJS_ARTIFACT));
    }

    #[tokio::test]
    async fn the_real_scientific_pins_refuse_a_wrong_wheel() {
        let bytes = vec![0u8; SCIENTIFIC_PYTHON_BUNDLE.archive.size as usize];
        let dir = tempfile::tempdir().unwrap();
        let store = ArtifactStore::new(dir.path(), Arc::new(Fixed(bytes)));
        assert!(store.ensure_bundle(&SCIENTIFIC_PYTHON_BUNDLE).await.is_err());
        assert!(!store.is_bundle_installed(&SCIENTIFIC_PYTHON_BUNDLE));
    }
}

mod request_validation {
    use super::*;

    fn offline_runtime() -> SandboxRuntime {
        let dir = tempfile::tempdir().unwrap();
        let mut config = SandboxConfig::new(dir.keep());
        config.source = Arc::new(Offline);
        SandboxRuntime::new(config)
    }

    #[tokio::test]
    async fn unknown_python_packages_are_rejected_with_the_available_list() {
        let mut request = SandboxRequest::new(Language::Python, "print(1)");
        request.packages = vec!["numpy".into(), "requests".into()];
        let response = offline_runtime().run(request).await;
        assert_eq!(response.status, SandboxStatus::Error);
        let error = response.error.unwrap();
        assert!(error.contains("'requests'") && error.contains("numpy"), "{error}");
    }

    #[tokio::test]
    async fn javascript_has_no_packages() {
        let mut request = SandboxRequest::new(Language::JavaScript, "1");
        request.packages = vec!["lodash".into()];
        let response = offline_runtime().run(request).await;
        assert_eq!(response.status, SandboxStatus::Error);
        assert!(response.error.unwrap().contains("no packages"));
    }

    #[tokio::test]
    async fn javascript_without_a_download_is_unavailable_not_a_panic() {
        let response = offline_runtime()
            .run(SandboxRequest::new(Language::JavaScript, "1"))
            .await;
        assert_eq!(response.status, SandboxStatus::Unavailable);
    }

    #[test]
    fn language_names_parse() {
        for name in ["js", "JavaScript", "node", "nodejs", "quickjs"] {
            assert_eq!(Language::parse(name), Some(Language::JavaScript), "{name}");
        }
        assert_eq!(Language::parse("ruby"), None);
    }
}

fn real_runtime(scientific_only: bool) -> Option<Arc<SandboxRuntime>> {
    static RUNTIME: OnceLock<Option<Arc<SandboxRuntime>>> = OnceLock::new();
    let enabled = std::env::var("SANDBOX_TEST_DOWNLOAD").as_deref() == Ok("1")
        && (!scientific_only || std::env::var("SANDBOX_TEST_SCIENTIFIC").as_deref() == Ok("1"));
    if !enabled {
        return None;
    }
    RUNTIME
        .get_or_init(|| {
            let mut config = SandboxConfig::new(std::env::temp_dir().join("flow-sandbox-tests"));
            config.source = Arc::new(HttpArtifactSource::default());
            config.max_concurrent_runs = 4;
            Some(Arc::new(SandboxRuntime::new(config)))
        })
        .clone()
}

macro_rules! runtime_or_skip {
    ($scientific:expr) => {
        match real_runtime($scientific) {
            Some(runtime) => runtime,
            None => {
                eprintln!("SKIPPED: set SANDBOX_TEST_DOWNLOAD=1 (and SANDBOX_TEST_SCIENTIFIC=1)");
                return;
            }
        }
    };
}

fn js(code: &str) -> SandboxRequest {
    SandboxRequest::new(Language::JavaScript, code)
}

mod javascript {
    use super::*;

    #[tokio::test]
    async fn hello_world_with_inputs_and_outputs() {
        let runtime = runtime_or_skip!(false);
        let mut request = js("console.log('hello', inputs.xs.length); outputs.sum = inputs.xs.reduce((a, b) => a + b, 0);");
        request.inputs = json!({"xs": [1, 2, 3]});
        let response = runtime.run(request).await;

        assert_eq!(response.status, SandboxStatus::Ok, "{:?} {}", response.error, response.stderr);
        assert_eq!(response.stdout.trim(), "hello 3");
        assert_eq!(response.outputs["sum"], 6);
        let info = runtime.languages();
        assert!(info.iter().find(|l| l.id == "javascript").unwrap().installed);
    }

    #[tokio::test]
    async fn uncaught_errors_report_status_error_with_the_message() {
        let runtime = runtime_or_skip!(false);
        let response = runtime.run(js("throw new TypeError('boom')")).await;
        assert_eq!(response.status, SandboxStatus::Error);
        assert!(response.error.unwrap().contains("boom"));
    }

    #[tokio::test]
    async fn console_error_goes_to_stderr_and_top_level_await_works() {
        let runtime = runtime_or_skip!(false);
        let response = runtime
            .run(js("console.error('warned'); outputs.v = await Promise.resolve(41) + 1;"))
            .await;
        assert_eq!(response.status, SandboxStatus::Ok, "{:?}", response.error);
        assert_eq!(response.stderr.trim(), "warned");
        assert_eq!(response.outputs["v"], 42);
    }

    #[tokio::test]
    async fn files_go_in_and_new_or_changed_files_come_out() {
        let runtime = runtime_or_skip!(false);
        let mut request = js(r#"
import * as std from "std";
const rows = std.loadFile("/flow/in/data.csv").trim().split("\n");
const out = std.open("result.txt", "w");
out.puts(String(rows.length));
out.close();
"#);
        request.files = vec![
            SandboxFile::new("data.csv", "a,b\n1,2\n3,4\n"),
            SandboxFile::new("untouched.txt", "same"),
        ];
        let response = runtime.run(request).await;
        assert_eq!(response.status, SandboxStatus::Ok, "{:?} {}", response.error, response.stderr);
        let file = |p: &str| response.files.iter().find(|f| f.path == p);
        assert_eq!(file("out/result.txt").unwrap().bytes, b"3");
        assert!(file("in/untouched.txt").is_none());
    }

    #[tokio::test]
    async fn wall_time_stops_a_busy_loop() {
        let runtime = runtime_or_skip!(false);
        let mut request = js("while (true) {}");
        request.limits = SandboxLimits {
            wall_time: std::time::Duration::from_millis(500),
            ..SandboxLimits::default()
        };
        let response = runtime.run(request).await;
        assert_eq!(response.status, SandboxStatus::Timeout);
    }

    #[tokio::test]
    async fn memory_limit_turns_a_large_allocation_into_an_error() {
        let runtime = runtime_or_skip!(false);
        let mut request = js("const a = []; for (let i = 0; i < 1e9; i++) a.push(new Array(1e6).fill(i));");
        request.limits = SandboxLimits {
            memory_bytes: 64 * 1024 * 1024,
            ..SandboxLimits::default()
        };
        let response = runtime.run(request).await;
        assert!(
            matches!(response.status, SandboxStatus::OutOfMemory | SandboxStatus::Error),
            "{:?} {:?}",
            response.status,
            response.error
        );
    }

    #[tokio::test]
    async fn output_is_capped_and_flagged() {
        let runtime = runtime_or_skip!(false);
        let mut request = js("for (let i = 0; i < 5000; i++) console.log('x'.repeat(100));");
        request.limits = SandboxLimits {
            max_output_bytes: 1000,
            ..SandboxLimits::default()
        };
        let response = runtime.run(request).await;
        assert_eq!(response.status, SandboxStatus::Ok, "{:?}", response.error);
        assert!(response.stdout.len() <= 1000);
        assert!(response.truncated.stdout);
    }

    #[tokio::test]
    async fn only_flow_is_reachable() {
        let runtime = runtime_or_skip!(false);
        let response = runtime
            .run(js(r#"
import * as std from "std";
const f = std.open("/etc/passwd", "r");
outputs.etc = f !== null;
"#))
            .await;
        assert_eq!(response.outputs["etc"], false, "{:?} {}", response.error, response.stderr);
    }

    #[tokio::test]
    async fn png_and_csv_outputs_are_collected() {
        let runtime = runtime_or_skip!(false);
        let response = runtime
            .run(js(r#"
import * as std from "std";
const png = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
const f = std.open("plot.png", "wb");
const buf = new Uint8Array(png).buffer;
f.write(buf, 0, png.length);
f.close();
const csv = std.open("table.csv", "w"); csv.puts("a,b\n1,2\n"); csv.close();
"#))
            .await;
        assert_eq!(response.status, SandboxStatus::Ok, "{:?} {}", response.error, response.stderr);
        let png = response.files.iter().find(|f| f.path == "out/plot.png").unwrap();
        assert_eq!(&png.bytes[..4], &[0x89, 0x50, 0x4e, 0x47]);
        assert!(response.files.iter().any(|f| f.path == "out/table.csv"));
    }
}

mod scientific {
    use super::*;

    fn python_with(packages: &[&str], code: &str) -> SandboxRequest {
        let mut request = SandboxRequest::new(Language::Python, code);
        request.packages = packages.iter().map(|p| p.to_string()).collect();
        request.limits.wall_time = std::time::Duration::from_secs(240);
        request
    }

    #[tokio::test]
    async fn numpy_and_pandas_run() {
        let runtime = runtime_or_skip!(true);
        let response = runtime
            .run(python_with(
                &["numpy", "pandas"],
                r#"
import numpy as np, pandas as pd
df = pd.DataFrame({"a": [1, 2, 3], "b": [4.0, 5.0, 6.0]})
outputs["mean"] = float(np.mean(df["b"]))
outputs["shape"] = list(df.shape)
df.to_csv("out.csv", index=False)
print(df.describe().loc["max", "a"])
"#,
            ))
            .await;
        assert_eq!(response.status, SandboxStatus::Ok, "{:?} {}", response.error, response.stderr);
        assert_eq!(response.outputs["mean"], 5.0);
        assert_eq!(response.outputs["shape"], json!([3, 2]));
        assert_eq!(response.stdout.trim(), "3.0");
        let csv = response.files.iter().find(|f| f.path == "out/out.csv").unwrap();
        assert_eq!(csv.bytes, b"a,b\n1,4.0\n2,5.0\n3,6.0\n");
        assert!(runtime.languages()[0].packages_installed);
    }

    #[tokio::test]
    async fn matplotlib_writes_a_png() {
        let runtime = runtime_or_skip!(true);
        let response = runtime
            .run(python_with(
                &["matplotlib"],
                r#"
import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
plt.plot([1, 2, 3], [1, 4, 9])
plt.savefig("fig.png")
"#,
            ))
            .await;
        assert_eq!(response.status, SandboxStatus::Ok, "{:?} {}", response.error, response.stderr);
        let png = response.files.iter().find(|f| f.path == "out/fig.png").unwrap();
        assert_eq!(&png.bytes[..4], &[0x89, 0x50, 0x4e, 0x47]);
    }

    #[tokio::test]
    async fn plain_python_still_has_no_numpy() {
        let runtime = runtime_or_skip!(true);
        let response = runtime
            .run(SandboxRequest::new(Language::Python, "import numpy"))
            .await;
        assert_eq!(response.status, SandboxStatus::Error);
        assert!(response.error.unwrap().contains("ModuleNotFoundError"));
    }
}

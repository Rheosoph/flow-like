//! Language-neutral code sandbox for bots and hosts.
//!
//! [`SandboxRuntime`] runs one program per call inside a fresh WASI instance whose only
//! reachable directory is `/flow`:
//!
//! ```text
//! /flow/in/<path>   staged input files (changes are reported back)
//! /flow/out/<path>  the working directory; every file written here is returned
//! ```
//!
//! Languages: Python (CPython 3.12 for WASI, stdlib only; `packages: ["numpy", ...]` switches to
//! the optional scientific CPython build) and JavaScript (QuickJS-NG for WASI). Interpreters are
//! not bundled: [`SandboxRuntime::prepare`] downloads a pinned, sha256-verified copy into
//! `<data_dir>` (default: the app cache dir, see [`default_data_dir`]) on first use and
//! precompiles it for the host. iOS refuses with [`NOT_AVAILABLE_ON_DEVICE`] until a pulley64
//! artifact exists.

pub mod artifact;
mod files;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::{OnceCell, Semaphore};

use crate::pyodide::network::GuestNetwork;
use crate::pyodide::runtime::{
    BOOTSTRAP_SCRIPT, BootstrapResult, GuestLimits, GuestOutcome, GuestSpec, PyodideRuntime,
    RuntimeConfig,
};

pub use artifact::{
    ArtifactSource, ArtifactStore, BundleMember, HttpArtifactSource, PYTHON_ARTIFACT,
    QUICKJS_ARTIFACT, RuntimeArtifact, RuntimeBundle, SCIENTIFIC_PYTHON_BUNDLE,
};
pub use files::validate_relative_path;

pub const NOT_AVAILABLE_ON_DEVICE: &str = "Code execution is not available on this device yet.";

const JS_BOOTSTRAP_SCRIPT: &str = include_str!("bootstrap.js");

/// The packages that come with the scientific Python build (`SandboxRequest::packages`). Names
/// are matched case-insensitively with `-` and `_` treated alike; the right-hand side is the id
/// the model-facing docs use.
pub const SCIENTIFIC_PACKAGES: &[&str] = &[
    "numpy",
    "pandas",
    "scipy",
    "scikit-learn",
    "statsmodels",
    "matplotlib",
    "seaborn",
    "pillow",
    "sympy",
    "networkx",
    "openpyxl",
    "lxml",
    "beautifulsoup4",
    "pyyaml",
    "jinja2",
];

/// The guest needs about this much linear memory just to start the scientific build.
const SCIENTIFIC_MIN_MEMORY_BYTES: usize = 1024 * 1024 * 1024;

pub fn canonical_package(name: &str) -> Option<&'static str> {
    let key = name.trim().to_ascii_lowercase().replace('_', "-");
    let key = match key.as_str() {
        "sklearn" => "scikit-learn",
        "pil" => "pillow",
        "bs4" => "beautifulsoup4",
        "yaml" => "pyyaml",
        other => other,
    };
    SCIENTIFIC_PACKAGES.iter().copied().find(|p| *p == key)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    Python,
    JavaScript,
}

impl Language {
    pub const ALL: [Language; 2] = [Language::Python, Language::JavaScript];

    pub fn id(self) -> &'static str {
        match self {
            Self::Python => "python",
            Self::JavaScript => "javascript",
        }
    }

    pub fn parse(id: &str) -> Option<Self> {
        match id.trim().to_ascii_lowercase().as_str() {
            "python" | "py" | "python3" => Some(Self::Python),
            "javascript" | "js" | "node" | "nodejs" | "quickjs" => Some(Self::JavaScript),
            _ => None,
        }
    }

    pub fn file_extension(self) -> &'static str {
        match self {
            Self::Python => "py",
            Self::JavaScript => "js",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct LanguageInfo {
    pub language: Language,
    pub id: &'static str,
    pub display_name: &'static str,
    pub description: &'static str,
    pub file_extension: &'static str,
    pub installed: bool,
    pub download_bytes: u64,
    /// Packages `SandboxRequest::packages` can load; empty when the language has none.
    pub packages: &'static [&'static str],
    pub packages_installed: bool,
    pub packages_download_bytes: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum NetworkPolicy {
    #[default]
    Off,
    AllowHosts(Vec<String>),
}

#[derive(Debug, Clone)]
pub struct SandboxLimits {
    pub wall_time: Duration,
    pub memory_bytes: usize,
    pub fuel: Option<u64>,
    pub max_output_bytes: usize,
    pub max_input_bytes: u64,
    pub max_result_files: usize,
    pub max_result_bytes: u64,
}

impl Default for SandboxLimits {
    fn default() -> Self {
        Self {
            wall_time: Duration::from_secs(30),
            memory_bytes: 256 * 1024 * 1024,
            fuel: None,
            max_output_bytes: 1024 * 1024,
            max_input_bytes: 64 * 1024 * 1024,
            max_result_files: 50,
            max_result_bytes: 64 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxFile {
    pub path: String,
    pub bytes: Vec<u8>,
}

impl SandboxFile {
    pub fn new(path: impl Into<String>, bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            path: path.into(),
            bytes: bytes.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SandboxRequest {
    pub language: Language,
    pub code: String,
    pub inputs: Value,
    pub files: Vec<SandboxFile>,
    pub limits: SandboxLimits,
    pub network: NetworkPolicy,
    /// Optional library sets to load; see [`SCIENTIFIC_PACKAGES`]. Python only.
    pub packages: Vec<String>,
}

impl SandboxRequest {
    pub fn new(language: Language, code: impl Into<String>) -> Self {
        Self {
            language,
            code: code.into(),
            inputs: Value::Object(Default::default()),
            files: Vec::new(),
            limits: SandboxLimits::default(),
            network: NetworkPolicy::Off,
            packages: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxStatus {
    Ok,
    Error,
    Timeout,
    OutOfMemory,
    FuelExhausted,
    Unavailable,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Truncation {
    pub stdout: bool,
    pub stderr: bool,
    pub files: bool,
}

#[derive(Debug, Clone)]
pub struct SandboxResponse {
    pub status: SandboxStatus,
    pub outputs: Value,
    pub stdout: String,
    pub stderr: String,
    pub error: Option<String>,
    /// Paths are relative to `/flow`: `out/…` for everything written there, `in/…` for staged
    /// inputs the program modified.
    pub files: Vec<SandboxFile>,
    pub truncated: Truncation,
    pub duration_ms: u64,
}

impl SandboxResponse {
    fn failed(status: SandboxStatus, error: impl Into<String>, started: Instant) -> Self {
        Self {
            status,
            outputs: Value::Object(Default::default()),
            stdout: String::new(),
            stderr: String::new(),
            error: Some(error.into()),
            files: Vec::new(),
            truncated: Truncation::default(),
            duration_ms: started.elapsed().as_millis() as u64,
        }
    }

    pub fn is_ok(&self) -> bool {
        self.status == SandboxStatus::Ok
    }
}

#[derive(Clone)]
pub struct SandboxConfig {
    pub data_dir: PathBuf,
    pub source: Arc<dyn ArtifactSource>,
    pub platform: &'static str,
    pub max_concurrent_runs: usize,
    pub python_artifact: RuntimeArtifact,
    pub quickjs_artifact: RuntimeArtifact,
    pub scientific_bundle: RuntimeBundle,
}

impl SandboxConfig {
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
            source: Arc::new(HttpArtifactSource::default()),
            platform: std::env::consts::OS,
            max_concurrent_runs: 2,
            python_artifact: PYTHON_ARTIFACT,
            quickjs_artifact: QUICKJS_ARTIFACT,
            scientific_bundle: SCIENTIFIC_PYTHON_BUNDLE,
        }
    }
}

pub fn default_data_dir() -> PathBuf {
    crate::flow_like::utils::cache::get_cache_dir().join("sandbox")
}

pub fn shared_runtime() -> Arc<SandboxRuntime> {
    static SHARED: std::sync::OnceLock<Arc<SandboxRuntime>> = std::sync::OnceLock::new();
    SHARED
        .get_or_init(|| Arc::new(SandboxRuntime::new(SandboxConfig::new(default_data_dir()))))
        .clone()
}

pub fn platform_support(platform: &str) -> std::result::Result<(), &'static str> {
    match platform {
        "ios" => Err(NOT_AVAILABLE_ON_DEVICE),
        _ => Ok(()),
    }
}

struct Prepared {
    runtime: Arc<PyodideRuntime>,
    spec: GuestSpec,
    scripts: files::Scripts,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Interpreter {
    Python,
    Scientific,
    JavaScript,
}

pub struct SandboxRuntime {
    config: SandboxConfig,
    store: ArtifactStore,
    python: OnceCell<Arc<Prepared>>,
    scientific: OnceCell<Arc<Prepared>>,
    javascript: OnceCell<Arc<Prepared>>,
    permits: Arc<Semaphore>,
}

impl SandboxRuntime {
    pub fn new(config: SandboxConfig) -> Self {
        let store = ArtifactStore::new(config.data_dir.join("artifacts"), config.source.clone());
        let permits = Arc::new(Semaphore::new(config.max_concurrent_runs.max(1)));
        Self {
            config,
            store,
            python: OnceCell::new(),
            scientific: OnceCell::new(),
            javascript: OnceCell::new(),
            permits,
        }
    }

    pub fn is_supported(&self) -> bool {
        platform_support(self.config.platform).is_ok()
    }

    pub fn languages(&self) -> Vec<LanguageInfo> {
        if !self.is_supported() {
            return Vec::new();
        }
        vec![
            LanguageInfo {
                language: Language::Python,
                id: Language::Python.id(),
                display_name: "Python 3.12",
                description: "CPython for WASI with the standard library. Pass packages (numpy, pandas, scipy, scikit-learn, matplotlib, ...) to run on the optional scientific build (CPython 3.15, large first download).",
                file_extension: Language::Python.file_extension(),
                installed: self.store.is_installed(&self.config.python_artifact),
                download_bytes: self.config.python_artifact.size,
                packages: SCIENTIFIC_PACKAGES,
                packages_installed: self.store.is_bundle_installed(&self.config.scientific_bundle),
                packages_download_bytes: self.config.scientific_bundle.archive.size,
            },
            LanguageInfo {
                language: Language::JavaScript,
                id: Language::JavaScript.id(),
                display_name: "JavaScript (QuickJS)",
                description: "ES2023 on QuickJS-NG for WASI: no Node or browser APIs, no npm. Top-level await works; console.log is captured.",
                file_extension: Language::JavaScript.file_extension(),
                installed: self.store.is_installed(&self.config.quickjs_artifact),
                download_bytes: self.config.quickjs_artifact.size,
                packages: &[],
                packages_installed: false,
                packages_download_bytes: 0,
            },
        ]
    }

    pub async fn prepare(&self, language: Language) -> Result<()> {
        platform_support(self.config.platform).map_err(|reason| anyhow!(reason))?;
        let interpreter = match language {
            Language::Python => Interpreter::Python,
            Language::JavaScript => Interpreter::JavaScript,
        };
        self.prepared(interpreter).await.map(|_| ())
    }

    pub async fn prepare_packages(&self) -> Result<()> {
        platform_support(self.config.platform).map_err(|reason| anyhow!(reason))?;
        self.prepared(Interpreter::Scientific).await.map(|_| ())
    }

    async fn prepared(&self, interpreter: Interpreter) -> Result<Arc<Prepared>> {
        let cell = match interpreter {
            Interpreter::Python => &self.python,
            Interpreter::Scientific => &self.scientific,
            Interpreter::JavaScript => &self.javascript,
        };
        cell.get_or_try_init(|| async {
            let (wasm, spec, scripts) = match interpreter {
                Interpreter::Python => (
                    self.store.ensure(&self.config.python_artifact).await?,
                    GuestSpec::python_cli(),
                    files::Scripts {
                        bootstrap_name: "bootstrap.py",
                        bootstrap: BOOTSTRAP_SCRIPT,
                        code_name: "code.py",
                    },
                ),
                Interpreter::JavaScript => (
                    self.store.ensure(&self.config.quickjs_artifact).await?,
                    GuestSpec::command(&["qjs", "--std", "/flow/bootstrap.js"]),
                    files::Scripts {
                        bootstrap_name: "bootstrap.js",
                        bootstrap: JS_BOOTSTRAP_SCRIPT,
                        code_name: "code.js",
                    },
                ),
                Interpreter::Scientific => {
                    let dir = self.store.ensure_bundle(&self.config.scientific_bundle).await?;
                    let mut spec = GuestSpec::command(&[
                        "/usr/local/bin/python.wasm",
                        "/flow/bootstrap.py",
                    ]);
                    spec.reactor = true;
                    spec.env = vec![
                        ("TMPDIR".to_string(), "/tmp".to_string()),
                        ("PYTHONTZPATH".to_string(), String::new()),
                    ];
                    spec.read_only_mounts = vec![(dir.join("lib"), "/usr/local/lib".to_string())];
                    (
                        dir.join("python.wasm"),
                        spec,
                        files::Scripts {
                            bootstrap_name: "bootstrap.py",
                            bootstrap: BOOTSTRAP_SCRIPT,
                            code_name: "code.py",
                        },
                    )
                }
            };
            let runtime = PyodideRuntime::new(RuntimeConfig {
                wasm_binary_path: Some(wasm),
                aot_cache_dir: Some(self.config.data_dir.join("aot")),
                fuel_metering: true,
            })?;
            runtime.compiled_module().await?;
            Ok(Arc::new(Prepared {
                runtime: Arc::new(runtime),
                spec,
                scripts,
            }))
        })
        .await
        .cloned()
    }

    pub async fn run(&self, mut request: SandboxRequest) -> SandboxResponse {
        let started = Instant::now();
        if let Err(reason) = platform_support(self.config.platform) {
            return SandboxResponse::failed(SandboxStatus::Unavailable, reason, started);
        }
        let interpreter = match resolve_interpreter(&request) {
            Ok(interpreter) => interpreter,
            Err(message) => return SandboxResponse::failed(SandboxStatus::Error, message, started),
        };
        if interpreter == Interpreter::Scientific {
            request.limits.memory_bytes = request
                .limits
                .memory_bytes
                .max(SCIENTIFIC_MIN_MEMORY_BYTES);
        }
        let prepared = match self.prepared(interpreter).await {
            Ok(prepared) => prepared,
            Err(e) => {
                return SandboxResponse::failed(
                    SandboxStatus::Unavailable,
                    format!("{} runtime is not ready: {e:#}", request.language.id()),
                    started,
                );
            }
        };
        let Ok(_permit) = self.permits.clone().acquire_owned().await else {
            return SandboxResponse::failed(
                SandboxStatus::Unavailable,
                "sandbox is closed",
                started,
            );
        };
        match execute(&prepared, request).await {
            Ok(mut response) => {
                response.duration_ms = started.elapsed().as_millis() as u64;
                response
            }
            Err(e) => SandboxResponse::failed(SandboxStatus::Error, format!("{e:#}"), started),
        }
    }
}

fn resolve_interpreter(request: &SandboxRequest) -> std::result::Result<Interpreter, String> {
    match request.language {
        Language::JavaScript if !request.packages.is_empty() => Err(
            "JavaScript has no packages; remove 'packages' or use language python".to_string(),
        ),
        Language::JavaScript => Ok(Interpreter::JavaScript),
        Language::Python if request.packages.is_empty() => Ok(Interpreter::Python),
        Language::Python => {
            if let Some(unknown) = request
                .packages
                .iter()
                .find(|p| canonical_package(p).is_none())
            {
                return Err(format!(
                    "Package '{unknown}' is not available; available: {}",
                    SCIENTIFIC_PACKAGES.join(", ")
                ));
            }
            Ok(Interpreter::Scientific)
        }
    }
}

async fn execute(prepared: &Prepared, request: SandboxRequest) -> Result<SandboxResponse> {
    let runtime = &prepared.runtime;
    let limits = &request.limits;
    let staged = files::validate_inputs(&request.files, limits.max_input_bytes)?;

    let network = guest_network(&request.network).await;

    let exec_dir = tempfile::Builder::new()
        .prefix("flow-sandbox-")
        .tempdir()
        .context("create sandbox directory")?;
    let root = exec_dir.path();
    files::stage(root, &request, &network, &prepared.scripts).await?;

    let module = runtime.compiled_module().await?;
    let run = runtime
        .run_guest(
            &module,
            root,
            &GuestLimits {
                timeout: limits.wall_time,
                memory_limit: limits.memory_bytes,
                fuel: limits.fuel,
            },
            &network,
            &prepared.spec,
        )
        .await?;

    let bootstrap = read_bootstrap(root).await;
    let collected = files::collect(root, &staged, limits).await;

    let (mut outputs, mut stdout, mut stderr, mut error, dropped) = match bootstrap {
        Some(b) => (
            b.outputs,
            b.stdout,
            b.stderr,
            b.error,
            (b.stdout_dropped > 0, b.stderr_dropped > 0),
        ),
        None => (
            Value::Object(Default::default()),
            run.stdout,
            run.stderr,
            None,
            (false, false),
        ),
    };
    if !outputs.is_object() {
        outputs = Value::Object(Default::default());
    }

    let status = match run.outcome {
        GuestOutcome::TimedOut => {
            error.get_or_insert_with(|| {
                format!(
                    "Execution timed out after {:.1}s",
                    limits.wall_time.as_secs_f64()
                )
            });
            SandboxStatus::Timeout
        }
        GuestOutcome::OutOfFuel => {
            error.get_or_insert_with(|| "Execution exhausted its fuel budget".to_string());
            SandboxStatus::FuelExhausted
        }
        GuestOutcome::Failed(msg) => {
            error.get_or_insert(msg);
            SandboxStatus::Error
        }
        GuestOutcome::Exited => match &error {
            None if root.join("outputs.json").exists() => SandboxStatus::Ok,
            None => {
                error = Some(format!(
                    "The {} bootstrap did not produce outputs; the program exited abnormally",
                    request.language.id()
                ));
                SandboxStatus::Error
            }
            Some(e) if e.contains("MemoryError") || e.contains("out of memory") => {
                SandboxStatus::OutOfMemory
            }
            Some(_) => SandboxStatus::Error,
        },
    };

    let stdout_cut = truncate_utf8(&mut stdout, limits.max_output_bytes);
    let stderr_cut = truncate_utf8(&mut stderr, limits.max_output_bytes);
    if let Some(e) = error.as_mut() {
        truncate_utf8(e, limits.max_output_bytes);
    }

    Ok(SandboxResponse {
        status,
        outputs,
        stdout,
        stderr,
        error,
        files: collected.files,
        truncated: Truncation {
            stdout: stdout_cut || dropped.0,
            stderr: stderr_cut || dropped.1,
            files: collected.truncated,
        },
        duration_ms: 0,
    })
}

async fn guest_network(policy: &NetworkPolicy) -> GuestNetwork {
    match policy {
        NetworkPolicy::AllowHosts(hosts) if !hosts.is_empty() => {
            GuestNetwork::resolve_hosts(hosts).await
        }
        NetworkPolicy::Off | NetworkPolicy::AllowHosts(_) => GuestNetwork::Off,
    }
}

async fn read_bootstrap(root: &Path) -> Option<BootstrapResult> {
    let bytes = tokio::fs::read(root.join("outputs.json")).await.ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn truncate_utf8(text: &mut String, max_bytes: usize) -> bool {
    if text.len() <= max_bytes {
        return false;
    }
    let mut cut = max_bytes;
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    text.truncate(cut);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn network_is_off_unless_hosts_are_listed() {
        assert_eq!(guest_network(&NetworkPolicy::Off).await, GuestNetwork::Off);
        assert_eq!(
            guest_network(&NetworkPolicy::AllowHosts(vec![])).await,
            GuestNetwork::Off
        );
        let allowed = guest_network(&NetworkPolicy::AllowHosts(vec!["93.184.216.34".into()])).await;
        assert!(matches!(allowed, GuestNetwork::Hosts(ref ips) if ips.len() == 1));
    }

    #[test]
    fn truncation_keeps_utf8_boundaries() {
        let mut text = "aé".repeat(10);
        assert!(truncate_utf8(&mut text, 4));
        assert_eq!(text, "aéa");
        assert!(!truncate_utf8(&mut text, 100));
    }
}

//! Build script for the code-interpreter crate.
//!
//! When the `bundled-python` feature is active this script:
//! 1. Locates a `python.wasm` binary (env var → local cache → download)
//! 2. AOT-compiles it via `Engine::precompile_module`
//! 3. Writes the `.cwasm` to `$OUT_DIR` for `include_bytes!` at compile time
//!
//! The wasmtime `Engine` configuration used here **must** match the runtime
//! configuration in `PyodideRuntime::new()`:
//!   - `consume_fuel(false)` — the interpreter uses epoch-based timeouts only
//!   - All other settings match `WasmConfig::production()` defaults
//!
//! ## Locating `python.wasm`
//!
//! 1. `PYTHON_WASM_PATH` environment variable (CI override)
//! 2. Local cache (`~/.cache/flow-like/python-wasm/{VERSION}/python.wasm`)
//! 3. Auto-download from GitHub (CPython WASI build, cached for future builds)
//!
//! Set `PYTHON_WASM_SKIP_DOWNLOAD=1` to disable auto-download.

fn main() {
    println!("cargo::rustc-check-cfg=cfg(has_bundled_python)");

    #[cfg(feature = "bundled-python")]
    bundled_python::compile();
}

#[cfg(feature = "bundled-python")]
mod bundled_python {
    use std::path::PathBuf;

    /// Pinned CPython WASI release from vmware-labs/webassembly-language-runtimes.
    const PYTHON_VERSION: &str = "3.12.0";
    const RELEASE_TAG: &str = "20231211-040d5a6";
    /// Direct .wasm download URL (WASI, wasi-sdk-20.0 build).
    const DOWNLOAD_URL: &str = "https://github.com/vmware-labs/webassembly-language-runtimes/releases/download/python%2F3.12.0%2B20231211-040d5a6/python-3.12.0.wasm";
    /// Expected blake3 hash of the downloaded binary — acts as integrity check.
    /// Set to empty string to skip verification (e.g. when bumping versions).
    const EXPECTED_BLAKE3: &str =
        "8a08d95f3e35e0a5a638fa038ed3e61def9cd1c47df9af517450d79252b12c26";

    pub fn compile() {
        println!("cargo:rerun-if-env-changed=PYTHON_WASM_PATH");
        println!("cargo:rerun-if-env-changed=PYTHON_WASM_SKIP_DOWNLOAD");

        let wasm_path = locate_or_download();

        let wasm_bytes = std::fs::read(&wasm_path).unwrap_or_else(|e| {
            panic!(
                "bundled-python: failed to read {}: {e}",
                wasm_path.display()
            );
        });

        if !EXPECTED_BLAKE3.is_empty() {
            let actual = blake3::hash(&wasm_bytes).to_hex().to_string();
            if actual != EXPECTED_BLAKE3 {
                panic!(
                    "bundled-python: blake3 mismatch for {}\n  expected: {}\n  actual:   {}",
                    wasm_path.display(),
                    EXPECTED_BLAKE3,
                    actual,
                );
            }
        }

        eprintln!(
            "bundled-python: AOT-compiling {} ({} bytes)…",
            wasm_path.display(),
            wasm_bytes.len(),
        );

        let engine = build_engine();
        let serialized = engine.precompile_module(&wasm_bytes).unwrap_or_else(|e| {
            panic!("bundled-python: failed to AOT-compile python.wasm: {e}");
        });

        let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
        let cwasm_path = out_dir.join("python.cwasm");
        std::fs::write(&cwasm_path, &serialized).unwrap_or_else(|e| {
            panic!(
                "bundled-python: failed to write {}: {e}",
                cwasm_path.display()
            );
        });

        println!("cargo:rustc-cfg=has_bundled_python");
        eprintln!(
            "bundled-python: {} → {} ({} bytes)",
            wasm_path.display(),
            cwasm_path.display(),
            serialized.len(),
        );
    }

    fn locate_or_download() -> PathBuf {
        // 1. Explicit env var (CI override)
        if let Ok(p) = std::env::var("PYTHON_WASM_PATH") {
            let path = PathBuf::from(p);
            if path.exists() {
                eprintln!("bundled-python: using PYTHON_WASM_PATH={}", path.display());
                return path;
            }
            panic!(
                "bundled-python: PYTHON_WASM_PATH={} does not exist",
                path.display()
            );
        }

        // 2. Local cache
        let cache_dir = cache_directory();
        let cached = cache_dir.join("python.wasm");
        if cached.exists() {
            eprintln!("bundled-python: using cached {}", cached.display());
            return cached;
        }

        // 3. Auto-download
        if std::env::var("PYTHON_WASM_SKIP_DOWNLOAD").is_ok() {
            panic!(
                "bundled-python: python.wasm not found and PYTHON_WASM_SKIP_DOWNLOAD is set.\n\
                 Either provide PYTHON_WASM_PATH or remove PYTHON_WASM_SKIP_DOWNLOAD."
            );
        }

        eprintln!("bundled-python: downloading CPython {PYTHON_VERSION} WASI from GitHub…");
        download_to(&cached);
        cached
    }

    fn download_to(dest: &PathBuf) {
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).unwrap_or_else(|e| {
                panic!(
                    "bundled-python: cannot create cache dir {}: {e}",
                    parent.display()
                );
            });
        }

        let agent = ureq::Agent::new_with_defaults();

        let resp = agent.get(DOWNLOAD_URL).call().unwrap_or_else(|e| {
            panic!("bundled-python: failed to download {DOWNLOAD_URL}: {e}");
        });

        let body = resp
            .into_body()
            .with_config()
            .limit(100 * 1024 * 1024) // 100 MB
            .read_to_vec()
            .unwrap_or_else(|e| {
                panic!("bundled-python: failed to read response body: {e}");
            });

        // Write atomically via a temp file to avoid partial downloads on interrupt
        let tmp = dest.with_extension("wasm.tmp");
        std::fs::write(&tmp, &body).unwrap_or_else(|e| {
            panic!("bundled-python: failed to write {}: {e}", tmp.display());
        });
        std::fs::rename(&tmp, dest).unwrap_or_else(|e| {
            panic!("bundled-python: failed to rename temp file: {e}");
        });

        eprintln!(
            "bundled-python: downloaded {} bytes → {}",
            body.len(),
            dest.display(),
        );
    }

    fn cache_directory() -> PathBuf {
        // ~/.cache/flow-like/python-wasm/{version}-{tag}/
        let base = dirs_next::cache_dir().unwrap_or_else(|| PathBuf::from(".cache"));
        base.join("flow-like")
            .join("python-wasm")
            .join(format!("{}-{}", PYTHON_VERSION, RELEASE_TAG))
    }

    /// Build a wasmtime Engine whose config matches `PyodideRuntime::new()`.
    fn build_engine() -> wasmtime::Engine {
        let mut config = wasmtime::Config::new();
        config.parallel_compilation(true);
        config.cranelift_opt_level(wasmtime::OptLevel::Speed);

        config.consume_fuel(false);
        config.epoch_interruption(true);

        config.wasm_gc(true);
        config.wasm_exceptions(true);
        config.wasm_function_references(true);
        config.wasm_simd(true);
        config.wasm_relaxed_simd(true);
        config.wasm_wide_arithmetic(true);
        config.memory_init_cow(true);

        wasmtime::Engine::new(&config).expect("bundled-python: failed to create wasmtime Engine")
    }
}

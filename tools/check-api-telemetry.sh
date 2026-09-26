#!/usr/bin/env bash
set -euo pipefail

# Compile the real request/body, span sink and Lambda export modules without
# linking the API catalog. Dependencies come from the workspace lock.
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
CHECK_DIR="$(mktemp -d "${TMPDIR:-/tmp}/flow-like-api-telemetry.XXXXXX")"
CHECK_TARGET="${FLOW_LIKE_TELEMETRY_TEST_TARGET_DIR:-${TMPDIR:-/tmp}/flow-like-api-telemetry-target}"
trap 'rm -rf "$CHECK_DIR"' EXIT

python3 - "$ROOT_DIR" "$CHECK_DIR" <<'PY'
import json
import re
import shutil
import sys
import tomllib
from pathlib import Path

root, check = map(Path, sys.argv[1:])
lock = tomllib.loads((root / "Cargo.lock").read_text())

def dependency(name, prefix, *, features=(), optional=False, default_features=True):
    packages = [p for p in lock["package"] if p["name"] == name and p["version"].startswith(prefix)]
    if len(packages) != 1:
        raise SystemExit(f"Expected one locked {name} {prefix} package, found {len(packages)}")
    package = packages[0]
    fields = [f'version = "={package["version"]}"']
    if package.get("source", "").startswith("git+"):
        repository, revision = package["source"][4:].split("#", 1)
        fields.extend(["git = " + json.dumps(repository.split("?", 1)[0]), "rev = " + json.dumps(revision)])
    if features:
        fields.append("features = " + json.dumps(features))
    if optional:
        fields.append("optional = true")
    if not default_features:
        fields.append("default-features = false")
    return f"{name} = {{ {', '.join(fields)} }}"

manifest = '''[package]
name = "flow-like-api-telemetry-check"
version = "0.1.0"
edition = "2024"
[features]
default = ["otel"]
otel = ["dep:opentelemetry", "dep:tracing-opentelemetry", "dep:opentelemetry_sdk", "dep:opentelemetry-otlp", "dep:lambda_http"]
[dependencies]
'''
manifest += "\n".join([
    dependency("axum", "0.8."),
    dependency("bytes", "1."),
    dependency("futures", "0.3."),
    dependency("hyper", "1."),
    dependency("serde_json", "1."),
    dependency("serde", "1.", features=["derive"]),
    dependency("sea-orm", "2.", features=["sqlx-postgres", "runtime-tokio-rustls", "macros", "with-json"]),
    dependency("chrono", "0.4.", features=["serde"]),
    dependency("rand", "0.9."),
    dependency("hex", "0.4."),
    dependency("blake3", "1."),
    dependency("cuid2", ""),
    dependency("tokio", "1.", features=["macros", "rt", "rt-multi-thread", "time", "sync"]),
    dependency("tower", "0.5.", features=["util"]),
    dependency("tracing", "0.1."),
    dependency("tracing-subscriber", "0.3.", features=["env-filter", "json"]),
    dependency("opentelemetry", "0.32.", optional=True),
    dependency("tracing-opentelemetry", "0.33.", optional=True),
    dependency("opentelemetry_sdk", "0.32.", optional=True),
    dependency("opentelemetry-otlp", "0.32.", features=["grpc-tonic", "trace"], optional=True, default_features=False),
    dependency("lambda_http", "0.15.", optional=True),
]) + "\n"
(check / "Cargo.toml").write_text(manifest)
shutil.copyfile(root / "Cargo.lock", check / "Cargo.lock")
(check / "src").mkdir()

# Import the real span sink, including its test-only methods and visibility.
# Only AppState's unrelated startup configuration is replaced; no telemetry API
# is re-declared here. Tests do not spawn the database writer or open a database.
telemetry_source = (root / "packages/api/src/telemetry/mod.rs").read_text()
telemetry_helpers = []
for name in ("sink_from_env", "trace_sample_rate_from_env", "parse_sample_rate"):
    matches = re.findall(rf"(?ms)^pub(?:\(crate\))? fn {name}\(.*?^\}}", telemetry_source)
    if len(matches) != 1:
        raise SystemExit(f"Expected one real telemetry helper: {name}")
    telemetry_helpers.extend(matches)
constants = [
    ("DEFAULT_TRACE_SAMPLE_RATE", telemetry_source),
    ("DEFAULT_WRITE_CHUNK", (root / "packages/db/src/batch.rs").read_text()),
]
constant_source = {}
for name, contents in constants:
    matches = re.findall(rf"(?m)^(?:pub )?const {name}: [^;]+;", contents)
    if len(matches) != 1:
        raise SystemExit(f"Expected one real telemetry constant: {name}")
    constant_source[name] = matches[0]

source = r'''
extern crate self as flow_like_types;
extern crate self as flow_like_api;
pub use tokio;
pub use serde_json::Value;
pub use cuid2::create_id;

pub fn warn_env_filter() -> tracing_subscriber::EnvFilter {
    tracing_subscriber::EnvFilter::new("warn")
}

pub mod state {
    pub type AppState = std::sync::Arc<State>;
    pub struct State {
        pub db: sea_orm::DatabaseConnection,
        pub platform_config: PlatformConfig,
    }
    pub struct PlatformConfig { pub features: Features }
    pub struct Features { pub telemetry: bool }
}
pub mod db {
    WRITE_CHUNK_SOURCE
}
pub mod entity {
    #[path = TELEMETRY_ENTITY_PATH]
    pub mod telemetry_span;
}
pub mod telemetry {
    SAMPLE_RATE_SOURCE
    TELEMETRY_HELPER_SOURCE
    #[path = SPANS_PATH]
    pub mod spans;
    #[path = REQUEST_METRICS_PATH]
    pub mod request_metrics;
}
pub mod middleware {
    #[path = TRACE_CONTEXT_PATH]
    pub mod trace_context;
}
#[cfg(feature = "otel")]
#[path = LAMBDA_TELEMETRY_PATH]
pub mod aws_telemetry;
'''
for token, relative in {
    "SPANS_PATH": "packages/api/src/telemetry/spans.rs",
    "TELEMETRY_ENTITY_PATH": "packages/api/entity/src/generated/telemetry_span.rs",
    "REQUEST_METRICS_PATH": "packages/api/src/telemetry/request_metrics.rs",
    "TRACE_CONTEXT_PATH": "packages/api/src/middleware/trace_context.rs",
    "LAMBDA_TELEMETRY_PATH": "apps/backend/aws/api/src/telemetry.rs",
}.items():
    source = source.replace(token, json.dumps(str(root / relative)))
source = source.replace("WRITE_CHUNK_SOURCE", constant_source["DEFAULT_WRITE_CHUNK"])
source = source.replace("SAMPLE_RATE_SOURCE", constant_source["DEFAULT_TRACE_SAMPLE_RATE"])
source = source.replace("TELEMETRY_HELPER_SOURCE", "\n".join(telemetry_helpers))
(check / "src/lib.rs").write_text(source)
PY

cargo --config 'build.rustc-wrapper=""' test --offline \
    --manifest-path "$CHECK_DIR/Cargo.toml" --target-dir "$CHECK_TARGET" "$@"
cargo --config 'build.rustc-wrapper=""' test --offline --no-default-features \
    --manifest-path "$CHECK_DIR/Cargo.toml" --target-dir "$CHECK_TARGET" "$@"

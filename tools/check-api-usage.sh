#!/usr/bin/env bash
set -euo pipefail

# Exercise the real usage queries and generated entities without linking the catalog.
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
CHECK_DIR="$(mktemp -d "${TMPDIR:-/tmp}/flow-like-api-usage.XXXXXX")"
CHECK_TARGET="${FLOW_LIKE_USAGE_TEST_TARGET_DIR:-${TMPDIR:-/tmp}/flow-like-api-telemetry-target}"
trap 'rm -rf "$CHECK_DIR"' EXIT

python3 - "$ROOT_DIR" "$CHECK_DIR" <<'PY'
import json
import shutil
import sys
import tomllib
from pathlib import Path

root, check = map(Path, sys.argv[1:])
lock = tomllib.loads((root / "Cargo.lock").read_text())
manifest = '''[package]
name = "flow-like-api-usage-check"
version = "0.1.0"
edition = "2024"
[dependencies]
'''
for name, prefix, features in [
    ("axum", "0.8.", []),
    ("anyhow", "1.", []),
    ("serde", "1.", ["derive"]),
    ("chrono", "0.4.", ["serde"]),
    ("sea-orm", "2.", ["sqlx-postgres", "sqlx-sqlite", "runtime-tokio-rustls", "macros", "with-json"]),
    ("tokio", "1.", ["macros", "rt-multi-thread"]),
    ("tracing", "0.1.", []),
    ("utoipa", "5.", ["axum_extras", "chrono"]),
]:
    packages = [p for p in lock["package"] if p["name"] == name and p["version"].startswith(prefix)]
    if len(packages) != 1:
        raise SystemExit(f"Expected one locked {name} {prefix} package")
    manifest += f'{name} = {{ version = "={packages[0]["version"]}", features = {json.dumps(features)} }}\n'
manifest += 'flow-like-api-entity = { path = ' + json.dumps(str(root / "packages/api/entity")) + ', default-features = false }\n'
(check / "Cargo.toml").write_text(manifest)
shutil.copyfile(root / "Cargo.lock", check / "Cargo.lock")
(check / "src").mkdir()
source = r'''
extern crate self as flow_like_types;
pub use tokio;
pub use flow_like_api_entity as entity;

pub mod state {
    pub type AppState = std::sync::Arc<State>;
    pub struct State { pub db: sea_orm::DatabaseConnection }
}
pub mod error {
    #[derive(Debug)]
    pub struct ApiError;
    impl ApiError {
        pub const FORBIDDEN: Self = Self;
        pub fn internal_error(error: anyhow::Error) -> Self { panic!("{error:#}") }
    }
    impl From<anyhow::Error> for ApiError {
        fn from(error: anyhow::Error) -> Self { Self::internal_error(error) }
    }
    impl From<sea_orm::DbErr> for ApiError {
        fn from(error: sea_orm::DbErr) -> Self { Self::internal_error(error.into()) }
    }
}
pub mod middleware {
    pub mod jwt {
        pub struct AppUser;
        impl AppUser {
            pub fn sub(&self) -> anyhow::Result<String> { unreachable!("queries receive an authenticated subject") }
        }
    }
}
pub mod utils {
    #[path = TIME_PATH]
    pub mod time;
    #[path = PERIOD_PATH]
    pub mod stats_period;
}
pub mod usage {
    #[path = HISTORY_PATH]
    pub mod history;
    #[path = ACTIVITY_PATH]
    pub mod activity;
    #[cfg(test)]
    #[path = FIXTURES_PATH]
    pub mod test_db;
}
'''
for token, relative in {
    "TIME_PATH": "packages/api/src/utils/time.rs",
    "PERIOD_PATH": "packages/api/src/utils/stats_period.rs",
    "HISTORY_PATH": "packages/api/src/routes/usage/history.rs",
    "ACTIVITY_PATH": "packages/api/src/routes/usage/activity.rs",
    "FIXTURES_PATH": "packages/api/src/routes/usage/test_db.rs",
}.items():
    source = source.replace(token, json.dumps(str(root / relative)))
(check / "src/lib.rs").write_text(source)
PY

cargo --config 'build.rustc-wrapper=""' test --offline \
    --manifest-path "$CHECK_DIR/Cargo.toml" --target-dir "$CHECK_TARGET" "$@"

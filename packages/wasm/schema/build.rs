use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn find_workspace_manifest(start: &Path) -> PathBuf {
    start
        .ancestors()
        .map(|dir| dir.join("Cargo.toml"))
        .find(|candidate| {
            fs::read_to_string(candidate)
                .map(|content| content.lines().any(|line| line.trim() == "[workspace]"))
                .unwrap_or(false)
        })
        .expect("failed to find the workspace Cargo.toml")
}

fn wasmtime_major_version(workspace_toml: &str) -> &str {
    let version = workspace_toml
        .lines()
        .find_map(|line| {
            let line = line.trim();
            let value = line.strip_prefix("wasmtime")?.trim_start();
            if !value.starts_with('=') {
                return None;
            }
            let version = value.split_once("version")?.1;
            let quoted = version.split_once('"')?.1;
            quoted.split_once('"').map(|(version, _)| version)
        })
        .expect("could not find the workspace wasmtime dependency version");

    version
        .split('.')
        .next()
        .filter(|major| !major.is_empty())
        .expect("could not extract the Wasmtime major version")
}

fn main() {
    let manifest_dir = PathBuf::from(
        env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is always set by Cargo"),
    );
    let workspace_manifest = find_workspace_manifest(&manifest_dir);
    let workspace_toml =
        fs::read_to_string(&workspace_manifest).expect("failed to read workspace Cargo.toml");
    let major = wasmtime_major_version(&workspace_toml);

    println!("cargo:rerun-if-changed={}", workspace_manifest.display());
    println!("cargo:rustc-env=FLOW_LIKE_WASMTIME_MAJOR_VERSION={major}");
}

#[cfg(test)]
mod tests {
    use super::wasmtime_major_version;

    #[test]
    fn extracts_only_the_wasmtime_package_version() {
        let manifest = r#"
            wasmtime = { version = "47.0.3", default-features = false }
            wasmtime-wasi = "47.0.3"
        "#;
        assert_eq!(wasmtime_major_version(manifest), "47");
    }
}

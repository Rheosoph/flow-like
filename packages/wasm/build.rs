use std::fs;

fn main() {
    println!("cargo:rerun-if-changed=../../Cargo.toml");

    let workspace_toml =
        fs::read_to_string("../../Cargo.toml").expect("failed to read workspace Cargo.toml");

    // Find: wasmtime = { version = "XX.YY.ZZ", ... }
    let full_version = workspace_toml
        .lines()
        .find_map(|line| {
            let trimmed = line.trim();
            if trimmed.starts_with("wasmtime") && trimmed.contains("version") {
                // Extract the version string between quotes after `version = "`
                let after_version = trimmed.split("version").nth(1)?;
                let start = after_version.find('"')? + 1;
                let rest = &after_version[start..];
                let end = rest.find('"')?;
                Some(rest[..end].to_string())
            } else {
                None
            }
        })
        .expect("could not find wasmtime version in workspace Cargo.toml");

    let major_version = full_version
        .split('.')
        .next()
        .filter(|major| !major.is_empty())
        .expect("could not extract wasmtime major version from workspace Cargo.toml");

    println!("cargo:rustc-env=WASMTIME_MAJOR_VERSION={major_version}");
    // Backwards-compatible env name; this intentionally carries the artifact
    // compatibility version, not the full Cargo package version.
    println!("cargo:rustc-env=WASMTIME_VERSION={major_version}");
}

use std::env;

fn main() {
    let automatic = enabled("TRAINING_AUTO");
    let os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    let pc = arch == "x86_64"
        && matches!(
            (os.as_str(), target_env.as_str()),
            ("linux", "gnu") | ("windows", "msvc")
        );
    let desktop = matches!(os.as_str(), "linux" | "windows" | "macos")
        && matches!(arch.as_str(), "x86_64" | "aarch64");

    // Keep these targets aligned with the optional flow-like-ml-platform dependency.
    for (backend, compiled) in [
        ("cpu", enabled("CPU") || automatic),
        ("wgpu", enabled("WGPU") || automatic && desktop),
        ("cuda", enabled("CUDA") || automatic && pc),
        ("rocm", enabled("ROCM") || automatic && pc),
    ] {
        println!("cargo:rustc-check-cfg=cfg(ml_backend_{backend})");
        if compiled {
            println!("cargo:rustc-cfg=ml_backend_{backend}");
        }
    }
}

fn enabled(feature: &str) -> bool {
    env::var_os(format!("CARGO_FEATURE_{feature}")).is_some()
}

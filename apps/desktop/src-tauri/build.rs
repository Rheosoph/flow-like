use std::{fs, path::PathBuf};

fn ensure_ios_deeplink_scheme(scheme: &str) -> Result<(), String> {
    let mut plist_paths = vec![PathBuf::from("Info.plist")];

    if let Ok(entries) = fs::read_dir("gen/apple") {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            let dir_name = entry.file_name().to_string_lossy().to_string();
            if !dir_name.ends_with("_iOS") {
                continue;
            }

            let plist_path = path.join("Info.plist");
            if plist_path.exists() {
                plist_paths.push(plist_path);
            }
        }
    }

    for plist_path in plist_paths {
        let mut value = plist::Value::from_file(&plist_path)
            .map_err(|e| format!("{}: failed to parse plist: {e}", plist_path.display()))?;

        let root = value.as_dictionary_mut().ok_or_else(|| {
            format!(
                "{}: expected root dictionary in plist",
                plist_path.display()
            )
        })?;

        let mut changed = false;
        if !root.contains_key("CFBundleURLTypes") {
            root.insert(
                "CFBundleURLTypes".to_string(),
                plist::Value::Array(Vec::new()),
            );
            changed = true;
        }

        let url_types = root
            .get_mut("CFBundleURLTypes")
            .and_then(plist::Value::as_array_mut)
            .ok_or_else(|| {
                format!(
                    "{}: CFBundleURLTypes exists but is not an array",
                    plist_path.display()
                )
            })?;

        let original_url_type_count = url_types.len();
        url_types.retain(|entry| {
            entry
                .as_dictionary()
                .and_then(|dict| dict.get("CFBundleURLSchemes"))
                .and_then(plist::Value::as_array)
                .is_none_or(|schemes| !schemes.is_empty())
        });
        if url_types.len() != original_url_type_count {
            changed = true;
        }

        let has_scheme = url_types.iter().any(|entry| {
            entry
                .as_dictionary()
                .and_then(|dict| dict.get("CFBundleURLSchemes"))
                .and_then(plist::Value::as_array)
                .is_some_and(|schemes| schemes.iter().any(|item| item.as_string() == Some(scheme)))
        });

        if !has_scheme {
            let mut url_type = plist::Dictionary::new();
            url_type.insert(
                "CFBundleURLName".to_string(),
                plist::Value::String(scheme.to_string()),
            );
            url_type.insert(
                "CFBundleURLSchemes".to_string(),
                plist::Value::Array(vec![plist::Value::String(scheme.to_string())]),
            );
            url_types.push(plist::Value::Dictionary(url_type));
            changed = true;
        }

        if changed {
            value
                .to_file_xml(&plist_path)
                .map_err(|e| format!("{}: failed to write plist: {e}", plist_path.display()))?;
        }
    }

    Ok(())
}

fn ensure_ios_associated_domain(domain: &str) -> Result<(), String> {
    let normalized_domain = domain
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/')
        .to_string();

    if normalized_domain.is_empty() {
        return Err("associated domain is empty".to_string());
    }

    let required_entry = format!("applinks:{normalized_domain}");
    let mut entitlements_paths: Vec<PathBuf> = Vec::new();

    if let Ok(entries) = fs::read_dir("gen/apple") {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            let dir_name = entry.file_name().to_string_lossy().to_string();
            if !dir_name.ends_with("_iOS") {
                continue;
            }

            if let Ok(files) = fs::read_dir(path) {
                for file in files.flatten() {
                    let file_path = file.path();
                    let is_entitlements = file_path
                        .extension()
                        .and_then(|ext| ext.to_str())
                        .is_some_and(|ext| ext == "entitlements");
                    if is_entitlements {
                        entitlements_paths.push(file_path);
                    }
                }
            }
        }
    }

    for entitlements_path in entitlements_paths {
        let mut value = plist::Value::from_file(&entitlements_path).map_err(|e| {
            format!(
                "{}: failed to parse entitlements plist: {e}",
                entitlements_path.display()
            )
        })?;

        let root = value.as_dictionary_mut().ok_or_else(|| {
            format!(
                "{}: expected root dictionary in entitlements plist",
                entitlements_path.display()
            )
        })?;

        let mut changed = false;
        if !root.contains_key("com.apple.developer.associated-domains") {
            root.insert(
                "com.apple.developer.associated-domains".to_string(),
                plist::Value::Array(Vec::new()),
            );
            changed = true;
        }

        let domains = root
            .get_mut("com.apple.developer.associated-domains")
            .and_then(plist::Value::as_array_mut)
            .ok_or_else(|| {
                format!(
                    "{}: associated domains exists but is not an array",
                    entitlements_path.display()
                )
            })?;

        let already_present = domains
            .iter()
            .any(|item| item.as_string() == Some(required_entry.as_str()));

        if !already_present {
            domains.push(plist::Value::String(required_entry.clone()));
            changed = true;
        }

        if changed {
            value.to_file_xml(&entitlements_path).map_err(|e| {
                format!(
                    "{}: failed to write entitlements plist: {e}",
                    entitlements_path.display()
                )
            })?;
        }
    }

    Ok(())
}

fn emit_workflow_benchmark_build_identity() {
    fn collect(dir: &std::path::Path, files: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(dir).expect("benchmark source directory exists") {
            let entry = entry.expect("benchmark source entry is readable");
            let kind = entry
                .file_type()
                .expect("benchmark source type is readable");
            let path = entry.path();
            if kind.is_dir() {
                if !matches!(
                    entry.file_name().to_str(),
                    Some("node_modules" | "target" | ".git")
                ) {
                    collect(&path, files);
                }
            } else if kind.is_file()
                && matches!(
                    path.extension().and_then(|ext| ext.to_str()),
                    Some("rs" | "toml")
                )
            {
                files.push(path);
            }
        }
    }
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../..");
    let mut files = Vec::new();
    for path in ["packages", "apps/desktop/src-tauri/src"] {
        collect(&root.join(path), &mut files);
    }
    for path in [
        "Cargo.toml",
        "Cargo.lock",
        "apps/desktop/src-tauri/Cargo.toml",
        "apps/desktop/src-tauri/build.rs",
    ] {
        let path = root.join(path);
        println!("cargo:rerun-if-changed={}", path.display());
        files.push(path);
    }
    files.sort();
    let mut hash = blake3::Hasher::new();
    hash.update(b"flowpilot.workflow-benchmark-build/v1\n");
    for path in files {
        println!("cargo:rerun-if-changed={}", path.display());
        let relative = path.strip_prefix(&root).unwrap().to_string_lossy();
        let content = fs::read(&path).expect("benchmark source is readable");
        hash.update(&(relative.len() as u64).to_le_bytes());
        hash.update(relative.as_bytes());
        hash.update(&(content.len() as u64).to_le_bytes());
        hash.update(&content);
    }
    let mut build_settings: Vec<_> = std::env::vars()
        .filter(|(key, _)| {
            key.starts_with("CARGO_FEATURE_")
                || matches!(
                    key.as_str(),
                    "TARGET" | "PROFILE" | "CARGO_ENCODED_RUSTFLAGS"
                )
        })
        .collect();
    build_settings.sort();
    hash.update(&serde_json::to_vec(&build_settings).unwrap());
    println!(
        "cargo:rustc-env=FLOWPILOT_BENCHMARK_BUILD_ID={}",
        hash.finalize().to_hex()
    );
}

fn main() {
    emit_workflow_benchmark_build_identity();
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();

    if target_os == "macos" {
        for path in [
            "apple/FlowLikeNative/Sources",
            "apple/FlowLikeNative/Extensions",
            "apple/FlowLikeNative/Package.swift",
            "apple/FlowLikeNative/FlowLikeNative.xcodeproj/project.pbxproj",
        ] {
            println!("cargo:rerun-if-changed={path}");
        }
        println!("cargo:rerun-if-changed=../scripts/prepare-native-apple.ts");
        println!("cargo:rerun-if-changed=../scripts/apple-bundle-version.py");
        println!("cargo:rerun-if-changed=tauri.conf.json");
        println!("cargo:rerun-if-changed=tauri.macos.conf.json");
        println!("cargo:rerun-if-env-changed=TAURI_CONFIG");
        println!("cargo:rerun-if-env-changed=APPLE_SIGNING_IDENTITY");
        let mut command = std::process::Command::new("bun");
        command.arg("../scripts/prepare-native-apple.ts");
        if std::env::var("PROFILE").as_deref() == Ok("release") {
            command.arg("--release");
        }
        let status = command
            .status()
            .expect("bun is required to build Apple native integrations");
        assert!(status.success(), "Apple native integration build failed");
        let frameworks = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap())
            .join("binaries/apple-native");
        println!("cargo:rustc-link-search=framework={}", frameworks.display());
        println!("cargo:rustc-link-lib=framework=FlowLikeNative");
        println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path/../Frameworks");
        // `tauri dev` runs the binary outside an application bundle.
        println!("cargo:rustc-link-arg=-Wl,-rpath,{}", frameworks.display());
    }

    // Flow-Like's worker-heavy desktop runtime needs an 8 MiB main-thread
    // stack on Windows. Scope the option to the final MSVC desktop executable:
    // a global RUSTFLAGS value would rebuild every dependency with a different
    // fingerprint and `/STACK` is not valid for the Windows GNU linker.
    if target_os == "windows" && target_env == "msvc" {
        println!("cargo::rustc-link-arg-bin=flow-like-desktop=/STACK:8388608");
    }

    // Link against system zlib on iOS (needed by flate2 with zlib feature)
    if target_os == "ios" {
        println!("cargo:rustc-link-lib=z");
        println!("cargo:rustc-link-lib=framework=Accelerate");

        // Cargo also builds the cdylib crate type before Xcode links the final
        // static library into the app. The MLX C bridge lives in a SwiftPM
        // product that Xcode links later, so allow only those bridge symbols
        // to remain undefined in this intermediate artifact.
        for symbol in [
            "_flow_like_mlx_cancel",
            "_flow_like_mlx_clear_cache",
            "_flow_like_mlx_generate",
            "_flow_like_mlx_is_available",
            "_flow_like_mlx_unload",
            "_flow_like_native_publish_snapshot",
            "_flow_like_native_clear_snapshot",
            "_flow_like_native_take_actions",
            "_flow_like_native_pending_actions",
            "_flow_like_native_ack_action",
            "_flow_like_native_publish_app_icons",
            "_flow_like_native_complete_action",
            "_flow_like_native_free_string",
            "_flow_like_native_set_action_callback",
            "_flow_like_native_set_inactive_callback",
            "_flow_like_native_write_clipboard",
            "_flow_like_native_shared_files_directory",
            "_flow_like_native_get_location",
            "_flow_like_native_cancel_location",
            "_flow_like_native_set_location_background_callback",
            "_flow_like_native_scope",
            "_flow_like_native_sync_geofences",
            "_flow_like_native_geofence_permission",
            "_flow_like_native_set_geofence_callback",
            "_flow_like_native_set_geofence_expiration_callback",
            "_flow_like_native_geofence_background_time_remaining",
            "_flow_like_native_geofence_foreground",
            "_flow_like_native_take_geofence_events",
            "_flow_like_native_ack_geofence_events",
        ] {
            println!("cargo:rustc-link-arg-cdylib=-Wl,-U,{symbol}");
        }
    }

    if target_os == "android" {
        println!("cargo:rustc-link-lib=z");
        println!("cargo:rustc-link-lib=log");
    }

    println!("cargo:rerun-if-changed=../../../flow-like.config.json");
    println!("cargo:rerun-if-changed=../../../flow-like.config.prod.json");

    let cfg_str = fs::read_to_string("../../../flow-like.config.json").unwrap_or_default();
    let mut app_link_domain = String::from("app.flow-like.com");

    if let Ok(config) = serde_json::from_str::<serde_json::Value>(&cfg_str) {
        if let Some(domain) = config.get("domain").and_then(|d| d.as_str()) {
            println!("cargo:rustc-env=FLOW_LIKE_CONFIG_DOMAIN={}", domain);
        }
        if let Some(app_domain) = config.get("app").and_then(|d| d.as_str()) {
            app_link_domain = app_domain.to_string();
            println!("cargo:rustc-env=FLOW_LIKE_CONFIG_APP={}", app_domain);
        }
        if let Some(web_domain) = config.get("web").and_then(|d| d.as_str()) {
            println!("cargo:rustc-env=FLOW_LIKE_CONFIG_WEB={}", web_domain);
        }
        let secure = config
            .get("secure")
            .and_then(|s| s.as_bool())
            .unwrap_or(true);
        println!("cargo:rustc-env=FLOW_LIKE_CONFIG_SECURE={}", secure);
    }

    tauri_build::build();

    if target_os == "ios" {
        if let Err(err) = ensure_ios_deeplink_scheme("flow-like") {
            println!("cargo:warning=failed to enforce iOS deeplink scheme: {err}");
        }
        if let Err(err) = ensure_ios_associated_domain(&app_link_domain) {
            println!("cargo:warning=failed to enforce iOS associated domains: {err}");
        }
    }
}

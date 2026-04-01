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

fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();

    // Link against system zlib on iOS (needed by flate2 with zlib feature)
    if target_os == "ios" {
        println!("cargo:rustc-link-lib=z");
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

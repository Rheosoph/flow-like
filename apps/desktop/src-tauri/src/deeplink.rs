use flow_like_types::json;
use tauri::{AppHandle, Url};

pub fn handle_deep_link(app_handle: &AppHandle, urls: &Vec<Url>) {
    #[cfg(desktop)]
    {
        use tauri::Manager;

        if let Some(window) = app_handle.get_webview_window("main") {
            if !window.is_visible().unwrap_or(false) {
                let _ = window.show();
            }

            if window.is_minimized().unwrap_or(false) {
                let _ = window.unminimize();
            }

            let _ = window.set_focus();
        }
    }

    for url in urls {
        println!("Deep link URL: {}", url);

        // Handle file URLs first (iOS 'Open in...' / AirDrop of documents)
        if url.scheme() == "file" {
            // Convert to local path and notify UI to import
            if let Ok(path) = url.to_file_path() {
                let path_str = path.to_string_lossy().to_string();
                println!("Received file URL to import: {}", path_str);
                crate::utils::emit_throttled(
                    app_handle,
                    crate::utils::UiEmitTarget::All,
                    "import/file",
                    json::json!({ "path": path_str }),
                    std::time::Duration::from_millis(200),
                );
                continue;
            }
        }

        if is_app_universal_link(url) && handle_universal_link(app_handle, url) {
            continue;
        }

        if url.scheme() == "flow-like" {
            let command = url.host_str().unwrap_or_default();

            match command {
                "auth" => {
                    handle_auth(app_handle, url.as_str());
                }
                "thirdparty" => {
                    handle_thirdparty_callback(app_handle, url);
                }
                "trigger" => {
                    handle_trigger(app_handle, url);
                }
                "store" => {
                    handle_store(app_handle, url);
                }
                "join" => {
                    handle_join(app_handle, url);
                }
                _ => {
                    println!("Unknown deep link command: {}", command);
                }
            }
            continue;
        }

        println!("Unknown deep link URL: {}", url);
    }
}

fn is_app_universal_link(url: &Url) -> bool {
    if !(url.scheme() == "https" || url.scheme() == "http") {
        return false;
    }

    matches!(
        url.host_str(),
        Some("app.flow-like.com") | Some("localhost") | Some("127.0.0.1")
    )
}

fn handle_universal_link(app_handle: &AppHandle, url: &Url) -> bool {
    let path = url.path().trim_matches('/');

    match path {
        "callback" | "desktop/callback" => {
            handle_auth(app_handle, url.as_str());
            true
        }
        "thirdparty/callback" => {
            handle_thirdparty_callback(app_handle, url);
            true
        }
        _ if path.starts_with("trigger/") => {
            handle_trigger(app_handle, url);
            true
        }
        "store" => {
            handle_store(app_handle, url);
            true
        }
        _ if path.starts_with("store/") => {
            handle_store(app_handle, url);
            true
        }
        "join" => {
            handle_join(app_handle, url);
            true
        }
        _ => {
            println!("Unknown universal link path: {}", path);
            false
        }
    }
}

fn handle_auth(app_handle: &AppHandle, url: &str) {
    println!("Handling auth URL: {}", url);
    crate::utils::emit_throttled(
        app_handle,
        crate::utils::UiEmitTarget::All,
        "oidc/url",
        json::json!({ "url": url }),
        std::time::Duration::from_millis(200),
    );
}

fn handle_thirdparty_callback(app_handle: &AppHandle, url: &Url) {
    // Parse URL:
    // - flow-like://thirdparty/callback?code=...&state=...
    // - https://app.flow-like.com/thirdparty/callback?code=...&state=...
    // Supports both OAuth (code flow) and OIDC (implicit flow with id_token)
    let path = url.path().trim_matches('/');

    // Check if this is the callback path
    if path != "callback" && path != "thirdparty/callback" {
        println!("Unknown thirdparty path: {}", path);
        return;
    }

    // Parse query parameters (handles both query string and fragment params)
    let mut params = serde_json::Map::new();
    if let Some(query) = url.query() {
        for pair in query.split('&') {
            if let Some((key, value)) = pair.split_once('=') {
                let decoded_key = urlencoding::decode(key).unwrap_or_default().into_owned();
                let decoded_value = urlencoding::decode(value).unwrap_or_default().into_owned();
                params.insert(decoded_key, serde_json::Value::String(decoded_value));
            }
        }
    }

    // Also parse fragment (some OIDC providers return tokens in fragment)
    if let Some(fragment) = url.fragment() {
        for pair in fragment.split('&') {
            if let Some((key, value)) = pair.split_once('=') {
                let decoded_key = urlencoding::decode(key).unwrap_or_default().into_owned();
                let decoded_value = urlencoding::decode(value).unwrap_or_default().into_owned();
                // Don't overwrite query params with fragment params
                if !params.contains_key(&decoded_key) {
                    params.insert(decoded_key, serde_json::Value::String(decoded_value));
                }
            }
        }
    }

    println!(
        "Thirdparty OAuth/OIDC callback received with params: {:?}",
        params
    );

    // Emit the callback URL to the frontend for processing
    // Includes both OAuth (code) and OIDC (id_token, access_token) fields
    crate::utils::emit_throttled(
        app_handle,
        crate::utils::UiEmitTarget::All,
        "thirdparty/callback",
        json::json!({
            "url": url.as_str(),
            // OAuth Authorization Code flow
            "code": params.get("code"),
            "state": params.get("state"),
            // OIDC Implicit/Hybrid flow
            "id_token": params.get("id_token"),
            "access_token": params.get("access_token"),
            "token_type": params.get("token_type"),
            "expires_in": params.get("expires_in"),
            "scope": params.get("scope"),
            // Error handling
            "error": params.get("error"),
            "error_description": params.get("error_description")
        }),
        std::time::Duration::from_millis(200),
    );
}

fn handle_trigger(app_handle: &AppHandle, url: &Url) {
    // Parse URL:
    // - flow-like://trigger/{app_id}/{...route}?param1=value1&param2=value2
    // - https://app.flow-like.com/trigger/{app_id}/{...route}?param1=value1&param2=value2
    let raw_path = url.path().trim_matches('/');
    let path = raw_path.strip_prefix("trigger/").unwrap_or(raw_path);

    // Remove leading slash and split into parts
    let path_parts: Vec<&str> = path.split('/').collect();

    if path_parts.len() < 2 {
        println!("Invalid trigger URL: expected at least app_id and route");
        return;
    }

    let app_id = path_parts[0];
    let route = path_parts[1..].join("/");

    // Parse query parameters using Tauri's URL query method
    let query_params: serde_json::Value = if let Some(query) = url.query() {
        let mut params = serde_json::Map::new();
        // Parse query string manually
        for pair in query.split('&') {
            if let Some((key, value)) = pair.split_once('=') {
                // URL decode the values
                let decoded_key = urlencoding::decode(key).unwrap_or_default().into_owned();
                let decoded_value = urlencoding::decode(value).unwrap_or_default().into_owned();
                params.insert(decoded_key, serde_json::Value::String(decoded_value));
            }
        }
        serde_json::Value::Object(params)
    } else {
        serde_json::Value::Object(serde_json::Map::new())
    };

    println!(
        "Trigger deep link: app_id='{}', route='{}', params={:?}",
        app_id, route, query_params
    );

    // Call the deeplink sink handler
    match crate::event_sink::deeplink::DeeplinkSink::handle_trigger(
        app_handle,
        app_id,
        &route,
        query_params,
    ) {
        Ok(true) => {
            println!("✅ Deeplink event triggered successfully");
        }
        Ok(false) => {
            println!("⚠️ Deeplink event not triggered (offline or not found)");
        }
        Err(e) => {
            println!("❌ Failed to trigger deeplink event: {}", e);
        }
    }
}

fn handle_store(app_handle: &AppHandle, url: &Url) {
    // Parse URL:
    // - flow-like://store?id={app_id}
    // - https://app.flow-like.com/store?id={app_id}
    // - https://app.flow-like.com/store/{app_id}
    let mut app_id = url
        .query_pairs()
        .find(|(k, _)| k == "id")
        .map(|(_, v)| v.to_string());

    if app_id.is_none() {
        let path = url.path().trim_matches('/');
        if let Some(rest) = path.strip_prefix("store/")
            && !rest.is_empty()
        {
            app_id = Some(rest.to_string());
        }
    }

    println!("Store deep link: app_id={:?}", app_id);

    crate::utils::emit_throttled(
        app_handle,
        crate::utils::UiEmitTarget::All,
        "deeplink/store",
        json::json!({ "appId": app_id }),
        std::time::Duration::from_millis(200),
    );
}

fn handle_join(app_handle: &AppHandle, url: &Url) {
    // Parse URL:
    // - flow-like://join?appId={app_id}&token={token}
    // - https://app.flow-like.com/join?appId={app_id}&token={token}
    let params: std::collections::HashMap<_, _> = url.query_pairs().collect();
    let app_id = params.get("appId").map(|v| v.to_string());
    let token = params.get("token").map(|v| v.to_string());

    println!("Join deep link: app_id={:?}, token={:?}", app_id, token);

    crate::utils::emit_throttled(
        app_handle,
        crate::utils::UiEmitTarget::All,
        "deeplink/join",
        json::json!({ "appId": app_id, "token": token }),
        std::time::Duration::from_millis(200),
    );
}

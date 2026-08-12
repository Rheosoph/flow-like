use flow_like_types::json;
use tauri::{AppHandle, Url};

/// Host that serves the app itself. Universal links on this host are claimed by the
/// iOS entitlement / Android intent filter, so the OS routes them into the process.
const APP_HOST: &str = match option_env!("FLOW_LIKE_CONFIG_APP") {
    Some(host) => host,
    None => "app.flow-like.com",
};

/// Marketing host. It is not associated with the app today, but URLs on it can still reach
/// the process through the website's hand-off pages, so the parser accepts them.
const WEB_HOST: &str = match option_env!("FLOW_LIKE_CONFIG_WEB") {
    Some(host) => host,
    None => "flow-like.com",
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeepLinkIntent {
    ImportFile,
    Auth,
    Logout,
    ThirdParty,
    Trigger {
        app_id: String,
        route: String,
    },
    Store {
        app_id: Option<String>,
        package_id: Option<String>,
    },
    Join,
    Unknown,
}

pub fn handle_deep_link(app_handle: &AppHandle, urls: &Vec<Url>) {
    dispatch_deep_links(app_handle, urls, false);
}

/// `replayed` marks re-emissions of the launch URL (`get_current()` returns it
/// for the whole process lifetime). The frontend deduplicates replayed events
/// but always honors fresh ones, so clicking the same link twice works.
fn dispatch_deep_links(app_handle: &AppHandle, urls: &Vec<Url>, replayed: bool) {
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
        tracing::info!("Deep link URL: {}", url);

        match classify(url) {
            DeepLinkIntent::ImportFile => handle_import_file(app_handle, url),
            DeepLinkIntent::Auth | DeepLinkIntent::Logout => handle_auth(app_handle, url.as_str()),
            DeepLinkIntent::ThirdParty => handle_thirdparty_callback(app_handle, url),
            DeepLinkIntent::Trigger { app_id, route } => {
                handle_trigger(app_handle, url, &app_id, &route)
            }
            DeepLinkIntent::Store { app_id, package_id } => {
                tracing::info!(
                    "Store deep link: app_id={:?}, package_id={:?}",
                    app_id,
                    package_id
                );
                emit(
                    app_handle,
                    "deeplink/store",
                    json::json!({ "appId": app_id, "packageId": package_id, "replayed": replayed }),
                );
            }
            DeepLinkIntent::Join => handle_join(app_handle, url, replayed),
            DeepLinkIntent::Unknown => handle_unknown(app_handle, url),
        }
    }
}

/// Re-dispatch the URL the app was launched with.
///
/// Deep-link events are fire-and-forget: Tauri drops an emission for any webview that has no
/// listener registered for that event name yet. On a cold start the Rust side emits during
/// `setup()`, long before the frontend has mounted, so the navigation is lost. The frontend calls
/// this once its listeners are attached, which replays the launch URL through the same parser.
#[tauri::command]
pub fn deeplink_replay_pending(app_handle: AppHandle) {
    use tauri_plugin_deep_link::DeepLinkExt;

    match app_handle.deep_link().get_current() {
        Ok(Some(urls)) if !urls.is_empty() => {
            tracing::info!(
                "Replaying {} pending deep link(s) for the frontend",
                urls.len()
            );
            dispatch_deep_links(&app_handle, &urls, true);
        }
        Ok(_) => {}
        Err(error) => tracing::warn!("Failed to read pending deep links: {error}"),
    }
}

fn emit(app_handle: &AppHandle, event: &str, payload: serde_json::Value) {
    crate::utils::emit_to_ui(app_handle, event, payload);
}

fn classify(url: &Url) -> DeepLinkIntent {
    if url.scheme() == "file" {
        return DeepLinkIntent::ImportFile;
    }

    if is_app_universal_link(url) {
        let path = url.path().trim_matches('/');

        return match path {
            "callback" | "desktop/callback" => DeepLinkIntent::Auth,
            "logout" | "desktop/logout" => DeepLinkIntent::Logout,
            "thirdparty/callback" => DeepLinkIntent::ThirdParty,
            "join" => DeepLinkIntent::Join,
            "store" => classify_store(url, ""),
            _ if path.starts_with("store/") => classify_store(url, &path["store/".len()..]),
            _ if path.starts_with("trigger/") => classify_trigger(&path["trigger/".len()..]),
            _ => DeepLinkIntent::Unknown,
        };
    }

    if url.scheme() == "flow-like" {
        let rest = url.path().trim_matches('/');

        return match url.host_str().unwrap_or_default() {
            "auth" => DeepLinkIntent::Auth,
            "logout" => DeepLinkIntent::Logout,
            "thirdparty" if rest == "callback" => DeepLinkIntent::ThirdParty,
            "trigger" => classify_trigger(rest),
            "store" => classify_store(url, rest),
            "join" => DeepLinkIntent::Join,
            _ => DeepLinkIntent::Unknown,
        };
    }

    DeepLinkIntent::Unknown
}

fn is_app_universal_link(url: &Url) -> bool {
    if !(url.scheme() == "https" || url.scheme() == "http") {
        return false;
    }

    let Some(host) = url.host_str() else {
        return false;
    };
    let host = host.to_ascii_lowercase();

    host == APP_HOST || host == WEB_HOST || host == "localhost" || host == "127.0.0.1"
}

/// Accepts every documented store shape:
/// `store?id=X`, `store/X`, `store/packages?id=X`, `store/packages/X`.
fn classify_store(url: &Url, store_path: &str) -> DeepLinkIntent {
    let query_id = url
        .query_pairs()
        .find(|(key, _)| key == "id")
        .map(|(_, value)| value.to_string())
        .filter(|value| !value.is_empty());

    let store_path = store_path.trim_matches('/');

    if store_path == "packages" || store_path.starts_with("packages/") {
        let package_id = query_id.or_else(|| segment(&store_path["packages".len()..]));
        return DeepLinkIntent::Store {
            app_id: None,
            package_id,
        };
    }

    DeepLinkIntent::Store {
        app_id: query_id.or_else(|| segment(store_path)),
        package_id: None,
    }
}

fn classify_trigger(rest: &str) -> DeepLinkIntent {
    let parts: Vec<&str> = rest
        .trim_matches('/')
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();

    if parts.len() < 2 {
        return DeepLinkIntent::Unknown;
    }

    DeepLinkIntent::Trigger {
        app_id: parts[0].to_string(),
        route: parts[1..].join("/"),
    }
}

fn segment(raw: &str) -> Option<String> {
    let trimmed = raw.trim_matches('/');
    if trimmed.is_empty() {
        return None;
    }

    Some(
        urlencoding::decode(trimmed)
            .map(|decoded| decoded.into_owned())
            .unwrap_or_else(|_| trimmed.to_string()),
    )
}

fn handle_import_file(app_handle: &AppHandle, url: &Url) {
    // iOS 'Open in…' / AirDrop of documents.
    let Ok(path) = url.to_file_path() else {
        tracing::warn!(
            "Received a file deep link that is not a local path: {}",
            url
        );
        return;
    };

    let path_str = path.to_string_lossy().to_string();
    tracing::info!("Received file URL to import: {}", path_str);
    emit(app_handle, "import/file", json::json!({ "path": path_str }));
}

fn handle_auth(app_handle: &AppHandle, url: &str) {
    tracing::info!("Handling auth URL: {}", url);
    emit(app_handle, "oidc/url", json::json!({ "url": url }));
}

fn handle_thirdparty_callback(app_handle: &AppHandle, url: &Url) {
    // Supports both OAuth (code flow) and OIDC (implicit flow with id_token).
    let mut params = serde_json::Map::new();
    if let Some(query) = url.query() {
        collect_params(query, &mut params);
    }

    // Some OIDC providers return tokens in the fragment; query params win.
    if let Some(fragment) = url.fragment() {
        collect_params(fragment, &mut params);
    }

    tracing::info!(
        "Thirdparty OAuth/OIDC callback received with {} param(s)",
        params.len()
    );

    emit(
        app_handle,
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
    );
}

fn collect_params(raw: &str, params: &mut serde_json::Map<String, serde_json::Value>) {
    for pair in raw.split('&') {
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        let key = urlencoding::decode(key).unwrap_or_default().into_owned();
        if params.contains_key(&key) {
            continue;
        }
        let value = urlencoding::decode(value).unwrap_or_default().into_owned();
        params.insert(key, serde_json::Value::String(value));
    }
}

fn handle_trigger(app_handle: &AppHandle, url: &Url, app_id: &str, route: &str) {
    let mut params = serde_json::Map::new();
    if let Some(query) = url.query() {
        collect_params(query, &mut params);
    }
    let query_params = serde_json::Value::Object(params);

    tracing::info!(
        "Trigger deep link: app_id='{}', route='{}', params={:?}",
        app_id,
        route,
        query_params
    );

    match crate::event_sink::deeplink::DeeplinkSink::handle_trigger(
        app_handle,
        app_id,
        route,
        query_params,
    ) {
        Ok(true) => tracing::info!("Deeplink event triggered successfully"),
        Ok(false) => tracing::warn!("Deeplink event not triggered (offline or not found)"),
        Err(e) => tracing::error!("Failed to trigger deeplink event: {}", e),
    }
}

fn handle_join(app_handle: &AppHandle, url: &Url, replayed: bool) {
    let params: std::collections::HashMap<_, _> = url.query_pairs().collect();
    let app_id = params.get("appId").map(|v| v.to_string());
    let token = params.get("token").map(|v| v.to_string());

    tracing::info!("Join deep link: app_id={:?}", app_id);

    emit(
        app_handle,
        "deeplink/join",
        json::json!({ "appId": app_id, "token": token, "replayed": replayed }),
    );
}

/// A URL the app cannot route. Previously this dropped the URL and left the app sitting on
/// whatever it happened to be showing; hand http(s) URLs to the browser instead, which is what
/// the user expected when they tapped the link.
fn handle_unknown(app_handle: &AppHandle, url: &Url) {
    tracing::warn!("Unhandled deep link URL: {}", url);

    if !should_open_externally(url) {
        return;
    }

    use tauri_plugin_opener::OpenerExt;
    if let Err(error) = app_handle.opener().open_url(url.as_str(), None::<&str>) {
        tracing::warn!("Failed to open the unhandled deep link in a browser: {error}");
    }
}

/// `APP_HOST` is the only host claimed by the iOS entitlement and the Android intent filter, so
/// handing one of its URLs back to the OS would route it straight into the app again. Everything
/// else is safe to open.
fn should_open_externally(url: &Url) -> bool {
    if url.scheme() != "https" && url.scheme() != "http" {
        return false;
    }

    url.host_str()
        .is_none_or(|host| !host.eq_ignore_ascii_case(APP_HOST))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn intent(raw: &str) -> DeepLinkIntent {
        classify(&Url::parse(raw).expect("test URL parses"))
    }

    fn store(app_id: Option<&str>, package_id: Option<&str>) -> DeepLinkIntent {
        DeepLinkIntent::Store {
            app_id: app_id.map(ToString::to_string),
            package_id: package_id.map(ToString::to_string),
        }
    }

    #[test]
    fn accepts_the_configured_hosts_only() {
        for host in [APP_HOST, WEB_HOST, "localhost", "127.0.0.1"] {
            let url = Url::parse(&format!("https://{host}/store")).unwrap();
            assert!(is_app_universal_link(&url), "expected {host} to be allowed");
        }

        for host in [
            "evil.com",
            "app.flow-like.com.evil.com",
            "flow-like.com.evil.com",
            "notflow-like.com",
        ] {
            let url = Url::parse(&format!("https://{host}/store")).unwrap();
            assert!(
                !is_app_universal_link(&url),
                "expected {host} to be rejected"
            );
        }
    }

    #[test]
    fn rejects_non_http_schemes_as_universal_links() {
        let url = Url::parse(&format!("ftp://{APP_HOST}/store")).unwrap();
        assert!(!is_app_universal_link(&url));
    }

    #[test]
    fn uppercase_hosts_are_normalized() {
        let url = Url::parse("https://APP.FLOW-LIKE.COM/store/packages/pkg").unwrap();
        assert!(is_app_universal_link(&url));
        assert_eq!(classify(&url), store(None, Some("pkg")));
    }

    #[test]
    fn store_shapes_resolve_on_every_host_and_scheme() {
        let cases = [
            ("store?id=app-1", store(Some("app-1"), None)),
            ("store/app-1", store(Some("app-1"), None)),
            ("store", store(None, None)),
            ("store/packages?id=pkg-1", store(None, Some("pkg-1"))),
            ("store/packages/pkg-1", store(None, Some("pkg-1"))),
            ("store/packages", store(None, None)),
        ];

        for (path, expected) in cases {
            for host in [APP_HOST, WEB_HOST] {
                assert_eq!(
                    intent(&format!("https://{host}/{path}")),
                    expected,
                    "https://{host}/{path}"
                );
            }

            assert_eq!(
                intent(&format!("flow-like://{path}")),
                expected,
                "flow-like://{path}"
            );
        }
    }

    #[test]
    fn trailing_slashes_do_not_change_the_target() {
        for path in [
            "store",
            "store/app-1",
            "store/packages",
            "store/packages/pkg-1",
        ] {
            assert_eq!(
                intent(&format!("https://{APP_HOST}/{path}")),
                intent(&format!("https://{APP_HOST}/{path}/")),
                "{path} vs {path}/"
            );
        }

        assert_eq!(
            intent(&format!("https://{APP_HOST}/store/packages/")),
            store(None, None)
        );
    }

    #[test]
    fn only_unclaimed_hosts_are_handed_back_to_the_browser() {
        let claimed = Url::parse(&format!("https://{APP_HOST}/trigger/app-only")).unwrap();
        assert_eq!(classify(&claimed), DeepLinkIntent::Unknown);
        assert!(
            !should_open_externally(&claimed),
            "reopening an app-claimed host would bounce straight back into the app"
        );

        let unclaimed = Url::parse(&format!("https://{WEB_HOST}/pricing")).unwrap();
        assert!(should_open_externally(&unclaimed));

        let custom = Url::parse("flow-like://nonsense").unwrap();
        assert!(!should_open_externally(&custom));
    }

    #[test]
    fn the_reported_store_link_resolves_to_its_package() {
        assert_eq!(
            intent("https://flow-like.com/store/packages/com.flow-like.catena-x"),
            store(None, Some("com.flow-like.catena-x"))
        );
        assert_eq!(
            intent("https://app.flow-like.com/store/packages/com.flow-like.catena-x"),
            store(None, Some("com.flow-like.catena-x"))
        );
        assert_eq!(
            intent("https://app.flow-like.com/store/packages?id=com.flow-like.catena-x"),
            store(None, Some("com.flow-like.catena-x"))
        );
    }

    #[test]
    fn percent_encoded_ids_are_decoded() {
        assert_eq!(
            intent(&format!("https://{APP_HOST}/store/packages/com.acme%2Fpkg")),
            store(None, Some("com.acme/pkg"))
        );
    }

    #[test]
    fn auth_and_logout_paths_resolve() {
        for host in [APP_HOST, WEB_HOST] {
            assert_eq!(
                intent(&format!("https://{host}/callback?code=abc&state=xyz")),
                DeepLinkIntent::Auth
            );
            assert_eq!(
                intent(&format!("https://{host}/desktop/callback?code=abc")),
                DeepLinkIntent::Auth
            );
            assert_eq!(
                intent(&format!("https://{host}/logout")),
                DeepLinkIntent::Logout
            );
            assert_eq!(
                intent(&format!("https://{host}/desktop/logout")),
                DeepLinkIntent::Logout
            );
            assert_eq!(
                intent(&format!("https://{host}/thirdparty/callback?code=abc")),
                DeepLinkIntent::ThirdParty
            );
        }

        assert_eq!(intent("flow-like://auth?code=abc"), DeepLinkIntent::Auth);
        assert_eq!(intent("flow-like://logout"), DeepLinkIntent::Logout);
        assert_eq!(
            intent("flow-like://thirdparty/callback?code=abc"),
            DeepLinkIntent::ThirdParty
        );
    }

    #[test]
    fn trigger_needs_an_app_and_a_route() {
        assert_eq!(
            intent(&format!("https://{APP_HOST}/trigger/app-1/some/route?x=1")),
            DeepLinkIntent::Trigger {
                app_id: "app-1".into(),
                route: "some/route".into(),
            }
        );
        assert_eq!(
            intent("flow-like://trigger/app-1/route"),
            DeepLinkIntent::Trigger {
                app_id: "app-1".into(),
                route: "route".into(),
            }
        );
        assert_eq!(
            intent(&format!("https://{APP_HOST}/trigger/app-1")),
            DeepLinkIntent::Unknown
        );
    }

    #[test]
    fn join_resolves_on_both_transports() {
        assert_eq!(
            intent(&format!("https://{APP_HOST}/join?appId=a&token=t")),
            DeepLinkIntent::Join
        );
        assert_eq!(
            intent("flow-like://join?appId=a&token=t"),
            DeepLinkIntent::Join
        );
    }

    #[test]
    fn unrelated_urls_stay_unknown_so_they_reach_the_browser() {
        assert_eq!(
            intent(&format!("https://{WEB_HOST}/pricing")),
            DeepLinkIntent::Unknown
        );
        assert_eq!(
            intent("https://example.com/store/packages/x"),
            DeepLinkIntent::Unknown
        );
        assert_eq!(intent("flow-like://nonsense"), DeepLinkIntent::Unknown);
    }

    #[test]
    fn host_constants_match_the_repository_config() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../../../../flow-like.config.json"))
                .expect("flow-like.config.json parses");

        assert_eq!(config["app"].as_str(), Some(APP_HOST));
        assert_eq!(config["web"].as_str(), Some(WEB_HOST));
    }
}

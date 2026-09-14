use flow_like::flow::execution::LOCAL_USER_SUB;
use flow_like::flow::execution::context::ExecutionContext;
use flow_like_types::reqwest;
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
struct NotificationApiResponse {
    success: bool,
    #[serde(default)]
    persisted: Option<bool>,
    #[serde(default)]
    push_status: Option<String>,
}

fn notification_response_result(
    response: NotificationApiResponse,
) -> flow_like_types::Result<bool> {
    if !response.success
        || matches!(
            response.push_status.as_deref(),
            Some("failed" | "partial" | "deduplicated")
        )
    {
        return Err(flow_like_types::anyhow!(
            "Notification API reported a failure (persisted={}, push_status={})",
            response.persisted.unwrap_or(false),
            response.push_status.as_deref().unwrap_or("unknown"),
        ));
    }
    Ok(true)
}

fn local_notification_result(
    target: Option<&str>,
    executing_sub: Option<&str>,
) -> flow_like_types::Result<bool> {
    if let Some(target) = target
        && !target.is_empty()
        && target != LOCAL_USER_SUB
        && Some(target) != executing_sub
    {
        return Err(flow_like_types::anyhow!(
            "Remote notification persistence is required to notify another user"
        ));
    }
    Ok(false)
}

#[derive(Serialize)]
struct AppScopedRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    event_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    board_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    target_user_sub: Option<String>,
    title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    icon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    link: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    node_id: Option<String>,
}

#[derive(Serialize)]
struct UserScopedRequest {
    title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    icon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    link: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    app_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    node_id: Option<String>,
}

pub struct PersistNotificationParams {
    pub title: String,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub link: Option<String>,
    pub target_user_sub: Option<String>,
}

fn notification_api_origin(hub: &str, secure: bool) -> Option<String> {
    let trimmed = hub.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return None;
    }

    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return Some(trimmed.to_string());
    }

    if trimmed.starts_with("//") {
        let protocol = if secure { "https:" } else { "http:" };
        return Some(format!("{protocol}{trimmed}"));
    }

    let protocol = if secure { "https" } else { "http" };
    Some(format!("{protocol}://{trimmed}"))
}

/// Reserved by the `/use` route shell. User-supplied query params with these
/// names are prefixed with `_` so they don't collide with framework values;
/// `Get Query Params` reverses the prefix transparently.
pub const RESERVED_QUERY_KEYS: &[&str] = &["id", "route", "eventId"];

fn prefix_reserved_user_query(query: &str) -> String {
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            if RESERVED_QUERY_KEYS.contains(&key) {
                if pair.contains('=') {
                    format!("_{key}={value}")
                } else {
                    format!("_{key}")
                }
            } else {
                pair.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("&")
}

/// Canonical form of a route path, mirroring `normalizeRoutePath` in
/// `packages/ui/lib/route-path.ts`.
///
/// The `/use` shell matches this string against the paths stored on events, and a
/// miss falls back to the default route without saying so — so a link that differs
/// only by a trailing slash silently opens the wrong page.
fn normalize_route_path(path: &str) -> String {
    let trimmed = path.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return "/".to_string();
    }
    match trimmed.strip_prefix('/') {
        Some(rest) => format!("/{rest}"),
        None => format!("/{trimmed}"),
    }
}

/// Build a notification link from an app_id and a user-provided path.
///
/// If `user_link` is already a non-empty relative path (e.g. `/dashboard`
/// or `/store?item=abc`), it is turned into `/use?id={app_id}&route={path}&extra=params`.
/// If `user_link` is empty or missing, defaults to `/use?id={app_id}&route=/`.
/// Absolute URLs are rejected (security: avoids phishing via push notifications);
/// the link then opens the app's default route rather than the requested one.
///
/// Query keys reserved by the `/use` shell (`id`, `route`, `eventId`) are
/// `_`-prefixed when forwarded so user values never overwrite framework
/// values in `_query_params`.
pub fn build_notification_link(app_id: &str, user_link: Option<&str>) -> String {
    let raw = user_link.unwrap_or("").trim();

    // Reject absolute URLs
    if raw.starts_with("http://") || raw.starts_with("https://") || raw.starts_with("//") {
        return format!("/use?id={}", urlencoding::encode(app_id));
    }

    // Normalise: strip leading slash for splitting
    let trimmed = raw.strip_prefix('/').unwrap_or(raw);

    if trimmed.is_empty() {
        return format!("/use?id={}", urlencoding::encode(app_id));
    }

    // Split into path and query parts
    let (path_part, query_part) = trimmed.split_once('?').unwrap_or((trimmed, ""));
    let route = normalize_route_path(path_part);

    let mut link = format!(
        "/use?id={}&route={}",
        urlencoding::encode(app_id),
        urlencoding::encode(&route),
    );

    // Append any extra query params from the user-provided link, prefixing
    // reserved keys so they don't collide with the `/use` framework keys.
    if !query_part.is_empty() {
        let safe = prefix_reserved_user_query(query_part);
        if !safe.is_empty() {
            link.push('&');
            link.push_str(&safe);
        }
    }

    link
}

/// Persist a notification via the backend API.
///
/// 1. If the app is online (event_id or board_id available), tries the app-scoped
///    `POST /api/v1/apps/{app_id}/notifications/create` endpoint.
/// 2. If that returns 403/404 or no app context exists, falls back to the user-scoped
///    `POST /api/v1/user/notifications/create` endpoint.
/// 3. Returns `Ok(false)` for a local notification to the executing user when no
///    hub/token is available. API or reported push failures return an error.
pub async fn persist_notification(
    context: &ExecutionContext,
    params: PersistNotificationParams,
) -> flow_like_types::Result<bool> {
    let hub_url = match notification_api_origin(&context.profile.hub, context.profile.secure) {
        Some(url) => url,
        None => {
            return local_notification_result(
                params.target_user_sub.as_deref(),
                context.user_context().map(|user| user.sub.as_str()),
            );
        }
    };
    let token = match &context.token {
        Some(t) if !t.is_empty() => t,
        _ => {
            return local_notification_result(
                params.target_user_sub.as_deref(),
                context.user_context().map(|user| user.sub.as_str()),
            );
        }
    };

    let app_id = context
        .execution_cache
        .as_ref()
        .map(|c| c.app_id.clone())
        .filter(|id| !id.is_empty());

    let run_id = Some(context.run_id().to_string());
    let node_id = Some(context.id.to_string());
    let client = reqwest::Client::new();

    // Try app-scoped endpoint first (online projects with board context)
    if let Some(ref aid) = app_id {
        if params.target_user_sub.is_none() || params.target_user_sub.as_deref() == Some("local") {
            // self-notifications can fall back to user-scoped; try app-scoped first
        } else {
            // other-user targeting requires app scope — no fallback
        }

        let event_id = context.event_id().await;
        let board_id = context
            .execution_cache
            .as_ref()
            .map(|c| c.board_id.clone())
            .filter(|_| event_id.is_none());

        let url = format!("{}/api/v1/apps/{}/notifications/create", hub_url, aid);
        let body = AppScopedRequest {
            event_id,
            board_id,
            target_user_sub: params.target_user_sub.clone(),
            title: params.title.clone(),
            description: params.description.clone(),
            icon: params.icon.clone(),
            link: params.link.clone(),
            run_id: run_id.clone(),
            node_id: node_id.clone(),
        };

        let response = client
            .post(&url)
            .bearer_auth(token)
            .json(&body)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                return notification_response_result(resp.json().await?);
            }
            Ok(resp) if resp.status().as_u16() == 403 || resp.status().as_u16() == 404 => {
                // Fall through to user-scoped endpoint
            }
            Ok(resp) => {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                return Err(flow_like_types::anyhow!(
                    "App notification API returned {}: {}",
                    status,
                    text
                ));
            }
            Err(e) => {
                return Err(flow_like_types::anyhow!(
                    "App notification API request failed: {}",
                    e
                ));
            }
        }
    }

    // Another user's notification requires app scope to verify membership.
    // Targeting the executing user stays allowed — the user-scoped endpoint is
    // bound to their own token, and authenticated local runs report a real sub.
    let executing_sub = context.user_context().map(|user| user.sub.as_str());
    if let Some(ref target) = params.target_user_sub
        && target != LOCAL_USER_SUB
        && !target.is_empty()
        && Some(target.as_str()) != executing_sub
    {
        return local_notification_result(Some(target), executing_sub);
    }

    // Fallback: user-scoped endpoint (offline projects / no board context)
    let url = format!("{}/api/v1/user/notifications/create", hub_url);
    let body = UserScopedRequest {
        title: params.title,
        description: params.description,
        icon: params.icon,
        link: params.link,
        app_id,
        run_id,
        node_id,
    };

    let response = client
        .post(&url)
        .bearer_auth(token)
        .json(&body)
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        return Err(flow_like_types::anyhow!(
            "User notification API returned {}: {}",
            status,
            text
        ));
    }

    notification_response_result(response.json().await?)
}

#[cfg(test)]
mod tests {
    use super::{
        build_notification_link, local_notification_result, notification_api_origin,
        notification_response_result,
    };
    use flow_like_types::json as serde_json;

    #[test]
    fn successful_http_response_can_report_failed_push() {
        let response =
            serde_json::from_str(r#"{"success":false,"persisted":true,"push_status":"failed"}"#)
                .unwrap();
        assert!(notification_response_result(response).is_err());
        let response = serde_json::from_str(
            r#"{"success":false,"persisted":true,"push_status":"deduplicated"}"#,
        )
        .unwrap();
        assert!(notification_response_result(response).is_err());
        let response =
            serde_json::from_str(r#"{"success":true,"persisted":true,"push_status":"partial"}"#)
                .unwrap();
        assert!(notification_response_result(response).is_err());
    }

    #[test]
    fn persistence_without_a_push_target_and_legacy_responses_remain_successful() {
        for body in [
            r#"{"success":true,"persisted":true,"push_status":"no_targets"}"#,
            r#"{"success":true}"#,
        ] {
            assert!(notification_response_result(serde_json::from_str(body).unwrap()).unwrap());
        }
    }

    #[test]
    fn offline_success_only_applies_to_the_executing_user() {
        assert!(!local_notification_result(None, None).unwrap());
        assert!(!local_notification_result(Some("local"), None).unwrap());
        assert!(!local_notification_result(Some("user-a"), Some("user-a")).unwrap());
        assert!(local_notification_result(Some("user-b"), Some("user-a")).is_err());
    }

    #[test]
    fn prefixes_reserved_user_query_keys() {
        assert_eq!(
            build_notification_link("app1", Some("/mail?id=xyz")),
            "/use?id=app1&route=%2Fmail&_id=xyz"
        );
        assert_eq!(
            build_notification_link("app1", Some("/mail?route=foo&eventId=bar&mailid=42")),
            "/use?id=app1&route=%2Fmail&_route=foo&_eventId=bar&mailid=42"
        );
    }

    #[test]
    fn passes_through_non_reserved_query_keys() {
        assert_eq!(
            build_notification_link("app1", Some("/mail?mailid=xyz&from=abc")),
            "/use?id=app1&route=%2Fmail&mailid=xyz&from=abc"
        );
    }

    #[test]
    fn builds_the_reported_config_link() {
        assert_eq!(
            build_notification_link("app1", Some("/config?config_id=abc")),
            "/use?id=app1&route=%2Fconfig&config_id=abc"
        );
    }

    /// The `/use` shell compares the route against the path stored on the event with
    /// no normalization of its own, and a miss silently renders the default route.
    #[test]
    fn route_paths_are_canonical() {
        for link in ["/config/", "config", "config/", "  /config  ", "/config//"] {
            assert_eq!(
                build_notification_link("app1", Some(link)),
                "/use?id=app1&route=%2Fconfig",
                "link {link} should canonicalize to /config"
            );
        }

        assert_eq!(
            build_notification_link("app1", Some("/config/?config_id=abc")),
            "/use?id=app1&route=%2Fconfig&config_id=abc"
        );
    }

    #[test]
    fn nested_paths_keep_their_segments() {
        assert_eq!(
            build_notification_link("app1", Some("/config/general/?tab=2")),
            "/use?id=app1&route=%2Fconfig%2Fgeneral&tab=2"
        );
    }

    #[test]
    fn empty_and_root_links_target_the_default_route() {
        assert_eq!(build_notification_link("app1", None), "/use?id=app1");
        assert_eq!(build_notification_link("app1", Some("")), "/use?id=app1");
        assert_eq!(build_notification_link("app1", Some("/")), "/use?id=app1");
    }

    #[test]
    fn absolute_urls_fall_back_to_the_default_route() {
        for link in [
            "https://evil.example/steal",
            "http://evil.example/steal",
            "//evil.example/steal",
        ] {
            assert_eq!(
                build_notification_link("app1", Some(link)),
                "/use?id=app1",
                "absolute link {link} must not be followed"
            );
        }
    }

    #[test]
    fn normalizes_bare_hub_domains() {
        assert_eq!(
            notification_api_origin("api.flow-like.com", true).as_deref(),
            Some("https://api.flow-like.com")
        );
        assert_eq!(
            notification_api_origin("localhost:8080/", false).as_deref(),
            Some("http://localhost:8080")
        );
    }

    #[test]
    fn preserves_absolute_hub_urls() {
        assert_eq!(
            notification_api_origin("https://api.flow-like.com/", false).as_deref(),
            Some("https://api.flow-like.com")
        );
        assert_eq!(
            notification_api_origin("http://localhost:8080", true).as_deref(),
            Some("http://localhost:8080")
        );
    }
}

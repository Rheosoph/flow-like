use flow_like::flow::execution::context::ExecutionContext;
use flow_like_types::reqwest;
use serde::Serialize;

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

/// Build a notification link from an app_id and a user-provided path.
///
/// If `user_link` is already a non-empty relative path (e.g. `/dashboard`
/// or `/store?item=abc`), it is turned into `/use?id={app_id}&route={path}&extra=params`.
/// If `user_link` is empty or missing, defaults to `/use?id={app_id}&route=/`.
/// Absolute URLs are rejected (security: avoids phishing via push notifications).
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

    let mut link = format!(
        "/use?id={}&route={}",
        urlencoding::encode(app_id),
        urlencoding::encode(&format!("/{path_part}")),
    );

    // Append any extra query params from the user-provided link
    if !query_part.is_empty() {
        link.push('&');
        link.push_str(query_part);
    }

    link
}

/// Persist a notification via the backend API.
///
/// 1. If the app is online (event_id or board_id available), tries the app-scoped
///    `POST /api/v1/apps/{app_id}/notifications/create` endpoint.
/// 2. If that returns 403/404 or no app context exists, falls back to the user-scoped
///    `POST /api/v1/user/notifications/create` endpoint.
/// 3. Returns `Ok(false)` if no hub/token is available (purely local).
pub async fn persist_notification(
    context: &ExecutionContext,
    params: PersistNotificationParams,
) -> flow_like_types::Result<bool> {
    let hub_url = context.profile.hub.trim_end_matches('/');
    let token = match &context.token {
        Some(t) if !t.is_empty() => t,
        _ => return Ok(false),
    };
    if hub_url.is_empty() {
        return Ok(false);
    }
    if !hub_url.starts_with("http://") && !hub_url.starts_with("https://") {
        return Ok(false);
    }

    let app_id = context
        .execution_cache
        .as_ref()
        .map(|c| c.app_id.clone())
        .filter(|id| !id.is_empty());

    let run_id = Some(context.run_id().to_string());
    let node_id = Some(context.id.clone());
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
            Ok(resp) if resp.status().is_success() => return Ok(true),
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

    // For other-user targeting without a valid app, skip (can't verify membership)
    if let Some(ref target) = params.target_user_sub
        && target != "local"
        && !target.is_empty()
    {
        return Ok(false);
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

    Ok(true)
}

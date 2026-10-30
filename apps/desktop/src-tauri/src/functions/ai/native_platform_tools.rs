//! Native implementations of platform tools shared by desktop FlowPilot backends.

use std::{collections::HashSet, time::Duration};

use flow_like::{
    app::{App, AppVisibility},
    hub::hub_origin,
    profile::Profile,
};
use reqwest::{Client, Method};
use serde_json::{Value, json};
use tauri::AppHandle;

use super::frontend_tool_bridge::FrontendToolContext;
use crate::state::{TauriFlowLikeState, TauriSettingsState};

#[path = "native_app_interface_tools.rs"]
mod app_interface;
mod apps;
mod database;
mod event_tools;
mod event_trigger;
mod geojson_import;
mod marketplace;
mod storage;
mod templates;

pub(crate) async fn cancel_unavailable_interaction(
    session: &NativePlatformSession,
    app: &str,
    value: &Value,
) -> Result<(), String> {
    app_interface::cancel_unavailable(session, app, value).await
}

static CREATED_APPS: std::sync::LazyLock<dashmap::DashMap<String, String>> =
    std::sync::LazyLock::new(dashmap::DashMap::new);

pub(crate) fn created_app_for_run(context: Option<&FrontendToolContext>) -> Option<String> {
    let run_id = context.and_then(|context| context.run_id.as_deref())?;
    CREATED_APPS
        .get(run_id)
        .map(|entry| entry.value().clone())
        .or_else(|| super::native_tool_bridge::created_app(context))
}

pub(crate) fn clear_created_app(run_id: &str) {
    CREATED_APPS.remove(run_id);
}

fn remember_created_app(context: Option<&FrontendToolContext>, app_id: &str) {
    if let Some(run_id) = context.and_then(|context| context.run_id.as_deref()) {
        CREATED_APPS.insert(run_id.into(), app_id.into());
    }
}

pub(super) fn supports(tool: &str, args: &Value) -> bool {
    match tool {
        "list_apps"
        | "describe_app_interface"
        | "get_app_detail"
        | "search_apps"
        | "read_flowscript_source"
        | "create_app"
        | "delete_event"
        | "upsert_event"
        | "set_page_load_event"
        | "inspect_app"
        | "get_template_preview"
        | "fork_preview"
        | "fork_app"
        | "acquire_app"
        | "search_templates" => true,
        "call_app_event" | "call_app_chat" => true,
        "database_tool" => database::supports(args),
        "storage_tool" => true,
        _ => false,
    }
}

pub(super) async fn execute(
    app_handle: &AppHandle,
    context: Option<&FrontendToolContext>,
    tool: &str,
    args: &Value,
) -> Result<Value, String> {
    let session = NativePlatformSession::new(app_handle, context).await?;
    let result = match tool {
        "database_tool" => database::execute(&session, args).await,
        "storage_tool" => storage::execute(&session, args).await,
        "upsert_event" | "set_page_load_event" => event_tools::execute(&session, tool, args).await,
        "fork_app" | "acquire_app" => marketplace::execute(&session, tool, args).await,
        "search_templates" => templates::search(&session, args).await,
        "call_app_event" | "call_app_chat" => app_interface::execute(&session, tool, args).await,
        _ => apps::execute(&session, tool, args).await,
    }?;
    session.assert_current_profile().await?;
    Ok(result)
}

/// Captures the profile and account before any asynchronous tool work. Every hub request checks
/// them again, so a sign-in or profile switch cannot redirect an in-flight tool to another user.
pub(crate) struct NativePlatformSession {
    pub(crate) app_handle: AppHandle,
    pub(crate) profile: Profile,
    pub(crate) user_profile: crate::profile::UserProfile,
    pub(crate) hub: String,
    pub(crate) subject: Option<String>,
    pub(crate) auth: Option<crate::native_auth::SessionBinding>,
    pub(crate) context: Option<FrontendToolContext>,
    client: Client,
}

impl NativePlatformSession {
    pub(crate) async fn new(
        app_handle: &AppHandle,
        context: Option<&FrontendToolContext>,
    ) -> Result<Self, String> {
        let user_profile = TauriSettingsState::current_profile(app_handle)
            .await
            .map_err(err)?;
        let profile = user_profile.hub_profile.clone();
        if context
            .and_then(|c| c.profile_id.as_deref())
            .is_some_and(|id| id != profile.id)
        {
            return Err("The FlowPilot run belongs to a different profile. Start a new run in the selected profile.".into());
        }
        let hub = hub_origin(&profile.hub, profile.secure).unwrap_or_default();
        let auth = crate::native_auth::SessionBinding::capture(&profile.id, &hub).await?;
        let subject = auth.as_ref().map(|binding| binding.subject.clone());
        if let Some(identity) = super::native_tool_bridge::run_identity(context)
            && (identity.profile_id != profile.id
                || identity.hub != hub
                || identity.subject != subject
                || identity.auth.as_ref().map(|binding| &binding.generation)
                    != auth.as_ref().map(|binding| &binding.generation))
        {
            return Err(
                "The profile or authentication session changed since this FlowPilot run started."
                    .into(),
            );
        }
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(120))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(err)?;
        Ok(Self {
            app_handle: app_handle.clone(),
            profile,
            user_profile,
            hub,
            subject,
            auth,
            context: context.cloned(),
            client,
        })
    }

    pub(crate) async fn assert_current_profile(&self) -> Result<(), String> {
        let current = TauriSettingsState::current_profile(&self.app_handle)
            .await
            .map_err(err)?
            .hub_profile;
        let hub = hub_origin(&current.hub, current.secure).unwrap_or_default();
        if current.id != self.profile.id || hub != self.hub {
            return Err("The selected profile changed while the tool was running.".into());
        }
        let current_auth =
            crate::native_auth::SessionBinding::capture(&self.profile.id, &self.hub).await?;
        if current_auth
            .as_ref()
            .map(|binding| (&binding.subject, &binding.generation))
            != self
                .auth
                .as_ref()
                .map(|binding| (&binding.subject, &binding.generation))
        {
            return Err("The signed-in account changed while the tool was running.".into());
        }
        if super::native_tool_bridge::execution_cancellation().is_cancelled() {
            return Err("The owning FlowPilot run was cancelled.".into());
        }
        Ok(())
    }

    pub(crate) async fn token(&self) -> Result<String, String> {
        self.assert_current_profile().await?;
        match &self.auth {
            Some(auth) => auth.access_token(None).await,
            None => {
                crate::native_auth::request_login(self.profile.id.clone());
                Err("This tool requires an authenticated hub session. Complete sign-in and start a new run.".into())
            }
        }
    }

    pub(crate) fn app_id(&self, args: &Value) -> Result<String, String> {
        let scoped = self.context.as_ref().and_then(|c| c.app_id.as_deref());
        let supplied = string(args, "app_id", "appId");
        let created = created_app_for_run(self.context.as_ref());
        let app_id = supplied
            .or(created.as_deref())
            .or(scoped)
            .ok_or("This tool requires app_id.")?;
        self.require_visible(app_id)?;
        Ok(app_id.to_owned())
    }

    pub(crate) fn profile_app_ids(&self) -> HashSet<String> {
        self.profile
            .apps
            .iter()
            .flatten()
            .map(|app| app.app_id.clone())
            .collect()
    }

    pub(crate) fn require_visible(&self, app_id: &str) -> Result<(), String> {
        if created_app_for_run(self.context.as_ref()).as_deref() == Some(app_id)
            || self
                .profile
                .apps
                .iter()
                .flatten()
                .any(|entry| entry.app_id == app_id)
        {
            Ok(())
        } else {
            Err(format!(
                "App '{app_id}' is not visible in the current profile."
            ))
        }
    }

    pub(crate) async fn is_local(&self, app_id: &str) -> Result<bool, String> {
        self.assert_current_profile().await?;
        let state = TauriFlowLikeState::construct(&self.app_handle)
            .await
            .map_err(err)?;
        match App::load(app_id.to_owned(), state).await {
            Ok(app) => {
                let local = matches!(app.visibility, AppVisibility::Offline);
                if local {
                    self.require_visible(app_id)?;
                }
                Ok(local)
            }
            Err(_) if self.subject.is_some() => Ok(false),
            Err(error) => Err(error.to_string()),
        }
    }

    pub(crate) async fn request(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<Value, String> {
        self.request_with_headers(method, path, body, &[]).await
    }

    pub(crate) async fn request_with_headers(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
        headers: &[(&str, &str)],
    ) -> Result<Value, String> {
        let mut response = self.request_response(method, path, body, headers).await?;
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| "The hub response was interrupted.".to_string())?
        {
            if bytes.len().saturating_add(chunk.len()) > 16 * 1024 * 1024 {
                return Err("The hub response exceeds the tool response limit.".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        self.assert_current_profile().await?;
        if bytes.is_empty() {
            Ok(Value::Null)
        } else {
            serde_json::from_slice(&bytes).map_err(|_| "The hub returned invalid JSON.".into())
        }
    }

    pub(crate) async fn request_response(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
        headers: &[(&str, &str)],
    ) -> Result<reqwest::Response, String> {
        self.assert_current_profile().await?;
        let url = api_url(&self.hub, path)?;
        let mut token = match self.subject.as_deref() {
            Some(_) => Some(self.token().await?),
            None => None,
        };
        for attempt in 0..2 {
            self.assert_current_profile().await?;
            let mut request = self.client.request(method.clone(), url.clone());
            for (name, value) in headers {
                if name.eq_ignore_ascii_case("authorization")
                    || name.eq_ignore_ascii_case("cookie")
                    || name.eq_ignore_ascii_case("host")
                {
                    return Err(
                        "Authentication and destination headers are owned by the native session."
                            .into(),
                    );
                }
                request = request.header(*name, *value);
            }
            if let Some(token) = &token {
                request = request.bearer_auth(token);
            }
            if let Some(body) = body {
                request = request.json(body);
            }
            let response = request
                .send()
                .await
                .map_err(|_| "The hub request could not be completed.".to_string())?;
            if response.status() == reqwest::StatusCode::UNAUTHORIZED && self.auth.is_none() {
                crate::native_auth::request_login(self.profile.id.clone());
                return Err("This tool requires an authenticated hub session. Complete sign-in and start a new run.".into());
            }
            if response.status() == reqwest::StatusCode::UNAUTHORIZED
                && attempt == 0
                && let (Some(auth), Some(rejected)) = (self.auth.as_ref(), token.as_deref())
            {
                token = Some(auth.access_token(Some(rejected)).await?);
                continue;
            }
            if !response.status().is_success() {
                return Err(format!(
                    "The hub returned HTTP {} for this tool request.",
                    response.status().as_u16()
                ));
            }
            self.assert_current_profile().await?;
            return Ok(response);
        }
        Err("The hub rejected the refreshed session.".into())
    }

    pub(crate) fn client(&self) -> &Client {
        &self.client
    }
}

pub(crate) async fn app_events(
    session: &NativePlatformSession,
    app_id: &str,
) -> Result<Vec<Value>, String> {
    apps::events(session, app_id).await
}

pub(crate) fn workflow_entries(board: &Value, board_id: &str) -> Vec<Value> {
    board["nodes"].as_object().into_iter().flat_map(|nodes|nodes.values()).filter(|node| node["id"].as_str().is_some_and(|id|event_tools::runnable(board,id))).map(|node|json!({"id":node["id"],"board_id":board_id,"name":node["friendly_name"].as_str().filter(|s|!s.is_empty()).unwrap_or(node["name"].as_str().unwrap_or("")),"node_type":node["name"],"supported_event_types":event_tools::supported_types(node["name"].as_str().unwrap_or("")).1,"created_this_run":true})).collect()
}

pub(crate) async fn provision_inactive_event(
    session: &NativePlatformSession,
    args: &Value,
    reserved_id: &str,
) -> Result<Value, String> {
    event_tools::upsert_reserved(session, args, reserved_id).await
}

pub(crate) fn select_forwarded_attachments(
    files: &[flow_like::flow::copilot::AttachmentManifestEntry],
    requested: Option<&Value>,
) -> Result<Vec<flow_like::flow::copilot::AttachmentManifestEntry>, String> {
    app_interface::select_forwarded(files, requested)
}

pub(crate) async fn should_skip_cached_schema_approval(
    app: &AppHandle,
    context: Option<&FrontendToolContext>,
    args: &Value,
) -> bool {
    if string(args, "operation", "operation") != Some("create_table") {
        return false;
    }
    let Ok(session) = NativePlatformSession::new(app, context).await else {
        return false;
    };
    session
        .app_id(args)
        .is_ok_and(|app| database::schema_unavailable(&session, &app))
}

pub(crate) async fn table_metadata(
    session: &NativePlatformSession,
    app_id: &str,
    personal: bool,
    table: Option<&str>,
) -> Result<Value, String> {
    database::metadata(session, app_id, personal, table).await
}

fn api_url(hub: &str, path: &str) -> Result<reqwest::Url, String> {
    let base =
        reqwest::Url::parse(&format!("{}/api/v1/", hub.trim_end_matches('/'))).map_err(err)?;
    if path
        .split('?')
        .next()
        .unwrap_or(path)
        .split('/')
        .any(|part| urlencoding::decode(part).is_ok_and(|part| matches!(part.as_ref(), "." | "..")))
    {
        return Err("Invalid hub API path.".into());
    }
    let url = base.join(path).map_err(err)?;
    if url.origin() != base.origin()
        || !url.path().starts_with(base.path())
        || url.fragment().is_some()
    {
        return Err("Invalid hub API path.".into());
    }
    Ok(url)
}

pub(crate) fn segment(value: &str) -> String {
    value
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'~') {
                (byte as char).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

pub(crate) fn string<'a>(args: &'a Value, snake: &str, camel: &str) -> Option<&'a str> {
    args.get(snake)
        .or_else(|| args.get(camel))
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
}

pub(crate) fn required(args: &Value, snake: &str, camel: &str) -> Result<String, String> {
    string(args, snake, camel)
        .map(str::to_owned)
        .ok_or_else(|| format!("This operation requires {snake}."))
}

pub(crate) fn boolean(args: &Value, snake: &str, camel: &str, default: bool) -> bool {
    args.get(snake)
        .or_else(|| args.get(camel))
        .and_then(Value::as_bool)
        .unwrap_or(default)
}

pub(crate) fn limit(args: &Value, snake: &str, camel: &str, default: usize, max: usize) -> usize {
    args.get(snake)
        .or_else(|| args.get(camel))
        .and_then(Value::as_f64)
        .filter(|n| n.is_finite() && *n > 0.0)
        .map_or(default, |n| (n.floor() as usize).min(max))
}

pub(crate) fn err(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hub_paths_cannot_change_origin_or_escape_api() {
        for path in [
            "https://evil.test/",
            "//evil.test/",
            "../../secrets",
            "apps/%2e%2e/%2e%2e/secrets",
            "apps/%2E./secrets",
            "/other",
            "apps#fragment",
        ] {
            assert!(api_url("https://hub.test", path).is_err(), "{path}");
        }
        assert_eq!(
            api_url("https://hub.test", "apps/a/db?user_scoped=true")
                .unwrap()
                .as_str(),
            "https://hub.test/api/v1/apps/a/db?user_scoped=true"
        );
    }
    #[test]
    fn tool_limits_match_web_defaults_and_caps() {
        for value in [json!(0), json!(-1), json!("20"), Value::Null] {
            assert_eq!(
                limit(&json!({"limit": value}), "limit", "limit", 50, 200),
                50
            );
        }
        assert_eq!(
            limit(&json!({"limit": 999}), "limit", "limit", 50, 200),
            200
        );
        assert_eq!(segment("a/b?c"), "a%2Fb%3Fc");
    }
}

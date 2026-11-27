//! Calls to the hub as the signed-in user of the active profile; the token is refreshed once
//! on a 401.

use crate::native_auth::SessionBinding;
use crate::state::TauriSettingsState;
use reqwest::Method;
use serde_json::Value;
use std::time::Duration;
use tauri::AppHandle;

const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

#[derive(Debug)]
pub struct HubError {
    pub status: u16,
    pub message: String,
}

impl HubError {
    fn offline(message: impl Into<String>) -> Self {
        Self {
            status: 0,
            message: message.into(),
        }
    }

    pub fn is_transient(&self) -> bool {
        self.status == 0 || self.status == 429 || self.status >= 500
    }
}

impl std::fmt::Display for HubError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "hub answered {}: {}", self.status, self.message)
    }
}

pub struct Session {
    pub hub: String,
    pub auth: SessionBinding,
}

/// The signed-in session of the active profile; `None` when signed out.
pub async fn session(app: &AppHandle) -> Result<Option<Session>, String> {
    let settings = TauriSettingsState::construct(app)
        .await
        .map_err(|error| error.to_string())?;
    let (profile_id, hub) = {
        let settings = settings.lock().await;
        let profile = settings
            .get_current_profile()
            .map_err(|error| error.to_string())?;
        let hub = if profile.hub_profile.hub.trim().is_empty() {
            String::new()
        } else {
            flow_like::hub::hub_origin(&profile.hub_profile.hub, profile.hub_profile.secure)
                .ok_or("The active profile has an invalid hub origin.")?
        };
        (profile.hub_profile.id, hub)
    };
    Ok(SessionBinding::capture(&profile_id, &hub)
        .await?
        .map(|auth| Session { hub, auth }))
}

pub struct Hub {
    app: AppHandle,
    client: reqwest::Client,
}

impl Hub {
    pub fn new(app: AppHandle) -> Self {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(45))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap_or_default();
        Self { app, client }
    }

    pub async fn call(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<&Value>,
    ) -> Result<Value, HubError> {
        let session = session(&self.app)
            .await
            .map_err(HubError::offline)?
            .ok_or_else(|| HubError {
                status: 401,
                message: "Sign in to relay Discord messages".into(),
            })?;
        let url = format!("{}/api/v1/{path}", session.hub.trim_end_matches('/'));
        let mut token = session
            .auth
            .cached_access_token()
            .await
            .map_err(HubError::offline)?;
        for attempt in 0..2 {
            let mut request = self
                .client
                .request(method.clone(), &url)
                .query(query)
                .bearer_auth(&token);
            if let Some(body) = body {
                request = request.json(body);
            }
            let response = request
                .send()
                .await
                .map_err(|_| HubError::offline("the hub could not be reached"))?;
            let status = response.status().as_u16();
            if status == 401 && attempt == 0 {
                token = session
                    .auth
                    .access_token(Some(&token))
                    .await
                    .map_err(HubError::offline)?;
                continue;
            }
            let bytes = response
                .bytes()
                .await
                .map_err(|_| HubError::offline("the hub's answer broke off"))?;
            if bytes.len() > MAX_RESPONSE_BYTES {
                return Err(HubError::offline("the hub's answer is too large"));
            }
            let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
            if (200..300).contains(&status) {
                return Ok(value);
            }
            let message = value
                .get("message")
                .or_else(|| value.get("error"))
                .and_then(Value::as_str)
                .unwrap_or("request failed")
                .chars()
                .take(200)
                .collect();
            return Err(HubError { status, message });
        }
        Err(HubError::offline("the hub refused the session"))
    }
}

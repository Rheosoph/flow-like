//! The signed-in account at the active profile's hub. The hub is read here, so the webview
//! never chooses where a bot or its secret values go; only the person's token comes from it.

use super::{CommandError, workspace::DesktopWorkspaces};
use crate::{
    functions::app::fork::{
        self, UploadLocalAppContentArgs, is_uploaded_content_path, parse_storage_path,
        summarize_store_prefix,
    },
    state::TauriFlowLikeState,
};
use flow_like::{credentials::SharedCredentials, flow_like_storage::Path};
use flow_like_local_bots::{
    AccountHub, HubCall, HubError, HubMethod, MoveError, MoveStep, WorkspaceHost, WorkspaceSummary,
    codes, transfer::HUB_RESPONSE_INVALID,
};
use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use serde::Deserialize;
use serde_json::Value;
use std::{collections::HashMap, sync::Arc, time::Duration};
use tauri::AppHandle;
use zeroize::Zeroizing;

const HUB_MESSAGE_MAX_CHARS: usize = 300;
const WORKSPACE_FAILED: &str = "WORKSPACE_FAILED";
const WORKSPACE_FILES_SHOWN: usize = 20;

pub(super) struct HubClient {
    hub: String,
    secure: bool,
    authorization: Zeroizing<String>,
    client: reqwest::Client,
}

impl HubClient {
    /// The active profile's hub with `access_token`; `purpose` finishes "Sign in to …".
    pub(super) async fn active(
        app: &AppHandle,
        access_token: &str,
        purpose: &str,
    ) -> Result<Self, CommandError> {
        let sign_in = |reason: String| CommandError::new(codes::ACCOUNT_REQUIRED, reason);
        let token = access_token.trim();
        if token.is_empty() {
            return Err(sign_in(format!("Sign in to {purpose}")));
        }
        let profile = crate::state::TauriSettingsState::current_profile(app)
            .await
            .map_err(|_| sign_in(format!("Sign in to {purpose}: there is no active profile")))?
            .hub_profile;
        if flow_like::hub::hub_origin(&profile.hub, profile.secure).is_none() {
            return Err(sign_in(format!(
                "Sign in to {purpose}: the active profile has no hub"
            )));
        }
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(120))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| {
                CommandError::new(codes::STORAGE, "The hub client could not be created")
            })?;
        Ok(Self {
            hub: profile.hub,
            secure: profile.secure,
            authorization: Zeroizing::new(crate::offline_writes::authorization(token)),
            client,
        })
    }

    pub(super) async fn send(&self, call: HubCall<'_>) -> Result<Option<Value>, HubError> {
        let mut url = flow_like::hub::hub_api_url(&self.hub, self.secure, call.path)
            .map_err(|error| HubError::unreachable(error.to_string()))?;
        if !call.query.is_empty() {
            url.query_pairs_mut()
                .extend_pairs(call.query.iter().copied());
        }
        let mut request = self
            .client
            .request(method(call.method), url.as_str())
            .header(AUTHORIZATION, self.authorization.as_str())
            .header(ACCEPT, "application/json");
        let body = call.body.to_json().map_err(|error| {
            HubError::unreachable(format!("the request could not be written: {error}"))
        })?;
        if let Some(body) = body {
            request = request
                .header(CONTENT_TYPE, "application/json")
                .body(body.to_vec());
        }
        let response = request
            .send()
            .await
            .map_err(|_| HubError::unreachable("the hub could not be reached"))?;
        let status = response.status().as_u16();
        let bytes = response
            .bytes()
            .await
            .map_err(|_| HubError::unreachable("the hub's answer broke off"))?;
        answer(status, &bytes)
    }
}

fn method(method: HubMethod) -> reqwest::Method {
    match method {
        HubMethod::Get => reqwest::Method::GET,
        HubMethod::Post => reqwest::Method::POST,
        HubMethod::Put => reqwest::Method::PUT,
        HubMethod::Patch => reqwest::Method::PATCH,
        HubMethod::Delete => reqwest::Method::DELETE,
    }
}

/// A 2xx answer's JSON (`None` when empty); otherwise the hub's `{ error: { code, message } }`.
fn answer(status: u16, body: &[u8]) -> Result<Option<Value>, HubError> {
    #[derive(Deserialize)]
    struct Envelope {
        error: Refusal,
    }
    #[derive(Deserialize)]
    struct Refusal {
        code: Option<String>,
        message: Option<String>,
    }
    if !(200..300).contains(&status) {
        let refusal = serde_json::from_slice::<Envelope>(body)
            .ok()
            .map(|envelope| envelope.error);
        let message = refusal
            .as_ref()
            .and_then(|refusal| refusal.message.as_deref())
            .map(|message| message.chars().take(HUB_MESSAGE_MAX_CHARS).collect())
            .unwrap_or_else(|| format!("the hub answered HTTP {status}"));
        return Err(HubError::answered(
            status,
            refusal.and_then(|refusal| refusal.code),
            message,
        ));
    }
    if body.iter().all(u8::is_ascii_whitespace) {
        return Ok(None);
    }
    serde_json::from_slice(body).map(Some).map_err(|_| {
        HubError::answered(
            status,
            Some(HUB_RESPONSE_INVALID.into()),
            "the hub's answer is not JSON",
        )
    })
}

/// The account a bot moves into: the active profile's hub as the person whose token it is.
pub(super) struct AccountSession {
    app: AppHandle,
    client: HubClient,
    caller: String,
}

impl AccountSession {
    pub(super) async fn open(
        app: &AppHandle,
        access_token: &str,
        bot_id: &str,
    ) -> Result<Self, MoveError> {
        let refused =
            |error: CommandError| MoveError::new(MoveStep::Check, error.code, error.message);
        let client = HubClient::active(
            app,
            access_token,
            &format!("move bot {bot_id} to your account"),
        )
        .await
        .map_err(refused)?;
        let caller = match crate::offline_writes::token_subject(Some(access_token)) {
            Some(subject) => subject,
            None => caller_of(&client).await?,
        };
        Ok(Self {
            app: app.clone(),
            client,
            caller,
        })
    }
}

/// A token that is no JWT (a personal access token) names its person through `GET user/info`.
async fn caller_of(client: &HubClient) -> Result<String, MoveError> {
    let failed = |code: String, reason: &str| {
        MoveError::new(
            MoveStep::Check,
            code,
            format!("Could not find out who is signed in: {reason}"),
        )
    };
    let info = client
        .send(HubCall::new(HubMethod::Get, &["user", "info"]))
        .await
        .map_err(|error| {
            let code = error
                .code
                .clone()
                .unwrap_or_else(|| codes::ACCOUNT_REQUIRED.into());
            failed(code, &error.message)
        })?;
    info.as_ref()
        .and_then(|info| info.get("id"))
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .ok_or_else(|| failed(HUB_RESPONSE_INVALID.into(), "the hub named no account"))
}

impl AccountHub for AccountSession {
    fn caller(&self) -> &str {
        &self.caller
    }

    async fn call(&self, call: HubCall<'_>) -> Result<Option<Value>, HubError> {
        self.client.send(call).await
    }

    async fn summarize_workspace(&self, app_id: &str) -> Result<WorkspaceSummary, HubError> {
        let summary = fork::summarize_local_app_bundle(self.app.clone(), app_id.to_string())
            .await
            .map_err(|error| workspace_failed(app_id, "measure", error))?;
        Ok(WorkspaceSummary {
            total_size_bytes: summary.total_size_bytes,
            total_object_count: summary.total_object_count,
        })
    }

    async fn upload_workspace(&self, app_id: &str, grant: &Value) -> Result<(), HubError> {
        let (prefix, credentials) = upload_target(grant)?;
        let uploaded = fork::upload_local_app_content_bundle(
            self.app.clone(),
            UploadLocalAppContentArgs {
                source_app_id: app_id.to_string(),
                destination_content_prefix: prefix,
                credentials,
            },
        )
        .await
        .map_err(|error| workspace_failed(app_id, "copy", error))?;
        match uploaded.failures.first() {
            None => Ok(()),
            Some(failure) => Err(HubError::failed(
                WORKSPACE_FAILED,
                format!(
                    "{} of the files of workspace {app_id} could not be copied, such as {}: {}",
                    uploaded.failures.len(),
                    failure.kind_and_path,
                    failure.reason
                ),
            )),
        }
    }

    async fn workspace_differences(
        &self,
        app_id: &str,
        grant: &Value,
    ) -> Result<Vec<String>, HubError> {
        let (prefix, credentials) = upload_target(grant)?;
        let failed = |reason: String| workspace_failed(app_id, "compare", reason);
        let local_store = TauriFlowLikeState::get_project_storage_store(&self.app)
            .await
            .map_err(|error| failed(error.to_string()))?;
        let remote_store = credentials
            .to_store(false)
            .await
            .map_err(|error| failed(error.to_string()))?
            .as_generic();
        let local = summarize_store_prefix(&local_store, &Path::from("apps").join(app_id))
            .await
            .map_err(|error| failed(error.to_string()))?;
        let remote: HashMap<String, u64> =
            summarize_store_prefix(&remote_store, &parse_storage_path(&prefix))
                .await
                .map_err(|error| failed(error.to_string()))?
                .into_iter()
                .collect();
        let mut missing: Vec<String> = local
            .into_iter()
            .filter(|(path, size)| is_uploaded_content_path(path) && remote.get(path) != Some(size))
            .map(|(path, _)| path)
            .collect();
        missing.sort();
        missing.truncate(WORKSPACE_FILES_SHOWN);
        Ok(missing)
    }

    async fn delete_workspace(&self, app_id: &str) -> Result<(), HubError> {
        let host = DesktopWorkspaces::new(&self.app);
        let deleted = async {
            if host.app_exists(app_id).await? {
                host.delete_app(app_id).await?;
            }
            Ok::<(), String>(())
        };
        deleted
            .await
            .map_err(|error| workspace_failed(app_id, "delete", error))
    }
}

fn workspace_failed(app_id: &str, action: &str, reason: impl std::fmt::Display) -> HubError {
    HubError::failed(
        WORKSPACE_FAILED,
        format!("Could not {action} workspace {app_id}: {reason}"),
    )
}

/// Where the hub wants a workspace's files and the scoped credentials to write them.
fn upload_target(grant: &Value) -> Result<(String, Arc<SharedCredentials>), HubError> {
    let invalid = |reason: &str| {
        HubError::failed(
            HUB_RESPONSE_INVALID,
            format!("the hub's workspace upload is unusable: {reason}"),
        )
    };
    let prefix = grant
        .get("upload_path")
        .and_then(Value::as_str)
        .filter(|path| !path.is_empty())
        .ok_or_else(|| invalid("it names no upload_path"))?;
    let credentials = grant
        .get("shared_credentials")
        .cloned()
        .ok_or_else(|| invalid("it carries no shared_credentials"))
        .and_then(|value| {
            serde_json::from_value::<SharedCredentials>(value)
                .map_err(|_| invalid("its shared_credentials can't be read"))
        })?;
    Ok((prefix.to_string(), Arc::new(credentials)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_refusal_carries_the_hubs_code_and_message() {
        let body = json!({ "error": { "code": "BOT_LIMIT_REACHED", "message": "You already have 50 bots" } });
        assert_eq!(
            answer(409, body.to_string().as_bytes()).unwrap_err(),
            HubError::answered(
                409,
                Some("BOT_LIMIT_REACHED".into()),
                "You already have 50 bots"
            )
        );
        let long = json!({ "error": { "code": "ERROR", "message": "x".repeat(1_000) } });
        let error = answer(500, long.to_string().as_bytes()).unwrap_err();
        assert_eq!(error.message.chars().count(), HUB_MESSAGE_MAX_CHARS);
        assert_eq!(
            answer(502, b"<html>Bad gateway</html>").unwrap_err(),
            HubError::answered(502, None, "the hub answered HTTP 502")
        );
    }

    #[test]
    fn a_success_is_its_json_or_nothing() {
        assert_eq!(
            answer(200, br#"{ "id": "bot_1" }"#).unwrap(),
            Some(json!({ "id": "bot_1" }))
        );
        assert_eq!(answer(204, b"").unwrap(), None);
        assert_eq!(
            answer(200, b"not json").unwrap_err().code.as_deref(),
            Some(HUB_RESPONSE_INVALID)
        );
    }

    #[test]
    fn calls_keep_their_method() {
        assert_eq!(method(HubMethod::Patch), reqwest::Method::PATCH);
        assert_eq!(method(HubMethod::Delete), reqwest::Method::DELETE);
    }
}

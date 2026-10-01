//! Subscription-backed model adapters. Authentication never starts an interactive login.

mod microsoft;

use super::{ModelConstructor, ModelLogic, responses_tools::NonStrictToolsClient};
use crate::{history::History, provider::ModelProvider};
use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rig::providers::{chatgpt, copilot};
use serde_json::Value;
use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const AUTH_TIMEOUT: Duration = Duration::from_secs(15);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(300);
const GITHUB_TOKEN_URL: &str = "https://api.github.com/copilot_internal/v2/token";

pub(super) fn param<'a>(provider: &'a ModelProvider, name: &str) -> Option<&'a str> {
    provider
        .params
        .as_ref()?
        .get(name)?
        .as_str()
        .map(str::trim)
        .filter(|v| !v.is_empty())
}

fn model_id(provider: &ModelProvider, fallback: &str) -> String {
    param(provider, "model_id")
        .or_else(|| {
            provider
                .model_id
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
        })
        .unwrap_or(fallback)
        .to_owned()
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn jwt_claims(token: &str) -> Option<Value> {
    let payload = token.split('.').nth(1)?;
    serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).ok()?).ok()
}

fn codex_account_id(token: &str) -> Option<String> {
    jwt_claims(token)?
        .get("https://api.openai.com/auth")?
        .get("chatgpt_account_id")?
        .as_str()
        .map(str::to_owned)
}

/// Expiry inspection is a readiness check, not signature verification or authorization.
pub(super) fn validate_token(token: &str, provider: &str) -> Result<()> {
    if token.trim().is_empty() {
        bail!("{provider} requires an access token");
    }
    http::HeaderValue::from_str(token)
        .map_err(|_| anyhow!("{provider} token is not a valid HTTP header value"))?;
    if jwt_claims(token)
        .and_then(|v| v.get("exp")?.as_u64())
        .is_some_and(|exp| exp <= unix_seconds() + 60)
    {
        bail!("{provider} access token has expired or expires within one minute; sign in again");
    }
    Ok(())
}

fn codex_auth_record(bytes: &[u8]) -> Result<chatgpt::ChatGPTAuth> {
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|_| anyhow!("Codex auth file is not valid JSON; sign in with Codex again"))?;
    let tokens = value.get("tokens").unwrap_or(&value);
    let token = tokens.get("access_token").and_then(Value::as_str)
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| anyhow!("Codex has no cached ChatGPT access token; sign in with Codex or supply an access token"))?;
    validate_token(token, "Codex")?;
    if tokens
        .get("expires_at")
        .and_then(Value::as_u64)
        .is_some_and(|exp| exp <= unix_seconds() + 60)
    {
        bail!("Codex access token has expired; sign in with Codex again");
    }
    let account_id = tokens
        .get("account_id")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| codex_account_id(token));
    Ok(chatgpt::ChatGPTAuth::AccessToken {
        access_token: token.to_owned(),
        account_id,
    })
}

fn codex_auth(provider: &ModelProvider) -> Result<chatgpt::ChatGPTAuth> {
    if let Some(token) = param(provider, "access_token") {
        validate_token(token, "Codex")?;
        return Ok(chatgpt::ChatGPTAuth::AccessToken {
            access_token: token.to_owned(),
            account_id: param(provider, "account_id")
                .map(str::to_owned)
                .or_else(|| codex_account_id(token)),
        });
    }
    #[cfg(not(target_family = "wasm"))]
    {
        use std::{fs::File, io::Read, path::PathBuf};
        let directory = std::env::var_os("CODEX_HOME").filter(|v| !v.is_empty()).map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(|home| PathBuf::from(home).join(".codex")))
            .ok_or_else(|| anyhow!("Codex access token is required because the local Codex auth directory is unavailable"))?;
        // Never pass this file to Rig's OAuth authenticator: it can rewrite the cache or launch login.
        let path = directory.join("auth.json");
        if !std::fs::metadata(&path)
            .map_err(|_| {
                anyhow!("Codex login is unavailable; sign in with Codex or supply an access token")
            })?
            .is_file()
        {
            bail!("Codex auth path is not a regular file");
        }
        let file = File::open(path).map_err(|_| {
            anyhow!("Codex login is unavailable; sign in with Codex or supply an access token")
        })?;
        let mut bytes = Vec::new();
        file.take(1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| anyhow!("Cannot read the local Codex auth file"))?;
        if bytes.len() > 1024 * 1024 {
            bail!("Codex auth file exceeds the size limit");
        }
        codex_auth_record(&bytes)
    }
    #[cfg(target_family = "wasm")]
    bail!("Codex requires an explicit access token in this runtime")
}

pub(super) fn http_client(timeout: Duration) -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::none())
        .build()?)
}

fn rig_http_client() -> Result<reqwest_rig::Client> {
    Ok(reqwest_rig::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .redirect(reqwest_rig::redirect::Policy::none())
        .build()?)
}

fn codex_client_builder(
    provider: &ModelProvider,
) -> Result<chatgpt::ClientBuilder<reqwest_rig::Client>> {
    Ok(chatgpt::Client::builder()
        .api_key(codex_auth(provider)?)
        .originator("flow-like")
        .http_client(rig_http_client()?))
}

async fn github_auth(provider: &ModelProvider) -> Result<(String, String)> {
    if let Some(token) = param(provider, "api_key") {
        validate_token(token, "GitHub Copilot")?;
        return Ok((token.to_owned(), "https://api.githubcopilot.com".to_owned()));
    }
    let access_token = param(provider, "access_token").ok_or_else(|| {
        anyhow!("GitHub Copilot requires a GitHub access token or Copilot API token")
    })?;
    validate_token(access_token, "GitHub Copilot")?;
    // Rig's bootstrap path writes a shared disk cache. Exchange into an in-memory token instead.
    let response = http_client(AUTH_TIMEOUT)?
        .get(GITHUB_TOKEN_URL)
        .header("authorization", format!("token {access_token}"))
        .header("accept", "application/json")
        .header("editor-version", "vscode/1.107.0")
        .header("editor-plugin-version", "copilot-chat/0.35.0")
        .header("user-agent", "GitHubCopilotChat/0.35.0")
        .send()
        .await
        .context("GitHub Copilot token exchange failed")?;
    if !response.status().is_success() {
        bail!(
            "GitHub Copilot token exchange returned HTTP {}; check the token and Copilot entitlement",
            response.status()
        );
    }
    let value: Value = response
        .json()
        .await
        .context("GitHub Copilot token response is invalid")?;
    let token = value
        .get("token")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("GitHub Copilot returned no API token"))?;
    validate_token(token, "GitHub Copilot")?;
    let endpoint = value
        .pointer("/endpoints/api")
        .and_then(Value::as_str)
        .unwrap_or("https://api.githubcopilot.com");
    validate_copilot_endpoint(endpoint)?;
    Ok((token.to_owned(), endpoint.to_owned()))
}

fn validate_copilot_endpoint(endpoint: &str) -> Result<()> {
    let url =
        reqwest::Url::parse(endpoint).context("GitHub Copilot returned an invalid API endpoint")?;
    let trusted = url
        .host_str()
        .is_some_and(|host| host == "githubcopilot.com" || host.ends_with(".githubcopilot.com"));
    if url.scheme() != "https"
        || !trusted
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        bail!("GitHub Copilot returned an untrusted API endpoint");
    }
    Ok(())
}

async fn check_github_model(provider: &ModelProvider) -> Result<()> {
    let (token, endpoint) = github_auth(provider).await?;
    let response = http_client(AUTH_TIMEOUT)?
        .get(format!("{}/models", endpoint.trim_end_matches('/')))
        .bearer_auth(token)
        .header("editor-version", "vscode/1.107.0")
        .header("editor-plugin-version", "copilot-chat/0.35.0")
        .header("user-agent", "GitHubCopilotChat/0.35.0")
        .send()
        .await
        .context("GitHub Copilot model discovery failed")?;
    if !response.status().is_success() {
        bail!(
            "GitHub Copilot model discovery returned HTTP {}; check the token and Copilot entitlement",
            response.status()
        );
    }
    let value: Value = response
        .json()
        .await
        .context("GitHub Copilot model list is invalid")?;
    let model = model_id(provider, copilot::GPT_4_1);
    let models = value
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("GitHub Copilot returned no model list"))?;
    if !models
        .iter()
        .any(|entry| entry.get("id").and_then(Value::as_str) == Some(model.as_str()))
    {
        bail!("The selected model is not available in this GitHub Copilot account");
    }
    Ok(())
}

enum Client {
    Codex(chatgpt::Client),
    GitHub(copilot::Client),
}

struct SubscriptionModel {
    client: Client,
    model: String,
}

#[async_trait]
impl ModelLogic for SubscriptionModel {
    async fn provider(&self) -> Result<ModelConstructor> {
        Ok(ModelConstructor {
            inner: match &self.client {
                Client::Codex(client) => Box::new(NonStrictToolsClient(client.clone())),
                Client::GitHub(client) => Box::new(client.clone()),
            },
        })
    }

    async fn default_model(&self) -> Option<String> {
        Some(self.model.clone())
    }

    fn request_params(&self, history: &History, _streaming: bool) -> Result<Option<Value>> {
        subscription_params(
            history,
            matches!(self.client, Client::Codex(_)),
            &self.model,
        )
    }
}

fn subscription_params(history: &History, codex: bool, model: &str) -> Result<Option<Value>> {
    let mut params = history
        .build_additional_params()?
        .unwrap_or_else(|| serde_json::json!({}));
    if let Some(params) = params.as_object_mut() {
        // Rig owns transport selection. These two fields belong only to OpenRouter.
        for key in ["stream", "stream_options", "usage", "preset"] {
            params.remove(key);
        }
        if codex {
            for key in [
                "top_p",
                "presence_penalty",
                "frequency_penalty",
                "stop",
                "user",
                "seed",
                "n",
                "response_format",
            ] {
                if params.contains_key(key) {
                    bail!(
                        "Codex subscription provider does not support the '{key}' history setting"
                    );
                }
            }
        }
        if let Some(thinking) = history.thinking {
            let effort = thinking.openai_reasoning_effort();
            if codex || model.to_ascii_lowercase().contains("codex") {
                params.insert("reasoning".into(), serde_json::json!({"effort": effort}));
            } else {
                params.insert("reasoning_effort".into(), serde_json::json!(effort));
            }
        }
    }
    Ok(Some(params))
}

/// Builds a provider without opening login or writing provider credential files.
pub async fn build(provider: &ModelProvider) -> Result<Arc<dyn ModelLogic>> {
    match provider.provider_name.trim().to_ascii_lowercase().as_str() {
        "custom:codex" => {
            let client = codex_client_builder(provider)?.build()?;
            Ok(Arc::new(SubscriptionModel {
                client: Client::Codex(client),
                model: model_id(provider, chatgpt::GPT_5_3_CODEX),
            }))
        }
        "custom:github-copilot" => {
            let (token, endpoint) = github_auth(provider).await?;
            let client = copilot::Client::builder()
                .api_key(copilot::CopilotAuth::ApiKey(token))
                .base_url(endpoint)
                .http_client(rig_http_client()?)
                .build()?;
            Ok(Arc::new(SubscriptionModel {
                client: Client::GitHub(client),
                model: model_id(provider, copilot::GPT_4_1),
            }))
        }
        "custom:microsoft-copilot" => Ok(Arc::new(microsoft::MicrosoftModel::new(provider)?)),
        name => bail!("Unsupported subscription provider '{name}'"),
    }
}

/// Checks credential readiness without generating a completion or starting login.
/// Token presence and expiry do not prove model entitlement, available quota, or Graph consent.
pub async fn check_available(provider: &ModelProvider) -> Result<()> {
    tokio::time::timeout(AUTH_TIMEOUT, async {
        match provider.provider_name.trim().to_ascii_lowercase().as_str() {
            "custom:codex" => {
                codex_auth(provider)?;
            }
            "custom:github-copilot" => {
                check_github_model(provider).await?;
            }
            "custom:microsoft-copilot" => {
                microsoft::MicrosoftModel::new(provider)?;
            }
            name => bail!("Unsupported subscription provider '{name}'"),
        }
        Ok(())
    })
    .await
    .map_err(|_| anyhow!("Provider availability check timed out"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn provider() -> ModelProvider {
        ModelProvider {
            provider_name: "custom:codex".into(),
            model_id: Some("fallback".into()),
            version: None,
            api_surface: None,
            params: Some(std::collections::HashMap::from([(
                "model_id".into(),
                json!("selected"),
            )])),
        }
    }

    #[test]
    fn model_pin_precedes_persisted_fallback() {
        let mut provider = provider();
        assert_eq!(model_id(&provider, "default"), "selected");
        provider.params = None;
        assert_eq!(model_id(&provider, "default"), "fallback");
    }

    #[test]
    fn parses_cli_tokens_without_refreshing_or_accepting_api_keys() {
        let auth = codex_auth_record(
            br#"{"tokens":{"access_token":"test-token","account_id":"account"}}"#,
        )
        .unwrap();
        assert!(
            matches!(auth, chatgpt::ChatGPTAuth::AccessToken { account_id: Some(id), .. } if id == "account")
        );
        assert!(codex_auth_record(br#"{"OPENAI_API_KEY":"test-key"}"#).is_err());
        assert!(
            codex_auth_record(br#"{"tokens":{"access_token":"test-token","expires_at":1}}"#)
                .is_err()
        );
    }

    #[test]
    fn expired_tokens_and_header_injection_are_rejected() {
        let token = format!(
            "header.{}.signature",
            URL_SAFE_NO_PAD.encode(br#"{"exp":1}"#)
        );
        assert!(validate_token(&token, "test").is_err());
        assert!(validate_token("bad\r\nheader", "test").is_err());
        assert!(validate_token("opaque", "test").is_ok());
    }

    #[test]
    fn exchanged_token_cannot_be_redirected_to_untrusted_host() {
        assert!(validate_copilot_endpoint("https://api.individual.githubcopilot.com").is_ok());
        for endpoint in [
            "http://api.githubcopilot.com",
            "https://githubcopilot.com.evil.test",
            "https://api.githubcopilot.com@evil.test",
            "https://api.githubcopilot.com/?token=x",
        ] {
            assert!(validate_copilot_endpoint(endpoint).is_err());
        }
    }

    #[test]
    fn maps_reasoning_to_the_rig_transport_dialect() {
        let mut history = History::new("model".into(), vec![]);
        history.thinking = Some(crate::history::HistoryThinking::Mid);
        let codex = subscription_params(&history, true, "model")
            .unwrap()
            .unwrap();
        assert_eq!(codex["reasoning"]["effort"], "medium");
        let github_chat = subscription_params(&history, false, "gpt-5.4")
            .unwrap()
            .unwrap();
        assert_eq!(github_chat["reasoning_effort"], "medium");
        let github_codex = subscription_params(&history, false, "gpt-5.3-codex")
            .unwrap()
            .unwrap();
        assert_eq!(github_codex["reasoning"]["effort"], "medium");
        assert!(codex.get("stream").is_none());
    }

    #[tokio::test]
    async fn codex_sends_tool_schemas_as_written() {
        use crate::llm::test_support::{
            assert_sends_tool_as_written, serve_once, sse_response, typeless_tool,
        };
        use futures::StreamExt;
        use rig::completion::Completion;

        let (endpoint, server) = serve_once(sse_response(&[])).await;
        let mut provider = provider();
        provider
            .params
            .as_mut()
            .unwrap()
            .insert("access_token".into(), json!("test-token"));
        let client = codex_client_builder(&provider)
            .unwrap()
            .base_url(&endpoint)
            .build()
            .unwrap();
        let model = SubscriptionModel {
            client: Client::Codex(client),
            model: model_id(&provider, chatgpt::GPT_5_3_CODEX),
        };
        let agent = model
            .provider()
            .await
            .unwrap()
            .into_client()
            .agent(&model.model)
            .build();

        let mut stream = agent
            .completion("Fill in the form.", Vec::<rig::completion::Message>::new())
            .await
            .unwrap()
            .tools(vec![typeless_tool()])
            .stream()
            .await
            .unwrap();
        let _ = stream.next().await;

        let body: Value = serde_json::from_str(&server.await.unwrap()).unwrap();
        assert_sends_tool_as_written(&body, &typeless_tool());
    }

    #[tokio::test]
    async fn availability_rejects_missing_explicit_credentials_without_network() {
        let mut provider = provider();
        provider.provider_name = "custom:github-copilot".into();
        assert!(
            check_available(&provider)
                .await
                .unwrap_err()
                .to_string()
                .contains("requires")
        );
        provider.provider_name = "custom:microsoft-copilot".into();
        assert!(
            check_available(&provider)
                .await
                .unwrap_err()
                .to_string()
                .contains("requires")
        );
    }
}

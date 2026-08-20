use std::{any::Any, future::Future};

use super::{LLMCallback, ModelLogic};
use crate::provider::random_provider;
use crate::{
    history::History,
    llm::ModelConstructor,
    provider::{ModelProvider, ModelProviderConfiguration, VertexConfig},
    response::Response,
};
use flow_like_types::{Cacheable, Result, anyhow, async_trait};
use google_cloud_auth::credentials::{
    CacheableResource, Credentials, CredentialsProvider, EntityTag, service_account,
};
use google_cloud_auth::errors::CredentialsError;
use http::{Extensions, HeaderMap, HeaderValue, header::AUTHORIZATION};
use rig::completion::CompletionModel;

#[derive(Clone, Debug)]
struct StaticAccessTokenCredentials {
    token: String,
}

impl CredentialsProvider for StaticAccessTokenCredentials {
    fn headers(
        &self,
        _extensions: Extensions,
    ) -> impl Future<Output = std::result::Result<CacheableResource<HeaderMap>, CredentialsError>> + Send
    {
        let token = self.token.clone();
        async move {
            let mut headers = HeaderMap::new();
            let value = HeaderValue::from_str(&format!("Bearer {token}")).map_err(|e| {
                CredentialsError::from_msg(false, format!("Invalid Vertex access token: {e}"))
            })?;
            headers.insert(AUTHORIZATION, value);
            Ok(CacheableResource::New {
                entity_tag: EntityTag::new(),
                data: headers,
            })
        }
    }

    async fn universe_domain(&self) -> Option<String> {
        Some("googleapis.com".to_string())
    }
}

pub struct VertexModel {
    client: rig_vertexai::Client,
    _provider: ModelProvider,
    default_model: Option<String>,
}

impl VertexModel {
    pub async fn new(
        provider: &ModelProvider,
        config: &ModelProviderConfiguration,
    ) -> flow_like_types::Result<Self> {
        let vertex_config = if config.vertex_config.is_empty() {
            VertexConfig::default()
        } else {
            random_provider(&config.vertex_config)?
        };

        let client = build_client(
            vertex_config.project_id.as_deref(),
            vertex_config.location.as_deref(),
            vertex_config.service_account_json.as_deref(),
            vertex_config.access_token.as_deref(),
        )?;

        Ok(VertexModel {
            client,
            _provider: provider.clone(),
            default_model: provider.model_id.clone(),
        })
    }

    pub async fn from_provider(provider: &ModelProvider) -> flow_like_types::Result<Self> {
        let params = provider.params.clone().unwrap_or_default();
        let project_id =
            string_param(&params, "project_id").or_else(|| string_param(&params, "project"));
        let location =
            string_param(&params, "location").or_else(|| string_param(&params, "region"));
        let service_account_json = string_param(&params, "service_account_json")
            .or_else(|| string_param(&params, "service_account_key"));
        let access_token = string_param(&params, "access_token");
        let model_id = string_param(&params, "model_id").or_else(|| provider.model_id.clone());

        let project_id = project_id.or_else(|| {
            service_account_json
                .as_deref()
                .and_then(service_account_project_id)
        });

        let client = build_client(
            project_id.as_deref(),
            location.as_deref(),
            service_account_json.as_deref(),
            access_token.as_deref(),
        )?;

        Ok(VertexModel {
            client,
            default_model: model_id,
            _provider: provider.clone(),
        })
    }
}

impl Cacheable for VertexModel {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[async_trait]
impl ModelLogic for VertexModel {
    #[allow(deprecated)]
    async fn provider(&self) -> Result<ModelConstructor> {
        Ok(ModelConstructor {
            inner: Box::new(self.client.clone()),
        })
    }

    async fn default_model(&self) -> Option<String> {
        self.default_model.clone()
    }

    #[allow(deprecated)]
    async fn invoke(&self, history: &History, lambda: Option<LLMCallback>) -> Result<Response> {
        use crate::llm::{CompletionModelHandle, emit_response_to_callback, invoke_without_stream};
        use std::sync::Arc;

        let mut history = history.clone();
        history.normalize_for_alternation();
        history.stream = Some(false);

        let model_name = self
            .default_model()
            .await
            .unwrap_or_else(|| history.model.clone());

        let constructor = self.provider().await?;
        let completion_model = constructor.inner.completion_model(&model_name);
        let completion_handle = CompletionModelHandle::new(Arc::from(completion_model));

        let system_prompt = history.take_system_prompt();
        let (prompt, chat_history) = history
            .extract_prompt_and_history()
            .map_err(|e| anyhow!("Failed to convert history into rig messages: {e}"))?;

        let mut builder =
            CompletionModel::completion_request(&completion_handle, prompt).messages(chat_history);

        if let Some(preamble) = system_prompt {
            builder = builder.preamble(preamble);
        }

        if let Some(temp) = history.temperature {
            builder = builder.temperature(temp as f64);
        }

        if let Some(max_tokens) = history.max_completion_tokens {
            builder = builder.max_tokens(max_tokens as u64);
        }

        if history.tools.is_some() {
            let tool_definitions = history.tools_to_rig()?;
            if !tool_definitions.is_empty() {
                builder = builder.tools(tool_definitions);
            }
        }

        if let Some(choice) = history.tool_choice_to_rig() {
            builder = builder.tool_choice(choice);
        }

        if let Some(params) = history.build_additional_params()? {
            builder = builder.additional_params(params);
        }

        let response = invoke_without_stream(builder, &model_name, None).await?;

        if let Some(callback) = lambda {
            emit_response_to_callback(&response, callback, &model_name).await?;
        }

        Ok(response)
    }
}

fn string_param(
    params: &std::collections::HashMap<String, flow_like_types::Value>,
    key: &str,
) -> Option<String> {
    params
        .get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn service_account_project_id(service_account_json: &str) -> Option<String> {
    flow_like_types::json::from_str::<flow_like_types::Value>(service_account_json)
        .ok()
        .and_then(|value| {
            value
                .get("project_id")
                .and_then(|project_id| project_id.as_str())
                .map(str::trim)
                .filter(|project_id| !project_id.is_empty())
                .map(ToOwned::to_owned)
        })
}

fn credentials_from_params(
    service_account_json: Option<&str>,
    access_token: Option<&str>,
) -> Result<Option<Credentials>> {
    if let Some(service_account_json) = service_account_json {
        let key = flow_like_types::json::from_str::<flow_like_types::Value>(service_account_json)
            .map_err(|e| anyhow!("Invalid Vertex service account JSON: {e}"))?;
        let credentials = service_account::Builder::new(key)
            .build()
            .map_err(|e| anyhow!("Failed to build Vertex service account credentials: {e}"))?;
        return Ok(Some(credentials));
    }

    if let Some(access_token) = access_token {
        return Ok(Some(Credentials::from(StaticAccessTokenCredentials {
            token: access_token.to_string(),
        })));
    }

    Ok(None)
}

fn build_client(
    project_id: Option<&str>,
    location: Option<&str>,
    service_account_json: Option<&str>,
    access_token: Option<&str>,
) -> Result<rig_vertexai::Client> {
    let project_id = non_empty(project_id);
    let location = non_empty(location);
    let service_account_json = non_empty(service_account_json);
    let access_token = non_empty(access_token);

    let mut builder = rig_vertexai::Client::builder();

    if let Some(project_id) = project_id {
        builder = builder.with_project(project_id);
    }

    if let Some(location) = location {
        builder = builder.with_location(location);
    }

    if let Some(credentials) = credentials_from_params(service_account_json, access_token)? {
        builder = builder.with_credentials(credentials);
    }

    builder
        .build()
        .map_err(|e| anyhow!("Failed to build Vertex AI client: {e}"))
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

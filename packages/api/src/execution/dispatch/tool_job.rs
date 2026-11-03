//! Bot tool jobs: one synchronous call to the executor's `POST /tool`.

use std::time::Duration;

use flow_like_bot_tools_host::{ToolJobEnvelope, ToolReply};

use super::{DispatchError, Dispatcher, ExecutionBackend, attach_executor_iam_auth};

const TOOL_URL_VAR: &str = "EXECUTOR_TOOL_URL";

/// Where tool jobs go: an executor reached over HTTP, or the executor Lambda by direct invoke.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolTarget {
    Http(String),
    #[cfg(feature = "lambda")]
    Lambda(String),
}

fn configured_url(
    tool_url: Option<&str>,
    backend: &ExecutionBackend,
    executor_url: Option<&str>,
) -> Option<String> {
    let clean = |value: &str| {
        let value = value.trim().trim_end_matches('/');
        (!value.is_empty()).then(|| value.to_string())
    };
    tool_url.and_then(clean).or_else(|| {
        (*backend == ExecutionBackend::Http)
            .then_some(executor_url)
            .flatten()
            .and_then(clean)
    })
}

impl Dispatcher {
    /// The executor a tool job can reach with this deployment's configuration, if any.
    pub fn tool_target(&self) -> Option<ToolTarget> {
        let tool_url = std::env::var(TOOL_URL_VAR).ok();
        if let Some(url) = configured_url(
            tool_url.as_deref(),
            &self.config.backend,
            self.config.executor_url.as_deref(),
        ) {
            return Some(ToolTarget::Http(url));
        }
        #[cfg(feature = "lambda")]
        if self.config.backend == ExecutionBackend::LambdaStream
            && self.lambda_client.is_some()
            && let Some(function) = &self.config.lambda_function_name
        {
            return Some(ToolTarget::Lambda(function.clone()));
        }
        None
    }

    /// Run `envelope` on the executor. `user_id` is the bot's owner: on Lambda it picks the
    /// tenant environment, like a run of that user.
    pub async fn invoke_tool(
        &self,
        user_id: &str,
        envelope: &ToolJobEnvelope,
        timeout: Duration,
    ) -> Result<ToolReply, DispatchError> {
        let target = self.tool_target().ok_or_else(|| {
            DispatchError::Configuration("No executor is configured for bot tools".into())
        })?;
        let body = match target {
            ToolTarget::Http(url) => self.tool_http(&url, envelope, timeout).await?,
            #[cfg(feature = "lambda")]
            ToolTarget::Lambda(function) => {
                self.tool_lambda(&function, user_id, envelope, timeout)
                    .await?
            }
        };
        serde_json::from_slice(&body).map_err(|error| {
            DispatchError::Serialization(format!("The executor's tool reply is malformed: {error}"))
        })
    }

    async fn tool_http(
        &self,
        url: &str,
        envelope: &ToolJobEnvelope,
        timeout: Duration,
    ) -> Result<bytes::Bytes, DispatchError> {
        let response = attach_executor_iam_auth(self.http_client.post(format!("{url}/tool")), url)
            .await?
            .timeout(timeout)
            .json(envelope)
            .send()
            .await
            .map_err(|error| DispatchError::Network(error.to_string()))?;
        let status = response.status();
        if status.is_success() {
            return response
                .bytes()
                .await
                .map_err(|error| DispatchError::Network(error.to_string()));
        }
        let text = response.text().await.unwrap_or_default();
        Err(if status.is_client_error() {
            DispatchError::Rejected(format!("HTTP {status}: {text}"))
        } else {
            DispatchError::Network(format!("HTTP {status}: {text}"))
        })
    }

    #[cfg(feature = "lambda")]
    async fn tool_lambda(
        &self,
        function: &str,
        user_id: &str,
        envelope: &ToolJobEnvelope,
        timeout: Duration,
    ) -> Result<bytes::Bytes, DispatchError> {
        use tracing::Instrument;

        let client = self
            .lambda_client
            .as_ref()
            .ok_or_else(|| DispatchError::Configuration("Lambda client not initialized".into()))?;
        let body = serde_json::to_value(envelope)
            .map_err(|error| DispatchError::Serialization(error.to_string()))?;
        let payload = serde_json::to_vec(&super::wrap_as_apigw_v2_event("/tool", body))
            .map_err(|error| DispatchError::Serialization(error.to_string()))?;
        let tenant_id = self.lambda_tenant_id_for_user(user_id)?;
        let mut invoke = client
            .invoke()
            .function_name(function)
            .invocation_type(aws_sdk_lambda::types::InvocationType::RequestResponse)
            .payload(aws_sdk_lambda::primitives::Blob::new(payload));
        if let Some(tenant_id) = tenant_id.as_deref() {
            invoke = invoke.tenant_id(tenant_id);
        }
        let response = flow_like_types::tokio::time::timeout(
            timeout,
            invoke.send().instrument(tracing::info_span!(
                target: "flow_like::observability",
                "aws.lambda.invoke_tool",
                cloud.service = "lambda",
                rpc.service = "Lambda",
                rpc.method = "Invoke"
            )),
        )
        .await
        .map_err(|_| DispatchError::Network("The tool job timed out".into()))?
        .map_err(|error| super::lambda_dispatch_error(error, tenant_id.is_some()))?;
        if let Some(error) = response.function_error() {
            return Err(DispatchError::ExecutorFailed(format!("Lambda {error}")));
        }
        let payload = response
            .payload()
            .map(|blob| blob.as_ref().to_vec())
            .unwrap_or_default();
        gateway_body(&payload)
    }
}

#[cfg(feature = "lambda")]
fn gateway_body(payload: &[u8]) -> Result<bytes::Bytes, DispatchError> {
    let value: serde_json::Value = serde_json::from_slice(payload)
        .map_err(|error| DispatchError::Serialization(error.to_string()))?;
    let status = value["statusCode"].as_u64().unwrap_or(500);
    let body = value["body"].as_str().unwrap_or_default();
    let body = if value["isBase64Encoded"].as_bool() == Some(true) {
        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, body)
            .map_err(|error| DispatchError::Serialization(error.to_string()))?
    } else {
        body.as_bytes().to_vec()
    };
    match status {
        200..=299 => Ok(bytes::Bytes::from(body)),
        400..=499 => Err(DispatchError::Rejected(format!(
            "HTTP {status}: {}",
            String::from_utf8_lossy(&body)
        ))),
        _ => Err(DispatchError::Network(format!(
            "HTTP {status}: {}",
            String::from_utf8_lossy(&body)
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tool_url_wins_and_the_executor_url_serves_only_the_http_backend() {
        assert_eq!(
            configured_url(
                Some("https://tools.example/"),
                &ExecutionBackend::LambdaStream,
                None
            ),
            Some("https://tools.example".to_string())
        );
        assert_eq!(
            configured_url(None, &ExecutionBackend::Http, Some("http://executor:8080/")),
            Some("http://executor:8080".to_string())
        );
        assert_eq!(
            configured_url(
                None,
                &ExecutionBackend::LambdaStream,
                Some("http://executor:8080")
            ),
            None
        );
        assert_eq!(
            configured_url(Some(" "), &ExecutionBackend::Http, None),
            None
        );
    }
}

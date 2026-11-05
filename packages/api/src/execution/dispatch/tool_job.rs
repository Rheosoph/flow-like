//! Bot tool jobs: one synchronous call to the executor's `POST /tool`.

use std::time::Duration;

use flow_like_bot_tools_host::{ToolJobEnvelope, ToolReply};

use super::{DispatchError, Dispatcher, ExecutionBackend, attach_executor_iam_auth};

const TOOL_URL_VAR: &str = "EXECUTOR_TOOL_URL";
// Four displayed images may each contain 4 MiB before base64 encoding.
#[cfg(any(feature = "lambda", test))]
const MAX_TOOL_REPLY_BYTES: usize = 32 * 1024 * 1024;

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

fn tool_http_error(error: reqwest::Error) -> DispatchError {
    if error.is_timeout() {
        DispatchError::ToolTimeout
    } else {
        DispatchError::Network(error.to_string())
    }
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
            .map_err(tool_http_error)?;
        let status = response.status();
        if status.is_success() {
            return response.bytes().await.map_err(tool_http_error);
        }
        let text = response.text().await.map_err(tool_http_error)?;
        Err(DispatchError::ToolHttp {
            status: status.as_u16(),
            message: text,
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
            .invoke_with_response_stream()
            .function_name(function)
            .payload(aws_sdk_lambda::primitives::Blob::new(payload));
        if let Some(tenant_id) = tenant_id.as_deref() {
            invoke = invoke.tenant_id(tenant_id);
        }
        // The executor uses lambda_http's streaming adapter for every route. Its response
        // is an HTTP metadata prelude followed by the tool JSON, including on /tool.
        flow_like_types::tokio::time::timeout(timeout, async {
            let response = invoke
                .send()
                .instrument(tracing::info_span!(
                    target: "flow_like::observability",
                    "aws.lambda.invoke_tool",
                    cloud.service = "lambda",
                    rpc.service = "Lambda",
                    rpc.method = "InvokeWithResponseStream"
                ))
                .await
                .map_err(|error| super::lambda_dispatch_error(error, tenant_id.is_some()))?;
            let stream =
                futures::stream::unfold(Some(response.event_stream), |receiver| async move {
                    let mut receiver = receiver?;
                    let (chunk, more) = super::lambda_stream_chunk(receiver.recv().await)?;
                    Some((chunk, more.then_some(receiver)))
                });
            lambda_tool_body(Box::pin(stream), MAX_TOOL_REPLY_BYTES).await
        })
        .await
        .map_err(|_| DispatchError::ToolTimeout)?
    }
}

#[cfg(any(feature = "lambda", test))]
async fn lambda_tool_body(
    mut stream: super::ByteStream,
    limit: usize,
) -> Result<bytes::Bytes, DispatchError> {
    use futures::StreamExt;

    let mut body = bytes::BytesMut::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if chunk.len() > limit.saturating_sub(body.len()) {
            return Err(DispatchError::Serialization(format!(
                "The executor's tool reply exceeds {limit} bytes"
            )));
        }
        body.extend_from_slice(&chunk);
    }
    // Read through InvokeComplete before accepting output: Lambda may report a crash
    // after it has already sent some or all of the response body.
    let body = body.freeze();
    match super::scan_lambda_http_prelude(&body) {
        super::LambdaPreludeScan::Complete {
            status,
            body_offset,
        } => {
            let body = body.slice(body_offset..);
            if (200..300).contains(&status) {
                Ok(body)
            } else {
                let end = body.len().min(super::LAMBDA_PRELUDE_ERROR_BODY_CAP);
                Err(DispatchError::ToolHttp {
                    status,
                    message: String::from_utf8_lossy(&body[..end]).into_owned(),
                })
            }
        }
        _ => Ok(body),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::tokio;

    fn byte_stream(chunks: &[&[u8]]) -> super::super::ByteStream {
        let chunks: Vec<_> = chunks
            .iter()
            .map(|chunk| Ok(bytes::Bytes::copy_from_slice(chunk)))
            .collect();
        Box::pin(futures::stream::iter(chunks))
    }

    #[tokio::test]
    async fn streamed_tool_reply_keeps_the_body_after_a_split_http_prelude() {
        let body = lambda_tool_body(
            byte_stream(&[
                b"{\"statusCode\":20",
                b"0,\"headers\":{}}\0\0\0\0",
                b"\0\0\0\0{\"output\":\"{\\\"status\\\":\\\"ok\\\"}\",",
                b"\"cpu_ms\":17}",
            ]),
            MAX_TOOL_REPLY_BYTES,
        )
        .await
        .unwrap();
        let reply: ToolReply = serde_json::from_slice(&body).unwrap();
        assert_eq!(reply.output, "{\"status\":\"ok\"}");
        assert_eq!(reply.cpu_ms, 17);
    }

    #[tokio::test]
    async fn streamed_tool_rejections_preserve_the_http_status() {
        for status in [401, 429, 501, 504] {
            let head = format!("{{\"statusCode\":{status},\"headers\":{{}}}}\0\0\0\0\0\0\0\0");
            let error = lambda_tool_body(
                byte_stream(&[head.as_bytes(), b"executor detail"]),
                MAX_TOOL_REPLY_BYTES,
            )
            .await
            .unwrap_err();
            assert!(
                matches!(error, DispatchError::ToolHttp { status: actual, message }
                if actual == status && message == "executor detail")
            );
        }
    }

    #[tokio::test]
    async fn a_terminal_lambda_failure_is_not_mistaken_for_a_successful_tool_reply() {
        let stream = Box::pin(futures::stream::iter(vec![
            Ok(bytes::Bytes::from_static(
                b"{\"statusCode\":200}\0\0\0\0\0\0\0\0{\"output\":\"ok\",\"cpu_ms\":1}",
            )),
            Err(DispatchError::ExecutorFailed("Runtime.OutOfMemory".into())),
        ]));
        assert!(matches!(
            lambda_tool_body(stream, MAX_TOOL_REPLY_BYTES).await,
            Err(DispatchError::ExecutorFailed(_))
        ));
    }

    #[tokio::test]
    async fn tool_reply_collection_stops_at_its_size_limit() {
        assert!(matches!(
            lambda_tool_body(byte_stream(&[b"123", b"456"]), 5).await,
            Err(DispatchError::Serialization(_))
        ));
        assert_eq!(
            lambda_tool_body(byte_stream(&[b"123", b"45"]), 5)
                .await
                .unwrap(),
            bytes::Bytes::from_static(b"12345")
        );
    }

    #[tokio::test]
    async fn raw_tool_json_is_kept_when_no_http_prelude_is_present() {
        let reply = b"{\"output\":\"ok\",\"cpu_ms\":1}";
        assert_eq!(
            lambda_tool_body(byte_stream(&[reply]), MAX_TOOL_REPLY_BYTES)
                .await
                .unwrap(),
            bytes::Bytes::copy_from_slice(reply)
        );
    }

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

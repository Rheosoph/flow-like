//! The query service as an AWS Lambda running `lambda_http`: one `InvokeWithResponseStream` per
//! request, the body wrapped as an API Gateway v2 event, the streamed answer read past its
//! metadata prelude. Only the API's execution role may invoke the function; the token is still
//! checked by the service.

use aws_sdk_lambda::error::DisplayErrorContext;
use aws_sdk_lambda::types::InvokeWithResponseStreamResponseEvent as StreamEvent;
use bytes::Bytes;
use flow_like_query::client::{ByteStream, QueryError, QueryTransport};
use flow_like_query::{CONTENT_TYPE, ErrorCode, QUERY_PATH, WireError};
use futures::StreamExt;
use hyper::http::{HeaderMap, HeaderValue, header};

use crate::execution::{
    LAMBDA_PRELUDE_SEPARATOR, LambdaPreludeScan, apigw_v2_event, scan_lambda_http_prelude,
};

/// How much of a body that is not a query response is quoted in the error.
const UNFRAMED_BODY_CAP: usize = 1024;

pub(super) struct LambdaTransport {
    client: aws_sdk_lambda::Client,
    function: String,
}

impl LambdaTransport {
    pub(super) async fn new(function: String) -> Self {
        let config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
        Self {
            client: aws_sdk_lambda::Client::new(&config),
            function,
        }
    }
}

fn request_headers(token: &str) -> Result<HeaderMap, QueryError> {
    let mut authorization = HeaderValue::from_str(&format!("Bearer {token}")).map_err(|_| {
        QueryError::Signing("The request token is not a valid header value".to_string())
    })?;
    authorization.set_sensitive(true);
    let mut headers = HeaderMap::new();
    headers.insert(header::AUTHORIZATION, authorization);
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    headers.insert(header::ACCEPT, HeaderValue::from_static(CONTENT_TYPE));
    Ok(headers)
}

#[async_trait::async_trait]
impl QueryTransport for LambdaTransport {
    async fn send(&self, body: Bytes, token: String) -> Result<ByteStream, QueryError> {
        let event = apigw_v2_event(QUERY_PATH, &body, request_headers(&token)?);
        let payload = serde_json::to_vec(&event).map_err(|error| {
            QueryError::Malformed(format!("Encoding the Lambda event failed: {error}"))
        })?;
        let response = self
            .client
            .invoke_with_response_stream()
            .function_name(&self.function)
            .payload(aws_sdk_lambda::primitives::Blob::new(payload))
            .send()
            .await
            .map_err(|error| {
                QueryError::Transport(format!(
                    "Invoking the query Lambda '{}' failed: {}",
                    self.function,
                    DisplayErrorContext(error)
                ))
            })?;
        let stream = futures::stream::unfold(Some(response.event_stream), |receiver| async move {
            let mut receiver = receiver?;
            let (chunk, more) = next_chunk(receiver.recv().await)?;
            Some((chunk, more.then_some(receiver)))
        });
        framed_body(Box::pin(stream)).await
    }

    fn name(&self) -> &'static str {
        "lambda"
    }
}

/// One stream event as the next body chunk; `None` ends the body. The flag says whether the
/// receiver may be polled again. A Lambda that dies mid-answer reports it in `InvokeComplete`.
fn next_chunk<E: std::error::Error>(
    event: Result<Option<StreamEvent>, E>,
) -> Option<(Result<Bytes, QueryError>, bool)> {
    match event {
        Ok(Some(StreamEvent::PayloadChunk(chunk))) => Some((
            Ok(chunk
                .payload
                .map(|payload| Bytes::from(payload.into_inner()))
                .unwrap_or_default()),
            true,
        )),
        Ok(Some(StreamEvent::InvokeComplete(complete))) => {
            let code = complete.error_code()?;
            Some((Err(QueryError::Remote(stopped(code))), false))
        }
        Ok(Some(_)) => Some((Ok(Bytes::new()), true)),
        Ok(None) => None,
        Err(error) => Some((
            Err(QueryError::Transport(format!(
                "Reading the query Lambda's answer failed: {}",
                DisplayErrorContext(error)
            ))),
            false,
        )),
    }
}

fn stopped(code: &str) -> WireError {
    match code {
        "Runtime.OutOfMemory" => WireError::new(
            ErrorCode::ResourcesExhausted,
            "The query needed more memory than the query service has",
        ),
        "Sandbox.Timedout" => WireError::new(
            ErrorCode::Timeout,
            "The query service ran out of time before it finished",
        ),
        other => WireError::new(
            ErrorCode::Unavailable,
            format!("The query service stopped before it finished ({other})"),
        ),
    }
}

/// Whether a `lambda_http` prelude announces a query response, whatever its status: the
/// service explains its errors in frames, as it does over HTTP.
fn prelude_is_framed(prelude: &[u8]) -> bool {
    let Ok(prelude) = serde_json::from_slice::<serde_json::Value>(prelude) else {
        return false;
    };
    let Some(headers) = prelude
        .get("headers")
        .and_then(|headers| headers.as_object())
    else {
        return false;
    };
    headers.iter().any(|(name, value)| {
        let value = value
            .as_str()
            .or_else(|| value.as_array()?.first()?.as_str());
        name.eq_ignore_ascii_case("content-type")
            && value.is_some_and(|value| value.starts_with(CONTENT_TYPE))
    })
}

fn chained(head: Vec<u8>, rest: Option<ByteStream>) -> ByteStream {
    let head = futures::stream::iter((!head.is_empty()).then(|| Ok(Bytes::from(head))));
    match rest {
        Some(rest) => Box::pin(head.chain(rest)),
        None => Box::pin(head),
    }
}

/// The response body after its metadata prelude. A body that is not a query response (a Lambda
/// runtime error page, an unknown route) fails here with its status, as `HttpTransport` does.
async fn framed_body(mut stream: ByteStream) -> Result<ByteStream, QueryError> {
    let mut buffered: Vec<u8> = Vec::new();
    loop {
        match scan_lambda_http_prelude(&buffered) {
            LambdaPreludeScan::Complete {
                status,
                body_offset,
            } => {
                let prelude = &buffered[..body_offset - LAMBDA_PRELUDE_SEPARATOR.len()];
                if prelude_is_framed(prelude) {
                    return Ok(chained(buffered[body_offset..].to_vec(), Some(stream)));
                }
                let mut body = buffered[body_offset..].to_vec();
                while body.len() < UNFRAMED_BODY_CAP {
                    match stream.next().await {
                        Some(Ok(bytes)) => body.extend_from_slice(&bytes),
                        _ => break,
                    }
                }
                body.truncate(UNFRAMED_BODY_CAP);
                return Err(QueryError::Transport(format!(
                    "The query Lambda answered HTTP {status} without a query response: {}",
                    String::from_utf8_lossy(&body).trim()
                )));
            }
            LambdaPreludeScan::Absent => return Ok(chained(buffered, Some(stream))),
            LambdaPreludeScan::Incomplete => match stream.next().await {
                Some(Ok(bytes)) => buffered.extend_from_slice(&bytes),
                Some(Err(error)) => return Err(error),
                None => return Ok(chained(buffered, None)),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_query::Trailer;
    use flow_like_query::frames::{FrameKind, ResponseAssembler, encode_json_frame};

    fn prelude(status: u16, content_type: &str) -> Vec<u8> {
        let mut bytes = serde_json::to_vec(&serde_json::json!({
            "statusCode": status,
            "headers": { "content-type": content_type },
            "cookies": [],
        }))
        .unwrap();
        bytes.extend_from_slice(&LAMBDA_PRELUDE_SEPARATOR);
        bytes
    }

    fn chunked(bytes: Vec<u8>, size: usize) -> ByteStream {
        let chunks: Vec<Result<Bytes, QueryError>> = bytes
            .chunks(size)
            .map(|chunk| Ok(Bytes::copy_from_slice(chunk)))
            .collect();
        Box::pin(futures::stream::iter(chunks))
    }

    async fn collect(stream: ByteStream) -> Vec<u8> {
        stream.map(|chunk| chunk.unwrap().to_vec()).concat().await
    }

    fn trailer_frame() -> Bytes {
        encode_json_frame(
            FrameKind::Trailer,
            &Trailer {
                elapsed_ms: 1,
                ..Trailer::default()
            },
        )
    }

    #[tokio::test]
    async fn framed_bodies_pass_through_whatever_their_status_and_chunking() {
        let refusal = encode_json_frame(
            FrameKind::Error,
            &WireError::new(ErrorCode::Unauthorized, "token refused"),
        );
        for (status, frames) in [(200, trailer_frame()), (401, refusal)] {
            let mut bytes = prelude(status, CONTENT_TYPE);
            bytes.extend_from_slice(&frames);
            for size in [1, 3, 17, bytes.len()] {
                let body = framed_body(chunked(bytes.clone(), size)).await.unwrap();
                assert_eq!(collect(body).await, frames.to_vec(), "{status} in {size}");
            }
        }
    }

    #[tokio::test]
    async fn unframed_answers_fail_with_their_status() {
        let mut bytes = prelude(502, "text/plain");
        bytes.extend_from_slice(b"Internal Server Error");
        let error = match framed_body(chunked(bytes, 5)).await {
            Ok(_) => panic!("an unframed body must not pass"),
            Err(error) => error.to_string(),
        };
        assert!(error.contains("HTTP 502"), "{error}");
        assert!(error.contains("Internal Server Error"), "{error}");
    }

    #[tokio::test]
    async fn bodies_without_a_prelude_reach_the_frame_decoder() {
        let frames = trailer_frame();
        let body = framed_body(chunked(frames.to_vec(), 2)).await.unwrap();
        let mut assembler = ResponseAssembler::default();
        assembler.push(&collect(body).await).unwrap();
        assert_eq!(assembler.finish().unwrap().trailer.elapsed_ms, 1);
    }

    #[test]
    fn a_lambda_that_dies_mid_answer_names_the_cause() {
        assert_eq!(
            stopped("Runtime.OutOfMemory").code,
            ErrorCode::ResourcesExhausted
        );
        assert_eq!(stopped("Sandbox.Timedout").code, ErrorCode::Timeout);
        assert_eq!(stopped("Runtime.ExitError").code, ErrorCode::Unavailable);
        let complete = aws_sdk_lambda::types::InvokeWithResponseStreamCompleteEvent::builder()
            .error_code("Runtime.OutOfMemory")
            .build();
        let (chunk, more) =
            next_chunk::<std::io::Error>(Ok(Some(StreamEvent::InvokeComplete(complete)))).unwrap();
        assert!(!more);
        assert!(matches!(
            chunk,
            Err(QueryError::Remote(WireError {
                code: ErrorCode::ResourcesExhausted,
                ..
            }))
        ));
        let clean = aws_sdk_lambda::types::InvokeWithResponseStreamCompleteEvent::builder().build();
        assert!(
            next_chunk::<std::io::Error>(Ok(Some(StreamEvent::InvokeComplete(clean)))).is_none()
        );
    }
}

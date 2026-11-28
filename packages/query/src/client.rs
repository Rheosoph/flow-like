//! Calling the query service: sign the exact request body, send it over a transport (HTTP here,
//! Lambda in the API, in-process for tests), and assemble the framed response.

use crate::frames::{FrameError, QueryResponse, ResponseAssembler};
use crate::protocol::{QueryRequest, WireError, body_digest};
use async_trait::async_trait;
use bytes::Bytes;
use futures::{Stream, StreamExt};
use std::pin::Pin;
use std::sync::Arc;

pub type ByteStream = Pin<Box<dyn Stream<Item = Result<Bytes, QueryError>> + Send>>;

#[derive(Debug, thiserror::Error)]
pub enum QueryError {
    #[error("The query service could not be reached: {0}")]
    Transport(String),
    #[error("Signing the query request failed: {0}")]
    Signing(String),
    #[error("{0}")]
    Remote(WireError),
    #[error("{0}")]
    Malformed(String),
}

impl From<FrameError> for QueryError {
    fn from(error: FrameError) -> Self {
        match error {
            FrameError::Remote(error) => Self::Remote(error),
            other => Self::Malformed(other.to_string()),
        }
    }
}

/// Signs a token for one request body; the API implements it with its backend key.
pub trait RequestSigner: Send + Sync {
    fn sign(&self, request_id: &str, body_sha256: &str) -> Result<String, String>;
}

/// Carries a signed body to the service and returns the response body as it arrives. A
/// transport returns the body even for an error status: the service explains errors in frames.
#[async_trait]
pub trait QueryTransport: Send + Sync {
    async fn send(&self, body: Bytes, token: String) -> Result<ByteStream, QueryError>;
    fn name(&self) -> &'static str;
}

#[derive(Clone)]
pub struct QueryClient {
    transport: Arc<dyn QueryTransport>,
    signer: Arc<dyn RequestSigner>,
}

impl QueryClient {
    pub fn new(transport: Arc<dyn QueryTransport>, signer: Arc<dyn RequestSigner>) -> Self {
        Self { transport, signer }
    }

    pub fn transport_name(&self) -> &'static str {
        self.transport.name()
    }

    pub async fn run(&self, request: &QueryRequest) -> Result<QueryResponse, QueryError> {
        let body = serde_json::to_vec(request)
            .map_err(|error| QueryError::Malformed(format!("encoding the request: {error}")))?;
        let token = self
            .signer
            .sign(&request.request_id, &body_digest(&body))
            .map_err(QueryError::Signing)?;
        let mut stream = self.transport.send(Bytes::from(body), token).await?;
        let mut assembler = ResponseAssembler::default();
        while let Some(chunk) = stream.next().await {
            assembler.push(&chunk?)?;
        }
        Ok(assembler.finish()?)
    }
}

#[cfg(feature = "client")]
pub use http::{HttpTransport, PlatformAuthorizer};

#[cfg(feature = "client")]
mod http {
    use super::*;
    use crate::protocol::{CONTENT_TYPE, QUERY_PATH, TOKEN_HEADER};

    /// Supplies the `Authorization` value a platform demands before the service sees a request,
    /// such as a Google ID token for a Cloud Run service with IAM invokers.
    #[async_trait]
    pub trait PlatformAuthorizer: Send + Sync {
        async fn authorization(&self) -> Result<String, QueryError>;
    }

    /// The service as a container behind a URL (docker-compose, Kubernetes, Azure, GCP).
    pub struct HttpTransport {
        client: reqwest::Client,
        url: String,
        authorizer: Option<Arc<dyn PlatformAuthorizer>>,
    }

    impl HttpTransport {
        pub fn new(base_url: &str) -> Result<Self, QueryError> {
            let client = reqwest::Client::builder()
                .build()
                .map_err(|error| QueryError::Transport(error.to_string()))?;
            Ok(Self {
                client,
                url: format!("{}{QUERY_PATH}", base_url.trim_end_matches('/')),
                authorizer: None,
            })
        }

        pub fn with_platform_authorizer(mut self, authorizer: Arc<dyn PlatformAuthorizer>) -> Self {
            self.authorizer = Some(authorizer);
            self
        }
    }

    #[async_trait]
    impl QueryTransport for HttpTransport {
        async fn send(&self, body: Bytes, token: String) -> Result<ByteStream, QueryError> {
            let mut request = self
                .client
                .post(&self.url)
                .header(TOKEN_HEADER, token)
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .header(reqwest::header::ACCEPT, CONTENT_TYPE);
            if let Some(authorizer) = &self.authorizer {
                request = request.header(
                    reqwest::header::AUTHORIZATION,
                    authorizer.authorization().await?,
                );
            }
            let response = request
                .body(body)
                .send()
                .await
                .map_err(|error| QueryError::Transport(error.to_string()))?;
            let framed = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value.starts_with(CONTENT_TYPE));
            if !framed {
                return Err(QueryError::Transport(format!(
                    "{} answered HTTP {} without a query response",
                    self.url,
                    response.status()
                )));
            }
            Ok(Box::pin(response.bytes_stream().map(|chunk| {
                chunk.map_err(|error| QueryError::Transport(error.to_string()))
            })))
        }

        fn name(&self) -> &'static str {
            "http"
        }
    }
}

//! The API's side of the query service. Routes authorize and build one request; this module
//! picks the transport from the environment, signs the exact request body with the backend key,
//! mints read-only storage credentials per source, and turns service failures into HTTP errors.
//! Request bodies carry credentials and are never logged.

#[cfg(feature = "lambda")]
mod lambda;
mod signer;
mod sources;
#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::time::Instant;

use axum::http::StatusCode;
use flow_like_query::client::HttpTransport;
use flow_like_query::{
    ErrorCode, Operation, PROTOCOL_VERSION, QueryClient, QueryError, QueryRequest, QueryResponse,
    QueryTransport, RequestLimits, SourceSpec, WireError,
};
use flow_like_storage::databases::workbench::{ExecutionBudget, SqlQueryResult, WorkbenchLimits};
use flow_like_types::Value;
use flow_like_types::tokio::sync::OnceCell;
use serde::Serialize;

use crate::error::ApiError;
use crate::middleware::jwt::AppUser;

pub(crate) use sources::LanceRead;

/// Name of the query Lambda the API invokes (AWS deployments, feature `lambda`).
pub const LAMBDA_FUNCTION_ENV: &str = "QUERY_LAMBDA_FUNCTION";
/// Base URL of the query service container (every other deployment).
pub const SERVICE_URL_ENV: &str = "QUERY_SERVICE_URL";

const NOT_CONFIGURED: &str =
    "The query service is not configured (set QUERY_LAMBDA_FUNCTION or QUERY_SERVICE_URL)";
const LAMBDA_UNSUPPORTED: &str = "QUERY_LAMBDA_FUNCTION is set, but this API was built without Lambda support; set QUERY_SERVICE_URL instead";

/// The service stops a query after this long, so the answer arrives before CloudFront's 60 s cut.
const QUERY_TIMEOUT_MS: u64 = 50_000;
/// How long the API waits for a whole answer.
const QUERY_DEADLINE_MS: i64 = 55_000;

#[derive(Debug, Clone, PartialEq, Eq)]
enum TransportChoice {
    Lambda(String),
    Http(String),
    InProcess,
    Unavailable(&'static str),
}

fn configured(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// A Lambda function wins over a URL; without either, only a build with the in-process service
/// can answer.
fn choose_transport(
    lambda_function: Option<String>,
    service_url: Option<String>,
) -> TransportChoice {
    if let Some(function) = configured(lambda_function) {
        return if cfg!(feature = "lambda") {
            TransportChoice::Lambda(function)
        } else {
            TransportChoice::Unavailable(LAMBDA_UNSUPPORTED)
        };
    }
    if let Some(url) = configured(service_url) {
        return TransportChoice::Http(url);
    }
    if cfg!(feature = "query-in-process") {
        TransportChoice::InProcess
    } else {
        TransportChoice::Unavailable(NOT_CONFIGURED)
    }
}

async fn build_transport(choice: TransportChoice) -> Result<Arc<dyn QueryTransport>, String> {
    match choice {
        #[cfg(feature = "lambda")]
        TransportChoice::Lambda(function) => {
            Ok(Arc::new(lambda::LambdaTransport::new(function).await))
        }
        #[cfg(not(feature = "lambda"))]
        TransportChoice::Lambda(_) => Err(LAMBDA_UNSUPPORTED.to_string()),
        TransportChoice::Http(url) => HttpTransport::new(&url)
            .map(|transport| Arc::new(transport) as Arc<dyn QueryTransport>)
            .map_err(|error| format!("{SERVICE_URL_ENV} cannot be used: {error}")),
        #[cfg(feature = "query-in-process")]
        TransportChoice::InProcess => {
            Ok(Arc::new(flow_like_query::server::InProcessTransport::new(
                Arc::new(flow_like_query::server::ServiceConfig::from_env()),
            )))
        }
        #[cfg(not(feature = "query-in-process"))]
        TransportChoice::InProcess => Err(NOT_CONFIGURED.to_string()),
        TransportChoice::Unavailable(message) => Err(message.to_string()),
    }
}

/// The process's way to the query service. The transport is chosen on first use and kept: the
/// environment that selects it cannot change while the process runs.
#[derive(Default)]
pub struct QueryService {
    transport: OnceCell<Result<Arc<dyn QueryTransport>, String>>,
}

impl QueryService {
    #[cfg(all(test, feature = "query-in-process"))]
    pub(crate) fn with_transport(transport: Arc<dyn QueryTransport>) -> Self {
        Self {
            transport: OnceCell::new_with(Some(Ok(transport))),
        }
    }

    async fn transport(&self) -> Result<Arc<dyn QueryTransport>, ApiError> {
        let transport = self
            .transport
            .get_or_init(|| async {
                let choice = choose_transport(
                    std::env::var(LAMBDA_FUNCTION_ENV).ok(),
                    std::env::var(SERVICE_URL_ENV).ok(),
                );
                let built = build_transport(choice).await;
                match &built {
                    Ok(transport) => {
                        tracing::info!(
                            transport = transport.name(),
                            "Query service transport ready"
                        );
                    }
                    Err(error) => tracing::error!(%error, "Query service transport unavailable"),
                }
                built
            })
            .await;
        transport.clone().map_err(|message| {
            ApiError::coded(
                StatusCode::SERVICE_UNAVAILABLE,
                "QUERY_SERVICE_UNAVAILABLE",
                message,
            )
        })
    }

    /// Whether the service runs inside this process, so storage credentials never leave it.
    pub(crate) async fn in_process(&self) -> bool {
        self.transport()
            .await
            .is_ok_and(|transport| transport.name() == "in-process")
    }

    /// Sends `request` signed for `subject` and returns the whole answer.
    pub(crate) async fn run(
        &self,
        subject: &str,
        request: &QueryRequest,
    ) -> Result<QueryResponse, ApiError> {
        let transport = self.transport().await?;
        let transport_name = transport.name();
        let client = QueryClient::new(transport, Arc::new(signer::BackendSigner::new(subject)));
        let started = Instant::now();
        let outcome = client.run(request).await;
        tracing::debug!(
            request_id = %request.request_id,
            transport = transport_name,
            elapsed_ms = started.elapsed().as_millis() as u64,
            ok = outcome.is_ok(),
            "Query service call"
        );
        outcome.map_err(service_error)
    }
}

/// One request under the API's deadline: the service may run for `QUERY_TIMEOUT_MS`, the API
/// waits a little longer for the answer to arrive.
pub(crate) fn request(sources: Vec<SourceSpec>, rows: usize, operation: Operation) -> QueryRequest {
    QueryRequest {
        version: PROTOCOL_VERSION,
        request_id: flow_like_types::create_id(),
        deadline_ms: chrono::Utc::now().timestamp_millis() + QUERY_DEADLINE_MS,
        sources,
        limits: RequestLimits {
            rows,
            memory_bytes: None,
            timeout_ms: QUERY_TIMEOUT_MS,
        },
        operation,
    }
}

/// Rows a workbench statement may return: its own limit, else the workbench default, never
/// above the workbench ceiling.
pub(crate) fn workbench_rows(limit: Option<usize>) -> usize {
    ExecutionBudget::from_limits(WorkbenchLimits {
        rows: limit,
        memory_bytes: None,
    })
    .rows
}

/// The `sub` claim of a request token: the person behind the call, or `api` for keys and apps.
pub(crate) fn subject(user: &AppUser) -> String {
    user.sub().unwrap_or_else(|_| "api".to_string())
}

/// Service failures as HTTP errors. Query mistakes keep their message; a failure of the service
/// itself is reported under an error id and answered generically, so nothing internal leaks.
pub(crate) fn service_error(error: QueryError) -> ApiError {
    match error {
        QueryError::Remote(WireError { code, message }) => match code {
            ErrorCode::InvalidRequest | ErrorCode::QueryFailed => ApiError::bad_request(message),
            ErrorCode::ResourcesExhausted => ApiError::coded(
                StatusCode::PAYLOAD_TOO_LARGE,
                "QUERY_RESOURCES_EXHAUSTED",
                message,
            ),
            ErrorCode::Timeout => ApiError::gateway_timeout(message),
            ErrorCode::Unavailable => ApiError::service_unavailable(message),
            ErrorCode::Unauthorized => ApiError::service_unavailable(format!(
                "The query service refused the API's token; it must verify with this API's BACKEND_PUB: {message}"
            )),
            ErrorCode::Internal => {
                ApiError::internal(format!("The query service failed: {message}"))
            }
        },
        QueryError::Transport(message) => ApiError::service_unavailable(message),
        QueryError::Signing(message) => {
            ApiError::internal(format!("Signing a query service request failed: {message}"))
        }
        QueryError::Malformed(message) => {
            tracing::warn!(%message, "The query service sent a malformed response");
            ApiError::bad_gateway("The query service sent an incomplete or invalid response")
        }
    }
}

/// Longest table, column or search text a parameter options request may name.
const MAX_OPTION_NAME_CHARS: usize = 256;

/// Bounds the names a `parameter_options` request carries; the service quotes them itself.
pub(crate) fn validate_option_target(
    table: &str,
    column: &str,
    search: Option<&str>,
) -> Result<(), ApiError> {
    for (label, value) in [("table", table), ("column", column)] {
        if value.trim().is_empty() || value.chars().count() > MAX_OPTION_NAME_CHARS {
            return Err(ApiError::bad_request(format!(
                "The {label} name must be 1 to {MAX_OPTION_NAME_CHARS} characters"
            )));
        }
    }
    if search.is_some_and(|search| search.chars().count() > MAX_OPTION_NAME_CHARS) {
        return Err(ApiError::bad_request(format!(
            "The search text is longer than {MAX_OPTION_NAME_CHARS} characters"
        )));
    }
    Ok(())
}

/// The JSON answer of a non-tabular operation, such as `describe` or `parameter_options`.
pub(crate) fn json_answer(response: QueryResponse) -> Result<Value, ApiError> {
    response
        .json
        .ok_or_else(|| ApiError::bad_gateway("The query service answered without a JSON result"))
}

/// A statement's rows in the workbench JSON shape, plus any limit the service lowered.
#[derive(Debug, Serialize)]
pub struct QueryRows {
    #[serde(flatten)]
    pub result: SqlQueryResult,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

impl QueryRows {
    pub(crate) fn from_response(response: &QueryResponse) -> Result<Self, ApiError> {
        Ok(Self {
            result: sql_result(response)?,
            warnings: response.trailer.warnings.clone(),
        })
    }
}

pub(crate) fn sql_result(response: &QueryResponse) -> Result<SqlQueryResult, ApiError> {
    response
        .to_sql_result()
        .map_err(|error| ApiError::internal(format!("Reading the query result failed: {error:#}")))
}

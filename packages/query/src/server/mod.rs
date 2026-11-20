//! The query service: verify the API's token, open the request's sources with the credentials
//! it carries, run the operation within the service's hard limits, and stream frames back.

pub mod auth;
pub mod external;
mod forecast;
mod graph;
mod ops;
mod table;

use crate::frames::{FrameKind, encode_json_frame};
use crate::protocol::{
    CONTENT_TYPE, ErrorCode, HEALTH_PATH, MAX_REQUEST_BYTES, PROTOCOL_VERSION, QUERY_PATH,
    QueryRequest, TOKEN_HEADER, WireError, body_digest,
};
use auth::TokenVerifier;
use axum::body::{Body, Bytes};
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::Response;
use axum::routing::{get, post};
use external::ExternalConnector;
use flow_like_storage::databases::workbench::SpillConfig;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

/// Ceilings no request can raise.
#[derive(Clone, Debug)]
pub struct ServiceLimits {
    pub max_rows: usize,
    pub max_memory_bytes: usize,
    pub max_timeout: Duration,
    pub spill: Option<SpillConfig>,
}

fn env_number<T: std::str::FromStr>(name: &str) -> Option<T> {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse().ok())
}

impl ServiceLimits {
    /// `QUERY_MAX_ROWS` (10,000), `QUERY_MAX_TIMEOUT_MS` (50,000), `QUERY_MEMORY_BYTES` (60 % of
    /// the Lambda's memory, else 2 GiB), `QUERY_SPILL_DIR` and `QUERY_SPILL_MAX_BYTES` (400 MiB on
    /// Lambda's default /tmp, else 4 GiB; 0 turns spilling off).
    pub fn from_env() -> Self {
        let lambda_memory_mb: Option<usize> = env_number("AWS_LAMBDA_FUNCTION_MEMORY_SIZE");
        let max_memory_bytes = env_number("QUERY_MEMORY_BYTES").unwrap_or_else(|| {
            lambda_memory_mb
                .map(|mb| mb * 1024 * 1024 / 10 * 6)
                .unwrap_or(2 * 1024 * 1024 * 1024)
        });
        let spill_bytes: u64 =
            env_number("QUERY_SPILL_MAX_BYTES").unwrap_or(if lambda_memory_mb.is_some() {
                400 * 1024 * 1024
            } else {
                4 * 1024 * 1024 * 1024
            });
        let spill = (spill_bytes > 0).then(|| SpillConfig {
            directory: std::env::var("QUERY_SPILL_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|_| std::env::temp_dir().join("flow-like-query-spill")),
            max_bytes: spill_bytes,
        });
        Self {
            max_rows: env_number("QUERY_MAX_ROWS").unwrap_or(10_000),
            max_memory_bytes,
            max_timeout: Duration::from_millis(
                env_number("QUERY_MAX_TIMEOUT_MS").unwrap_or(50_000),
            ),
            spill,
        }
    }
}

pub struct ServiceConfig {
    pub limits: ServiceLimits,
    pub connector: Arc<dyn ExternalConnector>,
}

impl ServiceConfig {
    /// Limits from the environment and the built-in drivers; `QUERY_ALLOW_PRIVATE_NETWORKS=true`
    /// lets external sources reach private addresses (self-hosted only).
    pub fn from_env() -> Self {
        let allow_private = std::env::var("QUERY_ALLOW_PRIVATE_NETWORKS")
            .is_ok_and(|value| matches!(value.trim(), "1" | "true" | "yes"));
        Self {
            limits: ServiceLimits::from_env(),
            connector: Arc::new(external::DriverConnector::new(allow_private)),
        }
    }
}

struct RouterState {
    config: Arc<ServiceConfig>,
    verifier: TokenVerifier,
}

/// `POST /v1/query` and `GET /health`. Every query must carry a token `verifier` accepts.
pub fn query_router(config: Arc<ServiceConfig>, verifier: TokenVerifier) -> axum::Router {
    let state = Arc::new(RouterState { config, verifier });
    axum::Router::new()
        .route(QUERY_PATH, post(query))
        .route(HEALTH_PATH, get(|| async { "ok" }))
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BYTES))
        .with_state(state)
}

fn framed(status: StatusCode, body: Body) -> Response {
    let mut response = Response::new(body);
    *response.status_mut() = status;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static(CONTENT_TYPE),
    );
    response
}

fn refuse(error: WireError) -> Response {
    let status =
        StatusCode::from_u16(error.code.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    framed(
        status,
        Body::from(encode_json_frame(FrameKind::Error, &error)),
    )
}

async fn query(State(state): State<Arc<RouterState>>, headers: HeaderMap, body: Bytes) -> Response {
    let dedicated = headers
        .get(TOKEN_HEADER)
        .and_then(|value| value.to_str().ok());
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    let Some(token) = dedicated.or(bearer) else {
        return refuse(WireError::new(
            ErrorCode::Unauthorized,
            "The request carries no bearer token",
        ));
    };
    if let Err(error) = state.verifier.verify(token, &body_digest(&body)) {
        tracing::warn!(error = %error, "Refused a query service request");
        return refuse(error);
    }
    let request: QueryRequest = match serde_json::from_slice(&body) {
        Ok(request) => request,
        Err(error) => {
            return refuse(WireError::new(
                ErrorCode::InvalidRequest,
                format!("The request is not a valid query request: {error}"),
            ));
        }
    };
    framed(StatusCode::OK, respond(state.config.clone(), request))
}

/// The framed response body for an already authorized request. In-process callers use this
/// directly; there is no token to check inside one process.
pub fn respond(config: Arc<ServiceConfig>, request: QueryRequest) -> Body {
    let (sender, receiver) = mpsc::channel::<Bytes>(8);
    tokio::spawn(async move {
        let sink = ops::FrameSink::new(sender);
        let outcome = if request.version != PROTOCOL_VERSION {
            Err(WireError::new(
                ErrorCode::InvalidRequest,
                format!(
                    "Protocol version {} is not supported; this service speaks {PROTOCOL_VERSION}",
                    request.version
                ),
            ))
        } else {
            ops::execute(&config, request, &sink).await
        };
        if let Err(error) = outcome
            && !sink.closed()
        {
            if error.code == ErrorCode::Internal {
                tracing::error!(error = %error.message, "Query service request failed");
            }
            let _ = sink.send(encode_json_frame(FrameKind::Error, &error)).await;
        }
    });
    let stream = futures::stream::unfold(receiver, |mut receiver| async move {
        receiver
            .recv()
            .await
            .map(|chunk| (Ok::<_, std::io::Error>(chunk), receiver))
    });
    Body::from_stream(stream)
}

#[cfg(feature = "client")]
pub use in_process::InProcessTransport;

#[cfg(feature = "client")]
mod in_process {
    use super::*;
    use crate::client::{ByteStream, QueryError, QueryTransport};
    use async_trait::async_trait;
    use futures::StreamExt;

    /// The service inside the calling process, for tests and single-process development.
    pub struct InProcessTransport {
        config: Arc<ServiceConfig>,
    }

    impl InProcessTransport {
        pub fn new(config: Arc<ServiceConfig>) -> Self {
            Self { config }
        }
    }

    #[async_trait]
    impl QueryTransport for InProcessTransport {
        async fn send(&self, body: Bytes, _token: String) -> Result<ByteStream, QueryError> {
            let request: QueryRequest = serde_json::from_slice(&body)
                .map_err(|error| QueryError::Malformed(error.to_string()))?;
            let stream = respond(self.config.clone(), request).into_data_stream();
            Ok(Box::pin(stream.map(|chunk| {
                chunk.map_err(|error| QueryError::Transport(error.to_string()))
            })))
        }

        fn name(&self) -> &'static str {
            "in-process"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frames::ResponseAssembler;
    use crate::protocol::{
        AUDIENCE, ExternalConnection, ExternalSource, JOSE_TYPE, Operation, QueryClaims,
        QueryShape, ReaderSpec, RequestLimits, SecretMaterial, SourceLocation, SourceSpec,
    };
    use async_trait::async_trait;
    use flow_like_storage::arrow_array::{Int64Array, RecordBatch, StringArray};
    use flow_like_storage::arrow_schema::{DataType, Field, Schema};
    use flow_like_storage::databases::workbench::ExternalTableResolver;
    use flow_like_storage::datafusion::catalog::TableProvider;
    use flow_like_storage::datafusion::datasource::MemTable;
    use serde_json::json;
    use tower::ServiceExt;

    struct Fixture;

    #[async_trait]
    impl ExternalTableResolver for Fixture {
        async fn table(&self, table: &str) -> flow_like_types::Result<Arc<dyn TableProvider>> {
            if table != "orders" {
                return Err(flow_like_types::anyhow!("no table {table}"));
            }
            let schema = Arc::new(Schema::new(vec![
                Field::new("id", DataType::Int64, false),
                Field::new("region", DataType::Utf8, false),
            ]));
            let batch = RecordBatch::try_new(
                schema.clone(),
                vec![
                    Arc::new(Int64Array::from(vec![1, 2, 3])),
                    Arc::new(StringArray::from(vec!["West", "East", "West"])),
                ],
            )?;
            Ok(Arc::new(MemTable::try_new(schema, vec![vec![batch]])?))
        }
    }

    #[async_trait]
    impl ExternalConnector for Fixture {
        async fn resolver(
            &self,
            _source: &ExternalSource,
            _deadline: Option<std::time::Instant>,
        ) -> Result<Arc<dyn ExternalTableResolver>, WireError> {
            Ok(Arc::new(Fixture))
        }
    }

    fn request(sql: &str) -> QueryRequest {
        QueryRequest {
            version: PROTOCOL_VERSION,
            request_id: "request-1".to_string(),
            deadline_ms: chrono_now_ms() + 30_000,
            sources: vec![SourceSpec {
                schema: Some("warehouse".to_string()),
                reader: ReaderSpec::Unrestricted,
                location: SourceLocation::External(ExternalSource {
                    source_id: "source-1".to_string(),
                    app_id: "app-1".to_string(),
                    connection: ExternalConnection::Clickhouse {
                        host: "db.example".to_string(),
                        port: 8123,
                        database: "sales".to_string(),
                    },
                    secret: SecretMaterial::None,
                }),
            }],
            limits: RequestLimits {
                rows: 10,
                memory_bytes: None,
                timeout_ms: 10_000,
            },
            operation: Operation::Sql {
                sql: sql.to_string(),
                params: json!({ "region": "West" }),
                shape: QueryShape::CrossApp,
                reuse_if_unchanged: None,
            },
        }
    }

    fn chrono_now_ms() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64
    }

    fn router(public_pem: &str) -> axum::Router {
        let config = Arc::new(ServiceConfig {
            limits: ServiceLimits {
                max_rows: 100,
                max_memory_bytes: 64 * 1024 * 1024,
                max_timeout: Duration::from_secs(10),
                spill: None,
            },
            connector: Arc::new(Fixture),
        });
        query_router(
            config,
            TokenVerifier::from_public_pem(public_pem.as_bytes()).unwrap(),
        )
    }

    async fn call(
        router: axum::Router,
        body: Vec<u8>,
        token: Option<String>,
    ) -> (StatusCode, Vec<u8>) {
        let mut builder = axum::http::Request::post(QUERY_PATH);
        if let Some(token) = token {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        send(router, builder, body).await
    }

    async fn send(
        router: axum::Router,
        builder: axum::http::request::Builder,
        body: Vec<u8>,
    ) -> (StatusCode, Vec<u8>) {
        let response = router
            .oneshot(builder.body(Body::from(body)).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, bytes.to_vec())
    }

    fn token_for(private_pem: &str, body: &[u8]) -> String {
        let now = chrono_now_ms() / 1000;
        auth::tests::sign(
            private_pem,
            JOSE_TYPE,
            &QueryClaims {
                iss: "flow-like".to_string(),
                sub: "user-1".to_string(),
                aud: AUDIENCE.to_string(),
                iat: now,
                nbf: now,
                exp: now + 60,
                jti: "jti".to_string(),
                request_id: "request-1".to_string(),
                body_sha256: body_digest(body),
            },
        )
    }

    #[tokio::test]
    async fn signed_requests_stream_rows_and_unsigned_ones_are_refused() {
        let keys = auth::tests::test_keys();
        let body = serde_json::to_vec(&request(
            "SELECT region, COUNT(*) AS n FROM warehouse.orders \
             WHERE region = $region GROUP BY region",
        ))
        .unwrap();

        let (status, bytes) = call(
            router(&keys.public_pem),
            body.clone(),
            Some(token_for(&keys.private_pem, &body)),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let mut assembler = ResponseAssembler::default();
        assembler.push(&bytes).unwrap();
        let response = assembler.finish().unwrap();
        assert_eq!(
            response.to_sql_result().unwrap().rows,
            vec![json!({ "region": "West", "n": 2 })]
        );
        assert_eq!(
            response.trailer.unversioned_tables,
            vec!["warehouse.orders"]
        );
        assert!(response.trailer.versions.is_empty());

        let (status, _) = call(router(&keys.public_pem), body.clone(), None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        let (status, _) = send(
            router(&keys.public_pem),
            axum::http::Request::post(QUERY_PATH)
                .header(TOKEN_HEADER, token_for(&keys.private_pem, &body))
                .header(header::AUTHORIZATION, "Bearer platform-id-token"),
            body.clone(),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "the dedicated header wins over a platform token in Authorization"
        );

        let (status, bytes) = call(
            router(&keys.public_pem),
            body.clone(),
            Some(token_for(&keys.private_pem, b"another body")),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        let mut refused = ResponseAssembler::default();
        assert!(refused.push(&bytes).is_err());
    }

    #[tokio::test]
    async fn query_errors_arrive_as_error_frames() {
        let keys = auth::tests::test_keys();
        let body = serde_json::to_vec(&request("SELECT * FROM warehouse.missing")).unwrap();
        let (status, bytes) = call(
            router(&keys.public_pem),
            body.clone(),
            Some(token_for(&keys.private_pem, &body)),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let mut assembler = ResponseAssembler::default();
        let error = assembler.push(&bytes).unwrap_err().to_string();
        assert!(error.contains("no table missing"), "{error}");
    }
}

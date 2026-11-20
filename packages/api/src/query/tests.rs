use super::signer::{BackendSigner, TOKEN_LIFETIME_SECONDS, claims};
use super::*;
use flow_like::credentials::SharedCredentials;
use flow_like::credentials::aws_credentials::AwsSharedCredentials;
use flow_like_query::{
    AUDIENCE, JOSE_TYPE, QueryClaims, ReaderSpec, RequestSigner, SourceLocation, body_digest,
};
use flow_like_storage::contracts::column_metadata::ColumnReader;
use serde_json::json;

#[test]
fn the_environment_picks_the_transport() {
    let lambda = choose_transport(
        Some("flow-like-query".to_string()),
        Some("http://query:8090".to_string()),
    );
    if cfg!(feature = "lambda") {
        assert_eq!(
            lambda,
            TransportChoice::Lambda("flow-like-query".to_string())
        );
    } else {
        assert_eq!(lambda, TransportChoice::Unavailable(LAMBDA_UNSUPPORTED));
    }
    assert_eq!(
        choose_transport(
            Some("  ".to_string()),
            Some(" http://query:8090 ".to_string())
        ),
        TransportChoice::Http("http://query:8090".to_string()),
        "blank values count as unset and URLs are trimmed"
    );
    let neither = choose_transport(None, Some(String::new()));
    if cfg!(feature = "query-in-process") {
        assert_eq!(neither, TransportChoice::InProcess);
    } else {
        assert_eq!(neither, TransportChoice::Unavailable(NOT_CONFIGURED));
    }
}

#[tokio::test]
async fn an_unconfigured_service_answers_503_naming_the_settings() {
    assert_eq!(
        build_transport(TransportChoice::Unavailable(NOT_CONFIGURED))
            .await
            .err(),
        Some(NOT_CONFIGURED.to_string())
    );
    let service = QueryService {
        transport: OnceCell::new_with(Some(Err(NOT_CONFIGURED.to_string()))),
    };
    let error = service
        .run("user-1", &request(Vec::new(), 1, describe("SELECT 1")))
        .await
        .unwrap_err();
    assert_eq!(error.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        error.public_message(),
        Some(
            "The query service is not configured (set QUERY_LAMBDA_FUNCTION or QUERY_SERVICE_URL)"
        )
    );
    assert!(!service.in_process().await);
}

#[tokio::test]
async fn http_transports_are_built_from_the_url() {
    let transport = build_transport(TransportChoice::Http("http://query:8090/".to_string()))
        .await
        .unwrap();
    assert_eq!(transport.name(), "http");
}

fn describe(sql: &str) -> Operation {
    Operation::Describe {
        sql: sql.to_string(),
        shape: flow_like_query::QueryShape::CrossApp,
    }
}

#[test]
fn requests_carry_the_service_deadline_and_limits() {
    let before = chrono::Utc::now().timestamp_millis();
    let built = request(Vec::new(), 250, describe("SELECT 1"));
    assert_eq!(built.version, PROTOCOL_VERSION);
    assert_eq!(built.limits.rows, 250);
    assert_eq!(built.limits.timeout_ms, QUERY_TIMEOUT_MS);
    assert_eq!(built.limits.memory_bytes, None);
    assert!(built.deadline_ms >= before + QUERY_DEADLINE_MS);
    assert!(built.deadline_ms - before > built.limits.timeout_ms as i64);
    assert_ne!(
        built.request_id,
        request(Vec::new(), 1, describe("SELECT 1")).request_id
    );
}

#[test]
fn option_targets_are_bounded() {
    assert!(validate_option_target("crm.customers", "region", Some("we")).is_ok());
    assert!(validate_option_target(" ", "region", None).is_err());
    assert!(validate_option_target("crm.customers", "", None).is_err());
    assert!(validate_option_target("crm.customers", &"c".repeat(257), None).is_err());
    assert!(validate_option_target("crm.customers", "region", Some(&"s".repeat(257))).is_err());
}

#[test]
fn workbench_rows_keep_the_workbench_default_and_ceiling() {
    assert_eq!(workbench_rows(None), 1_000);
    assert_eq!(workbench_rows(Some(25)), 25);
    assert_eq!(workbench_rows(Some(10_000_000)), 50_000);
}

#[test]
fn claims_bind_the_body_for_one_minute() {
    let claims = claims("user-1", "request-1", "digest", 1_000);
    assert_eq!(claims.iss, "flow-like");
    assert_eq!(claims.aud, AUDIENCE);
    assert_eq!(claims.sub, "user-1");
    assert_eq!((claims.iat, claims.nbf), (1_000, 1_000));
    assert_eq!(claims.exp, 1_000 + TOKEN_LIFETIME_SECONDS);
    assert_eq!(claims.request_id, "request-1");
    assert_eq!(claims.body_sha256, "digest");
    assert!(!claims.jti.is_empty());
    assert_ne!(
        claims.jti,
        super::signer::claims("user-1", "request-1", "digest", 1_000).jti
    );
}

#[test]
fn signed_tokens_are_es256_query_tokens_of_the_backend_key() {
    crate::backend_jwt::init_for_tests();
    let digest = body_digest(br#"{"version":1}"#);
    let token = BackendSigner::new("user-1")
        .sign("request-1", &digest)
        .unwrap();
    let header = jsonwebtoken::decode_header(&token).unwrap();
    assert_eq!(header.alg, jsonwebtoken::Algorithm::ES256);
    assert_eq!(header.typ.as_deref(), Some(JOSE_TYPE));
    let claims: QueryClaims = crate::backend_jwt::verify_any(&token).unwrap();
    assert_eq!(claims.aud, AUDIENCE);
    assert_eq!(claims.sub, "user-1");
    assert_eq!(claims.body_sha256, digest);
    assert_eq!(claims.exp - claims.iat, TOKEN_LIFETIME_SECONDS);
}

#[cfg(feature = "query-in-process")]
#[test]
fn the_service_verifier_accepts_the_api_tokens_for_their_body_only() {
    crate::backend_jwt::init_for_tests();
    let verifier = flow_like_query::server::auth::TokenVerifier::from_public_pem(
        crate::backend_jwt::public_key_pem_for_tests().expect("test keys are installed"),
    )
    .unwrap();
    let body = br#"{"version":1}"#;
    let token = BackendSigner::new("api")
        .sign("request-1", &body_digest(body))
        .unwrap();
    assert!(verifier.verify(&token, &body_digest(body)).is_ok());
    assert!(verifier.verify(&token, &body_digest(b"{}")).is_err());
}

#[test]
fn service_errors_become_http_errors_without_leaking_internals() {
    let remote = |code, message: &str| QueryError::Remote(WireError::new(code, message));
    let status = |error| service_error(error).status();
    assert_eq!(
        status(remote(ErrorCode::InvalidRequest, "x")),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        status(remote(ErrorCode::QueryFailed, "x")),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        status(remote(ErrorCode::ResourcesExhausted, "x")),
        StatusCode::PAYLOAD_TOO_LARGE
    );
    assert_eq!(
        status(remote(ErrorCode::Timeout, "x")),
        StatusCode::GATEWAY_TIMEOUT
    );
    assert_eq!(
        status(remote(ErrorCode::Unavailable, "x")),
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        status(remote(ErrorCode::Unauthorized, "x")),
        StatusCode::SERVICE_UNAVAILABLE,
        "a refused API token is a deployment fault, never the caller's session"
    );
    assert_eq!(
        status(remote(ErrorCode::Internal, "x")),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(
        status(QueryError::Transport("x".to_string())),
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        status(QueryError::Signing("x".to_string())),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(
        status(QueryError::Malformed("x".to_string())),
        StatusCode::BAD_GATEWAY
    );

    let failed = service_error(remote(ErrorCode::QueryFailed, "Unknown schema 'hr'"));
    assert_eq!(failed.public_message(), Some("Unknown schema 'hr'"));
    for hidden in [
        service_error(QueryError::Transport(
            "error sending request for url (http://10.0.0.5:8090/v1/query)".to_string(),
        )),
        service_error(remote(
            ErrorCode::Unavailable,
            "Opening the database of app 'a' failed: s3://content/apps/a",
        )),
    ] {
        let message = hidden.public_message().unwrap_or_default();
        assert!(
            !message.contains("10.0.0.5") && !message.contains("s3://"),
            "{message}"
        );
    }
}

fn sample_credentials() -> SharedCredentials {
    SharedCredentials::Aws(AwsSharedCredentials {
        access_key_id: Some("AKIAEXAMPLE".to_string()),
        secret_access_key: Some("secret".to_string()),
        session_token: Some("token".to_string()),
        meta_bucket: "meta".to_string(),
        content_bucket: "content".to_string(),
        logs_bucket: "logs".to_string(),
        meta_config: None,
        content_config: None,
        logs_config: None,
        region: "eu-central-1".to_string(),
        expiration: None,
        content_path_prefix: None,
        user_content_path_prefix: None,
    })
}

#[test]
fn sources_name_their_app_owner_reader_and_credentials() {
    let project = LanceRead {
        app_id: "app-1".to_string(),
        principal: "user-1".to_string(),
        owner: None,
        reader: ColumnReader::with_attributes(["sales", "support"]),
    };
    let spec = project.source(Some("crm".to_string()), sample_credentials());
    assert_eq!(spec.schema.as_deref(), Some("crm"));
    assert_eq!(
        spec.reader,
        ReaderSpec::Attributes {
            attributes: vec!["sales".to_string(), "support".to_string()]
        }
    );
    match &spec.location {
        SourceLocation::Lance { app_id, owner, .. } => {
            assert_eq!(app_id, "app-1");
            assert_eq!(owner, &None);
        }
        other => panic!("expected a Lance source, got {other:?}"),
    }

    let personal = LanceRead {
        owner: Some("user-1".to_string()),
        reader: ColumnReader::Unrestricted,
        ..project
    };
    let wire = serde_json::to_value(personal.source(None, sample_credentials())).unwrap();
    assert_eq!(wire["schema"], json!(null));
    assert_eq!(wire["reader"], json!({ "kind": "unrestricted" }));
    assert_eq!(wire["location"]["kind"], "lance");
    assert_eq!(wire["location"]["app_id"], "app-1");
    assert_eq!(wire["location"]["owner"], "user-1");
    assert!(wire["location"]["credentials"].is_object());
}

#[cfg(feature = "query-in-process")]
mod in_process {
    use super::*;
    use flow_like_query::server::external::ExternalConnector;
    use flow_like_query::server::{InProcessTransport, ServiceConfig, ServiceLimits};
    use flow_like_query::{ExternalConnection, ExternalSource, QueryShape, SecretMaterial};
    use flow_like_storage::arrow_array::{Int64Array, RecordBatch, StringArray};
    use flow_like_storage::arrow_schema::{DataType, Field, Schema};
    use flow_like_storage::databases::df_provider::lance_table_provider;
    use flow_like_storage::databases::workbench::ExternalTableResolver;
    use flow_like_storage::datafusion::catalog::TableProvider;
    use flow_like_storage::lancedb::{self, Connection};
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::time::Duration;

    struct TempApp {
        path: PathBuf,
    }

    impl Drop for TempApp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    async fn temp_app(table: &str, batch: RecordBatch) -> TempApp {
        let path = std::env::temp_dir().join(format!(
            "flow-like-api-query-{}",
            flow_like_types::create_id()
        ));
        std::fs::create_dir_all(&path).unwrap();
        lancedb::connect(&path.to_string_lossy())
            .execute()
            .await
            .unwrap()
            .create_table(table, vec![batch])
            .execute()
            .await
            .unwrap();
        TempApp { path }
    }

    fn customers() -> RecordBatch {
        RecordBatch::try_new(
            Arc::new(Schema::new(vec![
                Field::new("id", DataType::Int64, false),
                Field::new("name", DataType::Utf8, true),
                Field::new("region", DataType::Utf8, true),
            ])),
            vec![
                Arc::new(Int64Array::from(vec![1, 2, 3])),
                Arc::new(StringArray::from(vec!["Acme", "Globex", "Initech"])),
                Arc::new(StringArray::from(vec!["West", "East", "West"])),
            ],
        )
        .unwrap()
    }

    fn orders() -> RecordBatch {
        RecordBatch::try_new(
            Arc::new(Schema::new(vec![
                Field::new("id", DataType::Int64, false),
                Field::new("customer_id", DataType::Int64, false),
                Field::new("total", DataType::Int64, false),
            ])),
            vec![
                Arc::new(Int64Array::from(vec![10, 11, 12, 13])),
                Arc::new(Int64Array::from(vec![1, 1, 2, 3])),
                Arc::new(Int64Array::from(vec![100, 50, 70, 30])),
            ],
        )
        .unwrap()
    }

    /// Serves a source's tables from a LanceDB folder named by its source id, so the real
    /// query path runs without object storage.
    struct Folders(HashMap<String, PathBuf>);

    struct FolderTables(Connection);

    #[async_trait::async_trait]
    impl ExternalTableResolver for FolderTables {
        async fn table(&self, table: &str) -> flow_like_types::Result<Arc<dyn TableProvider>> {
            let table = self.0.open_table(table).execute().await?;
            Ok(lance_table_provider(table, false).await?)
        }
    }

    #[async_trait::async_trait]
    impl ExternalConnector for Folders {
        async fn resolver(
            &self,
            source: &ExternalSource,
        ) -> Result<Arc<dyn ExternalTableResolver>, WireError> {
            let path = self.0.get(&source.source_id).ok_or_else(|| {
                WireError::new(ErrorCode::InvalidRequest, "no folder for this source")
            })?;
            let connection = lancedb::connect(&path.to_string_lossy())
                .execute()
                .await
                .map_err(|error| WireError::new(ErrorCode::Unavailable, error.to_string()))?;
            Ok(Arc::new(FolderTables(connection)))
        }
    }

    fn folder_source(schema: &str, folder: &str) -> SourceSpec {
        SourceSpec {
            schema: Some(schema.to_string()),
            reader: ReaderSpec::Unrestricted,
            location: SourceLocation::External(ExternalSource {
                source_id: folder.to_string(),
                app_id: "app-1".to_string(),
                connection: ExternalConnection::Postgres {
                    host: "unused.example".to_string(),
                    port: 5432,
                    database: "unused".to_string(),
                    schema: "public".to_string(),
                    ssl_mode: "require".to_string(),
                },
                secret: SecretMaterial::None,
            }),
        }
    }

    fn service(folders: &[(&str, &TempApp)]) -> QueryService {
        let config = ServiceConfig {
            limits: ServiceLimits {
                max_rows: 100,
                max_memory_bytes: 64 * 1024 * 1024,
                max_timeout: Duration::from_secs(20),
                spill: None,
            },
            connector: Arc::new(Folders(
                folders
                    .iter()
                    .map(|(name, app)| (name.to_string(), app.path.clone()))
                    .collect(),
            )),
        };
        QueryService::with_transport(Arc::new(InProcessTransport::new(Arc::new(config))))
    }

    fn options(table: &str, column: &str, range: bool) -> Operation {
        Operation::ParameterOptions {
            table: table.to_string(),
            column: column.to_string(),
            search: None,
            range,
            shape: QueryShape::CrossApp,
        }
    }

    #[tokio::test]
    async fn cross_app_sql_runs_through_the_signed_client_and_the_in_process_service() {
        crate::backend_jwt::init_for_tests();
        let crm = temp_app("customers", customers()).await;
        let shop = temp_app("orders", orders()).await;
        let service = service(&[("crm-folder", &crm), ("shop-folder", &shop)]);
        assert!(service.in_process().await);
        let sources = vec![
            folder_source("crm", "crm-folder"),
            folder_source("shop", "shop-folder"),
        ];
        let run = |operation: Operation, rows: usize| {
            let request = request(sources.clone(), rows, operation);
            let service = &service;
            async move { service.run("user-1", &request).await }
        };

        let response = run(
            Operation::Sql {
                sql: "SELECT c.name, SUM(o.total) AS total FROM crm.customers c \
                      JOIN shop.orders o ON o.customer_id = c.id \
                      WHERE c.region = $region GROUP BY c.name ORDER BY c.name"
                    .to_string(),
                params: json!({ "region": "West" }),
                shape: QueryShape::CrossApp,
                reuse_if_unchanged: None,
            },
            10,
        )
        .await
        .unwrap();
        let rows = QueryRows::from_response(&response).unwrap();
        assert_eq!(
            rows.result.rows,
            vec![
                json!({ "name": "Acme", "total": 150 }),
                json!({ "name": "Initech", "total": 30 }),
            ]
        );
        assert!(!rows.result.truncated);
        assert_eq!(
            response.trailer.tables,
            vec!["crm.customers", "shop.orders"]
        );
        assert_eq!(
            response.trailer.unversioned_tables,
            vec!["crm.customers", "shop.orders"]
        );

        let truncated = run(
            Operation::Sql {
                sql: "SELECT id FROM shop.orders ORDER BY id".to_string(),
                params: Value::Null,
                shape: QueryShape::CrossApp,
                reuse_if_unchanged: None,
            },
            2,
        )
        .await
        .unwrap();
        let truncated = QueryRows::from_response(&truncated).unwrap();
        assert_eq!(truncated.result.row_count, 2);
        assert!(truncated.result.truncated);

        let description = json_answer(
            run(
                Operation::Describe {
                    sql: "SELECT name FROM crm.customers WHERE region = $region".to_string(),
                    shape: QueryShape::CrossApp,
                },
                1,
            )
            .await
            .unwrap(),
        )
        .unwrap();
        assert_eq!(description["columns"][0]["name"], "name");
        assert_eq!(description["parameters"][0]["name"], "region");
        assert_eq!(
            description["parameters"][0]["bindings"][0]["column"],
            "region"
        );

        let values = json_answer(
            run(options("crm.customers", "region", false), 1)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            values,
            json!({ "values": ["East", "West"], "truncated": false })
        );
        let range =
            json_answer(run(options("shop.orders", "total", true), 1).await.unwrap()).unwrap();
        assert_eq!(range, json!({ "min": 30, "max": 100 }));

        let broken = run(
            Operation::Sql {
                sql: "SELECT * FROM crm.missing".to_string(),
                params: Value::Null,
                shape: QueryShape::CrossApp,
                reuse_if_unchanged: None,
            },
            10,
        )
        .await
        .unwrap_err();
        assert_eq!(broken.status(), StatusCode::BAD_REQUEST);
        assert!(
            broken
                .public_message()
                .unwrap_or_default()
                .contains("crm.missing"),
            "{:?}",
            broken.public_message()
        );
    }
}

use axum::{Json, Router, extract::State, http::StatusCode, routing::get};
use flow_like_api::execution::state::ExecutionStateStore;
use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseConnection, Statement};
use serde::Serialize;
use std::{sync::Arc, time::Duration};

#[derive(Clone)]
struct Dependencies {
    database: DatabaseConnection,
    executions: Arc<dyn ExecutionStateStore>,
}

#[derive(Serialize)]
pub struct HealthResponse {
    status: String,
    version: String,
}

pub fn routes(database: DatabaseConnection, executions: Arc<dyn ExecutionStateStore>) -> Router {
    Router::new()
        .route("/live", get(liveness))
        .route("/ready", get(readiness))
        .route("/startup", get(startup))
        .with_state(Dependencies {
            database,
            executions,
        })
}

async fn liveness() -> (StatusCode, Json<HealthResponse>) {
    (
        StatusCode::OK,
        Json(HealthResponse {
            status: "healthy".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        }),
    )
}

async fn readiness(State(dependencies): State<Dependencies>) -> (StatusCode, Json<HealthResponse>) {
    // LIMIT 0 resolves the tables and columns without scanning application data.
    // The init container already waits for this release's migration Job.
    let probe = dependencies.database.query_all_raw(Statement::from_string(
        DatabaseBackend::Postgres,
        r#"SELECT u."id", a."id", r."id", r."status", r."expiresAt"
           FROM "User" u CROSS JOIN "App" a CROSS JOIN "ExecutionRun" r LIMIT 0"#,
    ));
    let check = async {
        probe.await.map_err(|_| ())?;
        dependencies
            .executions
            .get_run("__readiness_probe__")
            .await
            .map_err(|_| ())?;
        Ok::<_, ()>(())
    };
    let ready = matches!(
        tokio::time::timeout(Duration::from_secs(2), check).await,
        Ok(Ok(_))
    );
    (
        if ready {
            StatusCode::OK
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        },
        Json(HealthResponse {
            status: if ready {
                "ready"
            } else {
                "database, schema, or execution state unavailable"
            }
            .to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        }),
    )
}

async fn startup() -> (StatusCode, Json<HealthResponse>) {
    (
        StatusCode::OK,
        Json(HealthResponse {
            status: "started".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        }),
    )
}

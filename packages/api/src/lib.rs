use std::sync::Arc;

use axum::{
    Json, Router,
    middleware::{from_fn, from_fn_with_state},
    routing::{get, post},
};
use error::InternalError;
use flow_like_types::Value;
use middleware::error_reporting::error_reporting_middleware;
use middleware::jwt::jwt_middleware;
use state::{AppState, State};
use tower::ServiceBuilder;
use tower_http::{
    compression::{CompressionLayer, DefaultPredicate, Predicate, predicate::NotForContentType},
    cors::CorsLayer,
    decompression::RequestDecompressionLayer,
};
use tracing_subscriber::EnvFilter;
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

pub mod entity;
mod middleware;
pub mod openapi;
mod routes;

pub mod alerting;
pub mod audit;
pub mod cache;
#[cfg(feature = "cosmos")]
pub(crate) use flow_like_azure_data::cosmos;
pub mod credentials;
mod db_backfills;
pub mod error;
pub mod mail;
pub mod model_tier;
pub mod permission;
pub mod publication;
pub mod push_notifications;
pub mod state;
pub mod storage_config;
#[cfg(feature = "storage-queue")]
mod storage_queue;
pub mod telemetry;
pub mod usage_accounting;
pub mod usage_limits;
pub mod user_management;
pub mod utils;

pub mod app_connection_jwt;
pub mod backend_jwt;
pub mod compilation;
pub mod correlation;
pub mod execution;

pub use routes::registry::ServerRegistry;

#[cfg(feature = "kubernetes")]
pub mod kubernetes;

pub use axum;
pub mod auth {
    use crate::middleware;
    pub use middleware::jwt::AppUser;
}

pub use sea_orm;

pub fn warn_env_filter() -> EnvFilter {
    EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new("warn")
            .add_directive("hyper=warn".parse().unwrap())
            .add_directive("hyper_util=warn".parse().unwrap())
            .add_directive("rustls=warn".parse().unwrap())
            .add_directive("tokio=warn".parse().unwrap())
            .add_directive("h2=warn".parse().unwrap())
            .add_directive("tower=warn".parse().unwrap())
    })
}

pub fn construct_router(state: Arc<State>) -> Router {
    construct_router_with_cors(state, CorsLayer::permissive())
}

/// Construct the API router with an explicit CORS policy.
///
/// Deployment-specific entry points must use this function when a permissive
/// policy is not acceptable. Keeping CORS inside every nested route layer
/// prevents an inner wildcard response from bypassing a stricter outer layer.
pub fn construct_router_with_cors(state: Arc<State>, cors: CorsLayer) -> Router {
    // Executors are read-only on the meta store; guarantee compiled board
    // artifacts exist before every dispatch. Installed here because the
    // dispatcher is built before the state it needs exists.
    {
        let ensurer_state = state.clone();
        state.dispatcher.set_artifact_ensurer(std::sync::Arc::new(
            move |app_id, board_id, version| {
                let state = ensurer_state.clone();
                Box::pin(async move {
                    crate::execution::compiled_artifacts::ensure_compiled_artifact(
                        &state, &app_id, &board_id, version,
                    )
                    .await
                })
            },
        ));
    }

    if state.platform_config.audit.enabled && !audit::sign::is_signing_configured() {
        if state.platform_config.audit.require_signing {
            panic!(
                "AUDIT SIGNING REQUIRED but not configured. \
                 Set BACKEND_KEY (base64 P-256 PEM) and BACKEND_KID env vars."
            );
        }
        tracing::error!(
            "AUDIT SIGNING NOT CONFIGURED: Set BACKEND_KEY (base64 P-256 PEM) and BACKEND_KID. \
             Audit entries will be UNSIGNED until keys are provided."
        );
    }

    let router = Router::new()
        .route("/", get(hub_info))
        .nest("/health", routes::health::routes())
        .nest("/info", routes::info::routes())
        .nest("/user", routes::user::routes())
        .nest("/profile", routes::profile::routes())
        .nest("/apps", routes::app::routes())
        .nest("/bit", routes::bit::routes())
        .nest("/store", routes::store::routes())
        .nest("/auth", routes::auth::routes())
        .nest("/oauth", routes::oauth::routes())
        .nest("/chat", routes::chat::routes())
        .nest("/courses", routes::course::routes())
        .nest("/embeddings", routes::embeddings::routes())
        .nest("/ai", routes::ai::routes())
        .nest("/admin", routes::admin::routes())
        .nest("/tmp", routes::tmp::routes())
        .nest("/og", routes::og::routes())
        .nest("/solution", routes::solution::routes())
        .nest("/execution", routes::execution::routes())
        .nest("/interaction", routes::interaction::routes())
        .nest("/widget-query", routes::widget_query::routes())
        .nest("/maintenance", routes::maintenance::routes())
        .nest("/usage", routes::usage::routes())
        .nest("/registry", routes::registry::routes())
        .nest("/audit", routes::audit::routes())
        .nest("/sink", routes::sink::routes())
        .nest("/aliases", routes::alias::routes())
        .nest("/telemetry", routes::telemetry::routes())
        .nest("/flowscript", routes::flowscript::routes())
        .route("/webhook/stripe", post(routes::webhook::stripe_webhook))
        .with_state(state.clone())
        .route("/version", get(|| async { "0.0.0" }))
        .layer(from_fn_with_state(
            state.clone(),
            error_reporting_middleware,
        ))
        .layer(from_fn_with_state(state.clone(), jwt_middleware))
        .layer(cors.clone())
        .layer(
            ServiceBuilder::new()
                // .layer(TimeoutLayer::new(Duration::from_secs(15 * 60)))
                .layer(RequestDecompressionLayer::new())
                .layer(CompressionLayer::new().compress_when(
                    DefaultPredicate::new().and(NotForContentType::new("text/event-stream")),
                )),
        )
        // Outermost, so the server span covers the whole request and every
        // handler span nests inside the trace the client started.
        .layer(from_fn(telemetry::trace_context_middleware));

    // Inbound REST/MCP routers. They deliberately bypass the JWT
    // middleware (per-registration auth is enforced inside the handler)
    // but DO need CORS, compression, decompression and error reporting
    // so they behave like the rest of the API surface.
    let inbound_layers = ServiceBuilder::new()
        .layer(from_fn_with_state(
            state.clone(),
            error_reporting_middleware,
        ))
        .layer(cors.clone())
        .layer(RequestDecompressionLayer::new())
        .layer(CompressionLayer::new().compress_when(
            DefaultPredicate::new().and(NotForContentType::new("text/event-stream")),
        ));

    let inbound_rest = routes::inbound::rest_routes()
        .with_state(state.clone())
        .layer(inbound_layers.clone());
    let inbound_mcp = routes::inbound::mcp_routes()
        .with_state(state.clone())
        .layer(inbound_layers);

    Router::new()
        .merge(openapi_routes(cors))
        .nest("/r", inbound_rest)
        .nest("/m", inbound_mcp)
        .nest("/api/v1", router)
}

fn openapi_routes(cors: CorsLayer) -> Router {
    Router::from(
        SwaggerUi::new("/swagger-ui").url("/api-doc/openapi.json", openapi::ApiDoc::openapi()),
    )
    .layer(cors)
}

#[tracing::instrument(name = "GET /", skip(state))]
async fn hub_info(
    axum::extract::State(state): axum::extract::State<AppState>,
) -> Result<Json<Value>, InternalError> {
    // Serialize hub to JSON value so we can modify it
    let mut hub_value: Value = serde_json::to_value(&state.platform_config)?;

    // Strip sensitive OAuth fields (client_secret_env and client_secret)
    if let Some(oauth_providers) = hub_value.get_mut("oauth_providers")
        && let Some(providers_obj) = oauth_providers.as_object_mut()
    {
        for (_provider_id, provider_config) in providers_obj.iter_mut() {
            if let Some(config_obj) = provider_config.as_object_mut() {
                config_obj.remove("client_secret_env");
                config_obj.remove("client_secret");
            }
        }
    }

    Ok(Json(hub_value))
}

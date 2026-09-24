use std::sync::Arc;

use axum::{
    Json, Router,
    middleware::{from_fn, from_fn_with_state},
    routing::{get, post},
};
use error::InternalError;
use flow_like_types::Value;
use middleware::deadline::deadline_middleware;
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

pub use flow_like_api_entity as entity;
mod middleware;
pub mod openapi;
mod routes;

pub mod alerting;
pub mod audit;
mod bit_pricing;
pub mod cache;
pub mod capacity;
pub mod channel;
pub mod compute_attempts;
pub mod compute_cost;
#[cfg(feature = "cosmos")]
pub(crate) use flow_like_azure_data::cosmos;
pub mod credentials;
pub mod db;
pub mod deletion;
pub(crate) mod devices;
pub(crate) mod instances;
pub mod error;
pub mod mail;
pub mod model_tier;
pub mod notification_images;
pub mod package_license;
pub mod payments;
pub mod permission;
pub mod publication;
pub mod push_notifications;
pub mod quota;
#[cfg(test)]
mod quota_integration_tests;
pub mod quota_payloads;
pub mod quota_warnings;
pub mod realtime_ice;
pub(crate) mod rolling_usage;
mod runtime_config;
pub mod state;
pub mod storage_accounting;
pub mod storage_config;
pub mod storage_identity;
#[cfg(feature = "storage-queue")]
mod storage_queue;
pub mod stripe_connect;
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
    env_filter_defaulting_to("warn")
}

/// Filter for the long-running local API: our own `INFO` startup lines, with
/// the dependencies that are noisy at that level held down. `aws_config` is not
/// merely noisy - it logs the resolved access key id at `INFO`
/// (`loaded base credentials creds=Credentials { access_key_id: "AKIA…" }`), so
/// an unfiltered registry writes credential material into every log.
pub fn info_env_filter() -> EnvFilter {
    env_filter_defaulting_to("info")
}

fn env_filter_defaulting_to(level: &str) -> EnvFilter {
    EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new(level)
            .add_directive("hyper=warn".parse().unwrap())
            .add_directive("hyper_util=warn".parse().unwrap())
            .add_directive("rustls=warn".parse().unwrap())
            .add_directive("tokio=warn".parse().unwrap())
            .add_directive("h2=warn".parse().unwrap())
            .add_directive("tower=warn".parse().unwrap())
            .add_directive("aws_config=warn".parse().unwrap())
            .add_directive("aws_sigv4=warn".parse().unwrap())
            .add_directive("aws_smithy_runtime=warn".parse().unwrap())
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
    state.dispatcher.set_quota_state(&state);
    // Executors hold no meta-store credential and obtain their board only as
    // the presigned compiled artifact the dispatcher hands them, so the
    // artifact must exist before every dispatch. Installed here because the
    // dispatcher is built before the state it needs exists.
    {
        let ensurer_state = state.clone();
        state.dispatcher.set_artifact_ensurer(std::sync::Arc::new(
            move |app_id, board_id, version, board_etag, wasm_packages| {
                let state = ensurer_state.clone();
                Box::pin(async move {
                    crate::execution::compiled_artifacts::ensure_compiled_artifact(
                        &state,
                        &app_id,
                        &board_id,
                        version,
                        board_etag.as_deref(),
                        wasm_packages.as_ref(),
                    )
                    .await
                })
            },
        ));
    }

    {
        let resolver_state = state.clone();
        state
            .dispatcher
            .set_async_wasm_package_resolver(std::sync::Arc::new(move |app_id, target| {
                let state = resolver_state.clone();
                Box::pin(async move {
                    crate::execution::wasm_resolve::resolve_wasm_packages_for_platform(
                        &state, &app_id, &target,
                    )
                    .await
                })
            }));
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
        .nest("/devices", routes::devices::routes())
        .nest("/instances", routes::instances::routes())
        .nest("/oauth", routes::oauth::routes())
        .nest("/chat", routes::chat::routes())
        // Root-level: Rig's Responses client posts to `{base_url}/responses`,
        // and hosted Bits already point their base URL at `{api}/api/v1`.
        .route(
            "/responses",
            post(routes::chat::responses::invoke_responses),
        )
        .nest("/courses", routes::course::routes())
        .nest("/embeddings", routes::embeddings::routes())
        .nest("/ai", routes::ai::routes())
        .nest("/admin", routes::admin::routes())
        .nest("/tmp", routes::tmp::routes())
        .nest("/og", routes::og::routes())
        .nest("/solution", routes::solution::routes())
        .nest("/execution", routes::execution::routes())
        .nest("/channels", routes::channel::routes())
        .nest("/maintenance", routes::maintenance::routes())
        .nest("/usage", routes::usage::routes())
        .nest("/registry", routes::registry::routes())
        .nest("/audit", routes::audit::routes())
        .nest("/sink", routes::sink::routes())
        .nest("/aliases", routes::alias::routes())
        .nest("/telemetry", routes::telemetry::routes())
        .nest("/flowscript", routes::flowscript::routes())
        .route("/webhook/stripe", post(routes::webhook::stripe_webhook))
        .merge(payments::router())
        .with_state(state.clone())
        .route("/version", get(|| async { "0.0.0" }))
        .layer(from_fn_with_state(
            state.clone(),
            error_reporting_middleware,
        ))
        // Inside audit so a deadline 504 still gets its outcome record, inside
        // CORS so it carries the CORS headers, inside compression so a cut body
        // still ends with a valid encoder trailer.
        .layer(from_fn(deadline_middleware))
        .layer(from_fn_with_state(
            state.clone(),
            middleware::audit::audit_middleware,
        ))
        .layer(from_fn_with_state(state.clone(), jwt_middleware))
        .layer(cors.clone())
        .layer(
            ServiceBuilder::new()
                .layer(RequestDecompressionLayer::new())
                .layer(CompressionLayer::new().compress_when(
                    DefaultPredicate::new().and(NotForContentType::new("text/event-stream")),
                )),
        );

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

    // Router::layer keeps the deadline innermost: past decompression (whose body
    // type from_fn cannot accept inside a ServiceBuilder) and inside compression.
    let inbound_rest = routes::inbound::rest_routes()
        .with_state(state.clone())
        .layer(from_fn(deadline_middleware))
        .layer(inbound_layers.clone());
    let inbound_mcp = routes::inbound::mcp_routes()
        .with_state(state.clone())
        .layer(from_fn(deadline_middleware))
        .layer(inbound_layers.clone());
    let frontend = routes::frontend::routes()
        .with_state(state.clone())
        .layer(from_fn(deadline_middleware))
        .layer(inbound_layers.clone());
    let frontend_links = routes::frontend::link_routes()
        .with_state(state.clone())
        .layer(from_fn(deadline_middleware))
        .layer(inbound_layers);

    Router::new()
        .merge(openapi_routes(cors))
        .nest("/r", inbound_rest)
        .nest("/m", inbound_mcp)
        .nest("/frontend", frontend)
        .merge(frontend_links)
        .nest("/api/v1", router)
        // One outer boundary observes API, inbound REST/MCP, OpenAPI and fallbacks.
        .layer(from_fn(telemetry::trace_context_middleware))
}

fn openapi_routes(cors: CorsLayer) -> Router {
    Router::from(
        SwaggerUi::new("/swagger-ui").url("/api-doc/openapi.json", openapi::ApiDoc::openapi()),
    )
    .layer(from_fn(deadline_middleware))
    .layer(cors)
}

#[tracing::instrument(name = "GET /", skip(state))]
async fn hub_info(
    axum::extract::State(state): axum::extract::State<AppState>,
) -> Result<Json<Value>, InternalError> {
    Ok(Json(public_hub_value(serde_json::to_value(
        &state.platform_config,
    )?)))
}

fn public_hub_value(mut hub_value: Value) -> Value {
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

    // Realtime ICE provider settings name server-side secret-store entries.
    // Clients receive only the resulting short-lived ICE configuration from
    // the authenticated board realtime endpoint.
    if let Some(hub) = hub_value.as_object_mut() {
        hub.remove("realtime");
        if let Some(payments) = hub.get_mut("payments").and_then(Value::as_object_mut) {
            payments.retain(|key, _| {
                matches!(
                    key.as_str(),
                    "onboarding_enabled"
                        | "marketplace_enabled"
                        | "node_payments_enabled"
                        | "servicing_enabled"
                        | "livemode"
                        | "currencies"
                        | "marketplace_min_amount"
                        | "max_payment_amount"
                )
            });
        }
    }

    hub_value
}

#[cfg(test)]
mod hub_info_tests {
    use super::public_hub_value;
    use serde_json::json;

    #[test]
    fn public_hub_hides_server_side_credential_references() {
        let public = public_hub_value(json!({
            "name": "Test Hub",
            "oauth_providers": {
                "example": {
                    "client_secret_env": "OAUTH_CLIENT_SECRET",
                    "client_secret": "secret",
                    "client_id": "public-client-id"
                }
            },
            "realtime": {
                "ice": {
                    "provider": "cloudflare",
                    "turn_key_id_secret_ref": "CLOUDFLARE_TURN_KEY_ID",
                    "turn_key_api_token_secret_ref": "CLOUDFLARE_TURN_KEY_API_TOKEN"
                }
            }
        }));

        assert!(public.get("realtime").is_none());
        let provider = &public["oauth_providers"]["example"];
        assert!(provider.get("client_secret_env").is_none());
        assert!(provider.get("client_secret").is_none());
        assert_eq!(provider["client_id"], "public-client-id");
    }

    #[test]
    fn public_hub_exposes_payment_availability_without_seller_or_platform_identity() {
        let public = public_hub_value(
            json!({"payments":{"marketplace_enabled":true,"livemode":false,"seller_allowlist":["private-user"],"platform_account_id":"acct_private","legal_texts":[],"live_approved":true}}),
        );
        assert_eq!(
            public["payments"],
            json!({"marketplace_enabled":true,"livemode":false})
        );
    }
}

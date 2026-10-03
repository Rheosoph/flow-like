//! Executor → API: an app's declarative widgets and the widgets of its packages.
//!
//! The `Instantiate Widget` node used to read `apps/{app}/manifest.app` and
//! every `{widget}.widget` straight from the meta store — the one run-time read
//! that kept a storage credential in the executor. Executors already prove
//! their identity to this API with the executor JWT for progress reporting, so
//! widgets travel the same way. The executor caches the response for the run.
//!
//! The widgets of the packages added to an app are served here as well: which
//! packages an app pins is recorded in this API's database, and the executor
//! has neither a database nor an app manifest to read the pins from.

use crate::{
    error::ApiError,
    execution::{ExecutionClaims, verify_execution_jwt},
    routes::registry::server::ServerRegistry,
    state::AppState,
};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, header},
    response::IntoResponse,
};
use flow_like::a2ui::micro_widget::PackageWidgetRef;
use flow_like::a2ui::widget::Widget;
use flow_like::app::App;

/// Whether a run may read this app's widgets: its own app, or one it reached
/// through an app connection — the signed `app_chain` records exactly those.
fn executor_may_read_app(claims: &ExecutionClaims, app_id: &str) -> bool {
    claims.app_id == app_id
        || claims
            .app_chain
            .as_deref()
            .is_some_and(|chain| chain.iter().any(|chained| chained == app_id))
}

/// What both widget endpoints ask of a caller: the executor JWT of a run that
/// may read `app_id`.
fn ensure_executor_may_read_app(headers: &HeaderMap, app_id: &str) -> Result<(), ApiError> {
    let token = super::progress::extract_bearer_token(headers)?;
    let claims = verify_execution_jwt(token).map_err(|e| {
        tracing::warn!(error = %e, "Invalid execution JWT");
        ApiError::bad_request(format!("Invalid execution JWT: {}", e))
    })?;
    if !executor_may_read_app(&claims, app_id) {
        return Err(ApiError::forbidden(
            "execution is not bound to this app".to_string(),
        ));
    }
    Ok(())
}

/// The widgets of the packages `app_id` pins. A hub without a registry has no
/// packages, and the executor asks without knowing which kind of hub it runs
/// for: it gets an empty list, not the 503 of the registry routes. A database
/// failure stays typed, so a lost connection is answered as one.
async fn package_widgets(
    registry: Option<&ServerRegistry>,
    app_id: &str,
) -> Result<Vec<PackageWidgetRef>, sea_orm::DbErr> {
    match registry {
        Some(registry) => registry.app_widgets(app_id).await,
        None => Ok(Vec::new()),
    }
}

/// Declarative widgets of an app, for the executor running one of its boards.
#[utoipa::path(
    get,
    path = "/execution/apps/{app_id}/widgets",
    tag = "execution",
    description = "Declarative widgets of an app, served to the executor running one of its boards.",
    params(("app_id" = String, Path, description = "Application ID")),
    responses(
        (status = 200, description = "The app's widgets", body = String, content_type = "application/json"),
        (status = 400, description = "Invalid request or JWT"),
        (status = 403, description = "The run is not bound to this app")
    ),
    security(("executor_jwt" = []))
)]
#[tracing::instrument(name = "GET /execution/apps/{app_id}/widgets", skip(state, headers))]
pub async fn get_app_widgets(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(app_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    ensure_executor_may_read_app(&headers, &app_id)?;

    let app_state = state.master_state(&state).await?;
    let app = App::load(app_id, app_state).await?;
    let widgets: Vec<Widget> = app.get_widgets().await?;

    Ok((
        [(header::CACHE_CONTROL, HeaderValue::from_static("no-store"))],
        Json(widgets),
    ))
}

/// Widgets of the packages added to an app, for the executor running one of its boards.
#[utoipa::path(
    get,
    path = "/execution/apps/{app_id}/package-widgets",
    tag = "execution",
    description = "Widgets of the packages added to an app, served to the executor running one of its boards.",
    params(("app_id" = String, Path, description = "Application ID")),
    responses(
        (status = 200, description = "The widgets of the app's packages", body = String, content_type = "application/json"),
        (status = 400, description = "Invalid request or JWT"),
        (status = 403, description = "The run is not bound to this app")
    ),
    security(("executor_jwt" = []))
)]
#[tracing::instrument(
    name = "GET /execution/apps/{app_id}/package-widgets",
    skip(state, headers)
)]
pub async fn get_app_package_widgets(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(app_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    ensure_executor_may_read_app(&headers, &app_id)?;

    let widgets = package_widgets(state.wasm_registry.as_deref(), &app_id).await?;

    Ok((
        [(header::CACHE_CONTROL, HeaderValue::from_static("no-store"))],
        Json(widgets),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend_jwt::TokenType;
    use crate::execution::{ExecutionJwtParams, sign_execution_jwt};
    use axum::http::StatusCode;
    use flow_like_storage::files::store::FlowLikeStore;
    use flow_like_storage::object_store::memory::InMemory;
    use std::sync::Arc;

    fn claims(app_id: &str, chain: Option<Vec<&str>>) -> ExecutionClaims {
        ExecutionClaims {
            dispatch_hash: None,
            runtime_limit_ms: None,
            quota_receipt_url: None,
            sub: "user-1".into(),
            payer_sub: None,
            hosted_frontend: false,
            technical_user_id: None,
            run_id: "run-1".into(),
            app_id: app_id.into(),
            board_id: "board-1".into(),
            event_id: None,
            app_chain: chain.map(|c| c.into_iter().map(String::from).collect()),
            correlation: None,
            page_execution: None,
            shadow: None,
            callback_url: "https://api.test".into(),
            token_type: TokenType::Executor,
            iss: "flow-like".into(),
            aud: "flow-like-executor".into(),
            iat: 0,
            nbf: 0,
            exp: 0,
            jti: "jti-1".into(),
        }
    }

    #[test]
    fn a_run_reads_its_own_app_and_the_apps_it_was_chained_through_only() {
        assert!(executor_may_read_app(&claims("app-1", None), "app-1"));
        assert!(executor_may_read_app(
            &claims("app-2", Some(vec!["app-1", "app-2"])),
            "app-1"
        ));
        assert!(!executor_may_read_app(&claims("app-1", None), "app-9"));
        assert!(!executor_may_read_app(
            &claims("app-2", Some(vec!["app-1"])),
            "app-9"
        ));
    }

    fn bearer(app_id: &str, token_type: TokenType) -> HeaderMap {
        crate::backend_jwt::init_for_tests();
        let token = sign_execution_jwt(ExecutionJwtParams {
            user_id: "user-1".into(),
            technical_user_id: None,
            run_id: "run-1".into(),
            app_id: app_id.into(),
            board_id: "board-1".into(),
            event_id: None,
            app_chain: None,
            correlation: None,
            callback_url: "https://api.test".into(),
            token_type,
            ttl_seconds: None,
            shadow: None,
        })
        .unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {token}").parse().unwrap(),
        );
        headers
    }

    fn refusal(headers: &HeaderMap, app_id: &str) -> StatusCode {
        ensure_executor_may_read_app(headers, app_id)
            .expect_err("the request must be refused")
            .status()
    }

    #[test]
    fn widgets_are_served_to_the_executor_of_a_run_bound_to_the_app_only() {
        let executor = bearer("app-1", TokenType::Executor);
        assert!(ensure_executor_may_read_app(&executor, "app-1").is_ok());
        assert_eq!(refusal(&executor, "app-9"), StatusCode::FORBIDDEN);

        // The poll token the same run hands to the user who started it.
        let user = bearer("app-1", TokenType::User);
        assert_eq!(refusal(&user, "app-1"), StatusCode::BAD_REQUEST);
        let anonymous = HeaderMap::new();
        assert_eq!(refusal(&anonymous, "app-1"), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn a_hub_without_a_registry_lists_no_package_widgets() {
        assert!(package_widgets(None, "app-1").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_lost_database_is_answered_as_unavailable_not_as_an_internal_error() {
        // A closed lazy pool fails every query without a database to reach.
        let mut options = sea_orm::ConnectOptions::new("postgres://localhost/package_widgets_test");
        options.connect_lazy(true).min_connections(0);
        let db = sea_orm::Database::connect(options).await.unwrap();
        db.close_by_ref().await.unwrap();
        let store = Arc::new(FlowLikeStore::Memory(Arc::new(InMemory::new())));
        let registry = ServerRegistry::new(db, store.clone(), store);

        let error = package_widgets(Some(&registry), "app-1")
            .await
            .expect_err("the pool is closed");

        let error = ApiError::from(error);
        assert_eq!(error.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(error.public_code(), "DATABASE_UNAVAILABLE");
    }
}

//! Executor → API: an app's declarative widgets.
//!
//! The `Instantiate Widget` node used to read `apps/{app}/manifest.app` and
//! every `{widget}.widget` straight from the meta store — the one run-time read
//! that kept a storage credential in the executor. Executors already prove
//! their identity to this API with the executor JWT for progress reporting, so
//! widgets travel the same way. The executor caches the response for the run.

use crate::{
    error::ApiError,
    execution::{ExecutionClaims, verify_execution_jwt},
    state::AppState,
};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, header},
    response::IntoResponse,
};
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
    let token = super::progress::extract_bearer_token(&headers)?;
    let claims = verify_execution_jwt(token).map_err(|e| {
        tracing::warn!(error = %e, "Invalid execution JWT");
        ApiError::bad_request(format!("Invalid execution JWT: {}", e))
    })?;
    if !executor_may_read_app(&claims, &app_id) {
        return Err(ApiError::forbidden(
            "execution is not bound to this app".to_string(),
        ));
    }

    let app_state = state.master_state(&state).await?;
    let app = App::load(app_id, app_state).await?;
    let widgets: Vec<Widget> = app.get_widgets().await?;

    Ok((
        [(header::CACHE_CONTROL, HeaderValue::from_static("no-store"))],
        Json(widgets),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend_jwt::TokenType;

    fn claims(app_id: &str, chain: Option<Vec<&str>>) -> ExecutionClaims {
        ExecutionClaims {
            sub: "user-1".into(),
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
}

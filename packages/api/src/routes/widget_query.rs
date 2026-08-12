//! Live micro-widget query responses
//!
//! A running board can query a rendered micro widget (`a2ui_widget_query`
//! node → `A2UIServerMessage::WidgetQuery` over the run's event stream). The
//! surface hosting the widget answers through this endpoint; the pending
//! request is resolved in-process via the global frontend-request registry.
//!
//! Security:
//! - Authenticated endpoint (same middleware as every user route).
//! - The request id is an unguessable single-use capability (cuid2) minted by
//!   the run, delivered only over the caller's own authenticated event
//!   stream, and expiring with the node's timeout (≤10s).
//! - First response wins; late or duplicate deliveries report `accepted: false`.

use crate::{error::ApiError, middleware::jwt::AppUser, state::AppState};
use axum::{Extension, Json, Router, extract::Path, routing::post};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

pub fn routes() -> Router<AppState> {
    Router::new().route("/{request_id}/respond", post(respond_widget_query))
}

/// Query result envelope produced by the widget's `query:result` message
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct WidgetQueryRespondRequest {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Object>)]
    pub value: Option<flow_like_types::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct WidgetQueryRespondResponse {
    /// False when the query already timed out, was answered by another
    /// surface, or the id is unknown.
    pub accepted: bool,
}

/// Deliver a micro-widget query result to the board run awaiting it.
#[utoipa::path(
    post,
    path = "/widget-query/{request_id}/respond",
    params(("request_id" = String, Path, description = "Pending widget query request identifier")),
    request_body = WidgetQueryRespondRequest,
    responses(
        (status = 200, description = "Delivery result; accepted is false when the query already completed or timed out", body = WidgetQueryRespondResponse)
    ),
    tag = "Widget Queries"
)]
pub async fn respond_widget_query(
    Extension(_user): Extension<AppUser>,
    Path(request_id): Path<String>,
    Json(body): Json<WidgetQueryRespondRequest>,
) -> Result<Json<WidgetQueryRespondResponse>, ApiError> {
    let response = flow_like_types::json::to_value(&body)
        .map_err(|e| ApiError::bad_request(format!("Invalid widget query response: {e}")))?;

    let accepted =
        flow_like_types::frontend_request::resolve_frontend_request(&request_id, response).await;

    Ok(Json(WidgetQueryRespondResponse { accepted }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::json::json;

    #[tokio::test]
    async fn test_respond_resolves_pending_request() {
        let receiver =
            flow_like_types::frontend_request::register_frontend_request("wq-test-1").await;

        let body = WidgetQueryRespondRequest {
            ok: true,
            value: Some(json!({"rows": [1, 2, 3]})),
            error: None,
        };
        let response = flow_like_types::json::to_value(&body).unwrap();
        let accepted =
            flow_like_types::frontend_request::resolve_frontend_request("wq-test-1", response)
                .await;
        assert!(accepted);

        let delivered = receiver.await.unwrap();
        assert_eq!(delivered["ok"], json!(true));
        assert_eq!(delivered["value"]["rows"][2], json!(3));

        let late = WidgetQueryRespondRequest {
            ok: false,
            value: None,
            error: Some("late".into()),
        };
        let late_value = flow_like_types::json::to_value(&late).unwrap();
        assert!(
            !flow_like_types::frontend_request::resolve_frontend_request("wq-test-1", late_value)
                .await
        );
    }
}

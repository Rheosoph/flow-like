use crate::{
    ensure_permission,
    entity::event,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::inbound::{MemberExecutionIdentity, dispatch_member_mcp_operation},
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpOperationRequest {
    pub method: String,
    pub params: Option<Value>,
    pub profile_id: Option<String>,
}

fn operation_params(method: &str, params: Option<Value>) -> Result<Value, ApiError> {
    if !matches!(
        method,
        "tools/list"
            | "tools/call"
            | "resources/list"
            | "resources/templates/list"
            | "resources/read"
            | "prompts/list"
            | "prompts/get"
    ) {
        return Err(ApiError::bad_request("Unsupported MCP operation"));
    }
    let params = params.unwrap_or_else(|| json!({}));
    let Some(object) = params.as_object() else {
        return Err(ApiError::bad_request("MCP params must be an object"));
    };
    let required_name = match method {
        "tools/call" | "prompts/get" => Some("name"),
        "resources/read" => Some("uri"),
        _ => None,
    };
    if let Some(key) = required_name
        && object
            .get(key)
            .and_then(Value::as_str)
            .is_none_or(|value| value.trim().is_empty())
    {
        return Err(ApiError::bad_request(format!("{method} requires {key}")));
    }
    if matches!(method, "tools/call" | "prompts/get")
        && object
            .get("arguments")
            .is_some_and(|value| !value.is_object())
    {
        return Err(ApiError::bad_request("MCP arguments must be an object"));
    }
    Ok(params)
}

fn ensure_member_caller(user: &AppUser) -> Result<(), ApiError> {
    if user.is_connected_app() {
        return Err(ApiError::forbidden(
            "Connected apps must call MCP through the event's public endpoint or app-connection proxy",
        ));
    }
    Ok(())
}

/// Consume registered MCP operations with the same app permission as event invocation.
/// External MCP clients and connected apps retain their own transport and auth checks.
#[tracing::instrument(
    name = "POST /apps/{app_id}/events/{event_id}/mcp-operation",
    skip(state, user, body)
)]
pub async fn invoke_mcp_operation(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, event_id)): Path<(String, String)>,
    Json(body): Json<McpOperationRequest>,
) -> Result<Json<Value>, ApiError> {
    ensure_member_caller(&user)?;
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::ExecuteEvents);
    let user_id = permission.effective_user_id().map_err(|_| {
        ApiError::forbidden("Invoking requires a caller that is linked to a user account")
    })?;
    let params = operation_params(&body.method, body.params)?;
    let event_row = event::Entity::find_by_id(&event_id)
        .filter(event::Column::AppId.eq(&app_id))
        .one(&state.db)
        .await?
        .ok_or_else(|| ApiError::not_found("Event not found"))?;
    let identity = MemberExecutionIdentity {
        user_id,
        technical_user_id: permission.technical_user_id().map(ToOwned::to_owned),
        profile_id: body.profile_id,
    };
    dispatch_member_mcp_operation(
        &state,
        &event_row,
        &body.method,
        params,
        identity,
        permission.to_user_context(),
    )
    .await
    .map(Json)
}

#[cfg(test)]
mod tests {
    use super::{McpOperationRequest, ensure_member_caller, operation_params};
    use crate::middleware::jwt::{AppUser, ConnectedAppUser, OpenIDUser};
    use axum::http::StatusCode;
    use serde_json::json;

    #[test]
    fn tools_can_be_called_without_arguments_or_a_payload_pin() {
        assert_eq!(operation_params("tools/list", None).unwrap(), json!({}));
        assert_eq!(
            operation_params("tools/call", Some(json!({"name": "list_notes"}))).unwrap(),
            json!({"name": "list_notes"})
        );
    }

    #[test]
    fn rejects_transport_methods_and_malformed_operation_inputs() {
        for method in [
            "initialize",
            "notifications/initialized",
            "execute_node",
            "",
        ] {
            assert!(operation_params(method, None).is_err());
        }
        for params in [
            json!([]),
            json!({}),
            json!({"name": ""}),
            json!({"name": "list_notes", "arguments": []}),
        ] {
            assert!(operation_params("tools/call", Some(params)).is_err());
        }
        assert!(
            serde_json::from_value::<McpOperationRequest>(json!({
                "method": "tools/call", "params": {"name": "list_notes"}, "node_id": "private"
            }))
            .is_err()
        );
    }

    #[test]
    fn connected_apps_cannot_use_member_auth_to_bypass_mcp_registration_auth() {
        let connected = AppUser::ConnectedApp(ConnectedAppUser {
            sub: Some("user".to_string()),
            origin_app_id: "source".to_string(),
            target_app_id: "target".to_string(),
            app_chain: vec!["source".to_string()],
            technical_user_id: None,
            run_id: None,
            correlation: None,
        });
        assert_eq!(
            ensure_member_caller(&connected).unwrap_err().status(),
            StatusCode::FORBIDDEN
        );
        assert!(
            ensure_member_caller(&AppUser::OpenID(OpenIDUser {
                sub: "user".to_string(),
                access_token: "token".to_string(),
            }))
            .is_ok()
        );
    }
}

//! Connects to a stored external database through the query service, with its stored settings
//! and credentials, and lists the tables queries can address. The outcome is recorded on the
//! source; only a failure of the query service itself is an error of this route.

use super::{
    model::DataSourceCheck,
    query_source::{external_source_for_query, record_check},
};
use crate::{
    ensure_permission, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, query, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
};
use flow_like_query::Operation;
use flow_like_types::Value;

const CHECK_OK: &str = "ok";
/// The service registers every external source under a schema; a check runs no SQL, so any
/// name does.
const CHECK_SCHEMA: &str = "source";

#[utoipa::path(
    post,
    path = "/apps/{app_id}/data-sources/{source_id}/check",
    tag = "database",
    description = "Connect to one of the app's external databases with its stored settings and credentials, and list the tables queries can address. The outcome is saved as the database's last check.",
    params(
        ("app_id" = String, Path, description = "App id"),
        ("source_id" = String, Path, description = "External database id")
    ),
    responses(
        (status = 200, description = "Whether connecting worked, with the tables found or the error", body = DataSourceCheck),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "You cannot read this app's databases"),
        (status = 404, description = "External database not found"),
        (status = 503, description = "The query service is unavailable")
    ),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
#[tracing::instrument(name = "POST /apps/{app_id}/data-sources/{source_id}/check", skip_all)]
pub async fn check_data_source(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, source_id)): Path<(String, String)>,
) -> Result<Json<DataSourceCheck>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::ReadDatabase);
    let source = external_source_for_query(&state, &app_id, &source_id).await?;
    let request = query::request(
        vec![query::external_source(
            source,
            Some(CHECK_SCHEMA.to_string()),
        )],
        1,
        Operation::ExternalTest,
    );
    let answer = state
        .query
        .run(&query::subject(&user), &request)
        .await
        .and_then(query::json_answer);
    let check = outcome(answer, &source_id)?;
    record_check(
        &state,
        &app_id,
        &source_id,
        check.error.as_deref().unwrap_or(CHECK_OK),
    )
    .await?;
    Ok(Json(check))
}

/// A refused connection, wrong credentials or a blocked address come back as a rejected request
/// and make a failed check; the query service being unreachable is the route's own error.
fn outcome(answer: Result<Value, ApiError>, source_id: &str) -> Result<DataSourceCheck, ApiError> {
    match answer {
        Ok(answer) => Ok(DataSourceCheck {
            ok: true,
            tables: tables_of(&answer, source_id),
            error: None,
        }),
        Err(error) if error.status() == StatusCode::BAD_REQUEST => Ok(DataSourceCheck {
            ok: false,
            tables: Vec::new(),
            error: Some(
                error
                    .public_message()
                    .unwrap_or("The database refused the connection")
                    .to_string(),
            ),
        }),
        Err(error) => Err(error),
    }
}

/// The table names the service listed for `source_id`, in its `{ sources: [...] }` answer.
fn tables_of(answer: &Value, source_id: &str) -> Vec<String> {
    answer["sources"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|source| source["source_id"].as_str() == Some(source_id))
        .and_then(|source| source["tables"].as_array())
        .into_iter()
        .flatten()
        .filter_map(|table| table.as_str().map(str::to_string))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn listed_tables_make_a_passed_check() {
        let answer = json!({ "sources": [
            { "source_id": "other", "tables": ["x"] },
            { "source_id": "s1", "tables": ["orders", "public.customers"] },
        ] });
        let check = outcome(Ok(answer), "s1").unwrap();
        assert!(check.ok && check.error.is_none());
        assert_eq!(check.tables, vec!["orders", "public.customers"]);

        let empty = outcome(Ok(json!({ "sources": [] })), "s1").unwrap();
        assert!(empty.ok && empty.tables.is_empty());
    }

    #[test]
    fn a_refused_connection_is_a_failed_check_and_an_unreachable_service_an_error() {
        let refused = outcome(
            Err(ApiError::bad_request("password authentication failed")),
            "s1",
        )
        .unwrap();
        assert!(!refused.ok && refused.tables.is_empty());
        assert_eq!(
            refused.error.as_deref(),
            Some("password authentication failed")
        );

        let unavailable = outcome(Err(ApiError::service_unavailable("down")), "s1").unwrap_err();
        assert_eq!(unavailable.status(), StatusCode::SERVICE_UNAVAILABLE);
    }
}

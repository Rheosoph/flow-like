use async_trait::async_trait;
use axum::{
    Extension,
    extract::{Path, Query, State},
};
use flow_like::flow::copilot::{QueryExecutionLogsArgs, platform::PlatformToolBridge};
use serde_json::{Value, json};

use crate::{
    error::ApiError,
    middleware::jwt::AppUser,
    routes::app::board::query_logs::{QueryLogsRequest, query_logs},
    state::AppState,
};

const LOG_TOOL: &str = "query_execution_logs";
const MAX_RUN_ID_BYTES: usize = 512;

pub(super) fn validate_run_id(run_id: &str) -> Result<(), ApiError> {
    if run_id.len() > MAX_RUN_ID_BYTES
        || flow_like_storage::lancedb::utils::validate_table_name(run_id).is_err()
    {
        return Err(ApiError::bad_request(
            "An exact run id of at most 512 bytes is required; use letters, digits, '_', '-' or '.'.",
        ));
    }
    Ok(())
}

/// Reads one persisted board's logs as the person who opened the specialist.
pub(super) struct ServerBoardLogsBridge {
    state: AppState,
    user: AppUser,
    app_id: String,
    board_id: String,
}

impl ServerBoardLogsBridge {
    pub(super) fn new(state: AppState, user: AppUser, app_id: String, board_id: String) -> Self {
        Self {
            state,
            user,
            app_id,
            board_id,
        }
    }
}

fn scoped_query(
    app_id: &str,
    board_id: &str,
    tool_name: &str,
    arguments: Value,
) -> Result<QueryLogsRequest, ApiError> {
    if tool_name != LOG_TOOL {
        return Err(ApiError::forbidden(
            "This cloud board bridge only reads persisted execution logs.",
        ));
    }
    let args: QueryExecutionLogsArgs = serde_json::from_value(arguments)
        .map_err(|_| ApiError::bad_request("A board id and exact run id are required."))?;
    if args.app_id.as_deref().is_some_and(|id| id != app_id) || args.board_id != board_id {
        return Err(ApiError::forbidden(
            "Execution logs must belong to the app and board opened by this specialist.",
        ));
    }
    validate_run_id(&args.run_id)?;
    // Model-supplied metadata never changes the board, principal or storage backend.
    Ok(QueryLogsRequest {
        run_id: args.run_id,
        query: args.filter,
        limit: Some(args.limit.unwrap_or(100).clamp(1, 100)),
        offset: Some(args.offset.unwrap_or(0)),
    })
}

fn tool_error(error: ApiError) -> String {
    json!({
        "status": "error",
        "tool": LOG_TOOL,
        "code": error.public_code(),
        "http_status": error.status().as_u16(),
        "error": error.public_message().unwrap_or("Execution logs could not be read."),
    })
    .to_string()
}

#[async_trait]
impl PlatformToolBridge for ServerBoardLogsBridge {
    fn supports_tool(&self, tool_name: &str) -> bool {
        tool_name == LOG_TOOL
    }

    async fn call(&self, tool_name: &str, arguments: Value) -> String {
        let request = match scoped_query(&self.app_id, &self.board_id, tool_name, arguments) {
            Ok(request) => request,
            Err(error) => return tool_error(error),
        };
        let run_id = request.run_id.clone();
        let limit = request.limit.unwrap_or(100);
        let offset = request.offset.unwrap_or(0);
        let filter = request.query.clone().unwrap_or_default();
        // The normal API handler rechecks ReadBoards and ReadLogs on every call and obtains
        // read-only credentials for this user and app. Board visibility alone grants no logs.
        match query_logs(
            State(self.state.clone()),
            Extension(self.user.clone()),
            Path((self.app_id.clone(), self.board_id.clone())),
            Query(request),
        )
        .await
        {
            Ok(logs) => json!({
                "status": "ok",
                "app_id": self.app_id,
                "board_id": self.board_id,
                "run_id": run_id,
                "filter": filter,
                "offset": offset,
                "limit": limit,
                "log_count": logs.0.len(),
                "has_more": logs.0.len() == limit,
                "logs": logs.0,
            })
            .to_string(),
            Err(error) => tool_error(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;

    use super::*;

    #[test]
    fn log_queries_cannot_escape_the_specialists_board() {
        for arguments in [
            json!({ "app_id": "other-app", "board_id": "board", "run_id": "run" }),
            json!({ "board_id": "other-board", "run_id": "run" }),
        ] {
            let error = scoped_query("app", "board", LOG_TOOL, arguments).unwrap_err();
            assert_eq!(error.status(), StatusCode::FORBIDDEN);
        }
        for tool in [
            "execute_event",
            "execute_node",
            "run_board_tests",
            "database_tool",
        ] {
            let error = scoped_query(
                "app",
                "board",
                tool,
                json!({ "app_id": "app", "board_id": "board", "run_id": "run" }),
            )
            .unwrap_err();
            assert_eq!(error.status(), StatusCode::FORBIDDEN);
        }
    }

    #[test]
    fn log_queries_preserve_pagination_and_filter_with_bounded_pages() {
        let request = scoped_query("app", "board", LOG_TOOL, json!({
            "board_id": "board", "run_id": "run", "filter": "log_level >= 3",
            "offset": 200, "limit": 900,
            "run_metadata": { "app_id": "other-app", "board_id": "other-board", "is_remote": false }
        })).unwrap();
        assert_eq!(request.run_id, "run");
        assert_eq!(request.query.as_deref(), Some("log_level >= 3"));
        assert_eq!(request.offset, Some(200));
        assert_eq!(request.limit, Some(100));

        let request = scoped_query(
            "app",
            "board",
            LOG_TOOL,
            json!({
                "app_id": "app", "board_id": "board", "run_id": "run", "limit": 0
            }),
        )
        .unwrap();
        assert_eq!(request.offset, Some(0));
        assert_eq!(request.limit, Some(1));
    }

    #[test]
    fn log_queries_require_an_exact_run_and_board() {
        for arguments in [
            json!({ "run_id": "run" }),
            json!({ "board_id": "board" }),
            json!({ "board_id": "board", "run_id": " " }),
        ] {
            assert_eq!(
                scoped_query("app", "board", LOG_TOOL, arguments)
                    .unwrap_err()
                    .status(),
                StatusCode::BAD_REQUEST,
            );
        }
        for run_id in [" run", "run ", "invalid/run", "run\n", &"r".repeat(513)] {
            assert_eq!(
                scoped_query(
                    "app",
                    "board",
                    LOG_TOOL,
                    json!({ "board_id": "board", "run_id": run_id }),
                )
                .unwrap_err()
                .status(),
                StatusCode::BAD_REQUEST,
                "invalid run id reached storage: {run_id:?}"
            );
        }
    }
}

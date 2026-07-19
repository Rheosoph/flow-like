pub mod apply_flowscript;
pub mod delete_board;
pub mod execute_commands;
pub mod flow_ir_commit;
pub mod get_board;
pub mod get_board_versions;
pub mod get_boards;
pub mod get_execution_elements;
pub mod get_flowscript;
pub mod get_runs;
pub mod invoke_board;
pub mod invoke_board_async;
pub mod prerun_board;
pub mod query_logs;
pub mod realtime;
pub mod report_run;
pub mod scoring;
pub mod secrets;
pub mod summaries;
pub mod undo_redo_board;
pub mod upsert_board;
pub mod version_board;
pub mod workspace;

use axum::{
    Router,
    extract::DefaultBodyLimit,
    routing::{get, patch, post},
};

use crate::{error::ApiError, middleware::jwt::AppUser, state::AppState};

/// Board command batches (bulk FlowScript applies, large undo/redo stacks) can
/// exceed axum's 2MB default body limit. Clients cap their payloads well below
/// this value and fail fast instead of hitting a raw 413.
const BOARD_COMMAND_BODY_LIMIT_BYTES: usize = 16 * 1024 * 1024;

/// Board invocation, prerun, and realtime access all take an arbitrary board
/// id and either execute it or disclose its full definition. Human/API
/// principals keep the existing permission checks in the handlers. Connected
/// apps must never use these generic board surfaces because they would let a
/// connected app choose arbitrary board/node IDs; they have to enter through a
/// callable event or the proxy instead.
pub(crate) fn ensure_connected_app_board_invoke_denied(user: &AppUser) -> Result<(), ApiError> {
    if user.is_connected_app() {
        return Err(ApiError::forbidden(
            "Connected apps must reach workflows through an event endpoint or proxy, not the generic board surface",
        ));
    }
    Ok(())
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(get_boards::get_boards))
        .route("/summaries", get(summaries::board_summaries))
        .route(
            "/{board_id}",
            get(get_board::get_board)
                .post(execute_commands::execute_commands)
                .patch(version_board::version_board)
                .put(upsert_board::upsert_board)
                .delete(delete_board::delete_board)
                .layer(DefaultBodyLimit::max(BOARD_COMMAND_BODY_LIMIT_BYTES)),
        )
        .route(
            "/{board_id}/version",
            get(get_board_versions::get_board_versions),
        )
        .route(
            "/{board_id}/flowscript",
            get(get_flowscript::get_flowscript),
        )
        .route(
            "/{board_id}/flowscript/apply",
            post(apply_flowscript::apply_flowscript),
        )
        .route(
            "/{board_id}/flow-ir-commit/disposition",
            post(flow_ir_commit::flow_ir_commit_disposition),
        )
        .route(
            "/{board_id}/flow-ir-commit/apply",
            post(flow_ir_commit::apply_flow_ir_commit),
        )
        .route(
            "/{board_id}/realtime",
            get(realtime::jwks).post(realtime::access),
        )
        .route("/{board_id}/runs", get(get_runs::get_runs))
        .route("/{board_id}/runs/report", post(report_run::report_run))
        .route("/{board_id}/logs", get(query_logs::query_logs))
        .route(
            "/{board_id}/elements",
            get(get_execution_elements::get_execution_elements),
        )
        .route("/{board_id}/prerun", get(prerun_board::prerun_board))
        .route(
            "/{board_id}/undo",
            patch(undo_redo_board::undo_board)
                .layer(DefaultBodyLimit::max(BOARD_COMMAND_BODY_LIMIT_BYTES)),
        )
        .route(
            "/{board_id}/redo",
            patch(undo_redo_board::redo_board)
                .layer(DefaultBodyLimit::max(BOARD_COMMAND_BODY_LIMIT_BYTES)),
        )
        .route("/{board_id}/invoke", post(invoke_board::invoke_board))
        .route(
            "/{board_id}/invoke/async",
            post(invoke_board_async::invoke_board_async),
        )
        .route("/{board_id}/workspace", get(workspace::workspace))
}

#[cfg(test)]
mod tests {
    use super::ensure_connected_app_board_invoke_denied;
    use crate::middleware::jwt::{AppUser, ConnectedAppUser};

    #[test]
    fn connected_apps_cannot_invoke_arbitrary_boards_directly() {
        let connected = AppUser::ConnectedApp(ConnectedAppUser {
            sub: Some("user".to_string()),
            origin_app_id: "source".to_string(),
            target_app_id: "target".to_string(),
            app_chain: vec!["source".to_string()],
            technical_user_id: None,
            run_id: None,
            correlation: None,
        });

        assert!(ensure_connected_app_board_invoke_denied(&connected).is_err());
        assert!(ensure_connected_app_board_invoke_denied(&AppUser::Unauthorized).is_ok());
    }
}

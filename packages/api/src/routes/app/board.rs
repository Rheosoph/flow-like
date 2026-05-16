pub mod delete_board;
pub mod execute_commands;
pub mod get_board;
pub mod get_board_versions;
pub mod get_boards;
pub mod get_execution_elements;
pub mod get_runs;
pub mod invoke_board;
pub mod invoke_board_async;
pub mod prerun_board;
pub mod query_logs;
pub mod realtime;
pub mod report_run;
pub mod secrets;
pub mod summaries;
pub mod undo_redo_board;
pub mod upsert_board;
pub mod version_board;
pub mod workspace;

use axum::{
    Router,
    routing::{get, patch, post},
};

use crate::state::AppState;

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
                .delete(delete_board::delete_board),
        )
        .route(
            "/{board_id}/version",
            get(get_board_versions::get_board_versions),
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
        .route("/{board_id}/undo", patch(undo_redo_board::undo_board))
        .route("/{board_id}/redo", patch(undo_redo_board::redo_board))
        .route("/{board_id}/invoke", post(invoke_board::invoke_board))
        .route(
            "/{board_id}/invoke/async",
            post(invoke_board_async::invoke_board_async),
        )
        .route("/{board_id}/workspace", get(workspace::workspace))
}

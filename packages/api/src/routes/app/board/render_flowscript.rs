use crate::{
    ensure_permission, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::board::get_flowscript::FlowScriptResponse, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like::flow::{
    ast::{RenderOptions, board_to_flowscript},
    board::Board,
};
use serde::Deserialize;
use utoipa::ToSchema;

#[derive(Clone, Deserialize, ToSchema)]
pub struct RenderFlowScriptRequest {
    /// The board to render, as returned by `GET /apps/{app_id}/board/{board_id}` (optionally with
    /// a `version`). Its `id` must equal the `board_id` path parameter.
    #[schema(value_type = Object)]
    pub board: Board,
    /// Include `//@n:<id>` anchor comments (default: true).
    #[serde(default)]
    pub anchors: Option<bool>,
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/board/{board_id}/flowscript/render",
    tag = "boards",
    description = "Renders a board the client already holds, such as a stored version snapshot, as FlowScript without reading or changing anything on the server.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("board_id" = String, Path, description = "Board ID; must match the `id` of the board in the body")
    ),
    request_body = RenderFlowScriptRequest,
    responses(
        (status = 200, description = "The board rendered as FlowScript source text", body = FlowScriptResponse),
        (status = 400, description = "The board in the body is not the board named in the path"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 413, description = "The board exceeds the request body limit")
    )
)]
#[tracing::instrument(
    name = "POST /apps/{app_id}/board/{board_id}/flowscript/render",
    skip(state, user, request)
)]
pub async fn render_flowscript(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, board_id)): Path<(String, String)>,
    Json(request): Json<RenderFlowScriptRequest>,
) -> Result<Json<FlowScriptResponse>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::ReadBoards);
    render_board(&board_id, &request).map(Json)
}

fn render_board(
    board_id: &str,
    request: &RenderFlowScriptRequest,
) -> Result<FlowScriptResponse, ApiError> {
    if request.board.id != board_id {
        return Err(ApiError::bad_request(format!(
            "board `{}` in the body does not match board `{board_id}` in the path",
            request.board.id
        )));
    }
    let options = RenderOptions {
        anchors: request.anchors.unwrap_or(true),
        ..RenderOptions::default()
    };
    Ok(FlowScriptResponse {
        flowscript: board_to_flowscript(&request.board, &options),
        scope_anchors: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;
    use flow_like::flow::{
        board::{Comment, CommentType},
        node::Node,
        pin::ValueType,
        variable::{Variable, VariableType},
    };
    use std::time::SystemTime;

    fn board() -> Board {
        let mut board =
            Board::new_detached(Some("board-1".into()), flow_like_storage::Path::default());
        let mut node = Node::new("events_simple", "Simple Event", "Starts the flow", "Events");
        node.add_output_pin(
            "exec_out",
            "Output",
            "Starts the flow",
            VariableType::Execution,
        );
        node.add_output_pin("payload", "Payload", "Event payload", VariableType::Struct);
        board.nodes.insert(node.id.clone(), node);
        let variable = Variable::new("counter", VariableType::Integer, ValueType::Normal);
        board.variables.insert(variable.id.clone(), variable);
        let comment = Comment {
            id: "comment-1".into(),
            author: None,
            content: "note".into(),
            comment_type: CommentType::Text,
            timestamp: SystemTime::now(),
            coordinates: (1.0, 2.0, 0.0),
            width: None,
            height: None,
            layer: None,
            color: None,
            z_index: None,
            hash: None,
            is_locked: None,
            node_id: None,
        };
        board.comments.insert(comment.id.clone(), comment);
        board
    }

    fn request_for(board: &Board, anchors: Option<bool>) -> RenderFlowScriptRequest {
        let body = serde_json::json!({
            "board": serde_json::to_value(board).expect("board serializes"),
            "anchors": anchors,
        });
        serde_json::from_slice(&serde_json::to_vec(&body).unwrap())
            .expect("a board as served by GET /board/{id} deserializes from JSON")
    }

    #[test]
    fn a_served_board_round_trips_through_json_and_renders_like_the_original() {
        let original = board();
        for anchors in [None, Some(false), Some(true)] {
            let request = request_for(&original, anchors);
            let options = RenderOptions {
                anchors: anchors.unwrap_or(true),
                ..RenderOptions::default()
            };
            let rendered = render_board("board-1", &request).expect("renders");
            assert_eq!(
                rendered.flowscript,
                board_to_flowscript(&original, &options)
            );
            assert!(rendered.scope_anchors.is_none());
        }
    }

    #[test]
    fn a_board_that_is_not_the_path_board_is_rejected() {
        let request = request_for(&board(), None);
        let error = render_board("board-2", &request)
            .err()
            .expect("mismatched ids must be rejected");
        assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    }
}

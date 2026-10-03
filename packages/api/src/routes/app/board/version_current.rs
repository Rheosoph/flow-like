//! `…/board/{board_id}/version/current`: the flow as a version. Deploying an event that
//! follows Latest reads which published version equals the stored flow, and publishes one
//! when none does.

use crate::{
    ensure_permission, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like::flow::board::{
    Board, BoardVersionCurrent, BoardVersionPublished, DRAFT_NOT_COMPARABLE,
};

#[utoipa::path(
    get,
    path = "/apps/{app_id}/board/{board_id}/version/current",
    tag = "boards",
    description = "Tells whether a flow has edits that no published version holds: the published version that equals the flow as it is stored now, and the newest published version.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("board_id" = String, Path, description = "Board ID")
    ),
    responses(
        (status = 200, description = "The version that equals the stored flow, and the newest version; each is null when there is none", body = BoardVersionCurrent),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Board not found")
    )
)]
#[tracing::instrument(
    name = "GET /apps/{app_id}/board/{board_id}/version/current",
    skip(state, user)
)]
pub async fn get_version_current(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, board_id)): Path<(String, String)>,
) -> Result<Json<BoardVersionCurrent>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::ReadBoards);
    let sub = permission.sub()?;
    let board = state
        .master_board(&sub, &app_id, &board_id, &state, None)
        .await?;
    Ok(Json(board.version_current(None).await?))
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/board/{board_id}/version/current",
    tag = "boards",
    description = "Publishes the flow as it is stored now as a new patch version, unless its newest version already equals it. Deploying an event that follows Latest to a device calls this, so the device receives the current flow. The version stays in the flow's history.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("board_id" = String, Path, description = "Board ID")
    ),
    responses(
        (status = 200, description = "The version that holds the stored flow, and whether this call created it", body = BoardVersionPublished),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Board not found"),
        (status = 422, description = "A version was published, but the flow cannot be compared with it, so every call would publish another one. Pin a flow version on the event instead."),
        (status = 423, description = "Another writer holds this board's mutation lease (code BOARD_LOCKED). Nothing was written; retry the identical request shortly.")
    )
)]
#[tracing::instrument(
    name = "POST /apps/{app_id}/board/{board_id}/version/current",
    skip(state, user)
)]
pub async fn publish_version_current(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, board_id)): Path<(String, String)>,
) -> Result<Json<BoardVersionPublished>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::WriteBoards);
    let sub = permission.sub()?;
    let board = state
        .master_board(&sub, &app_id, &board_id, &state, None)
        .await?;
    // An unedited flow needs no writer: a deploy must not wait for, or fail on, someone
    // who has the flow open.
    if let Some(version) = board.published_version_of_draft(None).await? {
        return Ok(Json(BoardVersionPublished {
            version,
            created: false,
        }));
    }
    let published = publish_under_lease(&state, &user, &sub, &app_id, &board_id).await?;
    Ok(Json(published))
}

/// Publishes the stored flow while holding the board's mutation lease, with everything that
/// follows a publication.
async fn publish_under_lease(
    state: &AppState,
    user: &AppUser,
    sub: &str,
    app_id: &str,
    board_id: &str,
) -> Result<BoardVersionPublished, ApiError> {
    let mutation_guard = state.board_mutation_guard(app_id, board_id).await?;
    let mut board = state
        .master_board(sub, app_id, board_id, state, None)
        .await?;

    mutation_guard.ensure_held()?;
    let (version, created) = board.publish_if_changed(None).await?;
    if created {
        super::version_board::after_publication(state, user, sub, app_id, &board, version).await;
        ensure_comparable(&board, version).await?;
    }
    Ok(BoardVersionPublished { version, created })
}

/// A flow whose fresh version does not compare equal to its stored draft would get one more
/// version with every deploy and still be left out of the copy a device receives.
async fn ensure_comparable(board: &Board, version: (u32, u32, u32)) -> Result<(), ApiError> {
    if board.published_version_of_draft(None).await? == Some(version) {
        return Ok(());
    }
    Err(ApiError::unprocessable(DRAFT_NOT_COMPARABLE))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;
    use flow_like::{
        state::{FlowLikeConfig, FlowLikeState},
        utils::http::HTTPClient,
    };
    use flow_like_storage::{
        Path as StorePath, files::store::FlowLikeStore, object_store::memory::InMemory,
    };
    use std::sync::Arc;

    async fn stored_flow() -> Board {
        let state = Arc::new(FlowLikeState::new(
            FlowLikeConfig::with_default_store(FlowLikeStore::Memory(Arc::new(InMemory::new()))),
            HTTPClient::new_without_refetch(),
        ));
        let board = Board::new(Some("flow".into()), StorePath::from("apps/project"), state);
        board.save(None).await.unwrap();
        board
    }

    #[tokio::test]
    async fn a_second_publication_of_an_unedited_flow_creates_nothing() {
        let mut board = stored_flow().await;
        assert_eq!(
            board.version_current(None).await.unwrap(),
            BoardVersionCurrent::default()
        );
        let (version, created) = board.publish_if_changed(None).await.unwrap();
        assert!(created);
        ensure_comparable(&board, version).await.unwrap();
        assert_eq!(
            board.publish_if_changed(None).await.unwrap(),
            (version, false)
        );
        assert_eq!(
            serde_json::to_value(board.version_current(None).await.unwrap()).unwrap(),
            serde_json::json!({"current": [0, 0, 1], "newest": [0, 0, 1]})
        );
        assert_eq!(
            serde_json::to_value(BoardVersionPublished {
                version,
                created: true
            })
            .unwrap(),
            serde_json::json!({"version": [0, 0, 1], "created": true})
        );
    }

    #[tokio::test]
    async fn a_flow_that_never_equals_its_fresh_version_is_refused() {
        let mut board = stored_flow().await;
        let (version, _) = board.publish_if_changed(None).await.unwrap();
        let mut unsaved = board.clone();
        unsaved.name = "Never stored".into();
        unsaved.mark_changed();
        let orphan = (version.0, version.1, version.2 + 5);
        assert!(unsaved.snapshot_at_version(orphan, None).await.is_err());
        assert_eq!(
            board.version_current(None).await.unwrap(),
            BoardVersionCurrent {
                current: None,
                newest: Some(orphan)
            }
        );

        let error = ensure_comparable(&board, version).await.unwrap_err();
        assert_eq!(error.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(error.public_message(), Some(DRAFT_NOT_COMPARABLE));
    }

    /// The body of the function declared with `declaration`.
    fn body<'a>(source: &'a str, declaration: &str) -> &'a str {
        let body = source
            .split_once(&format!("\n{declaration}("))
            .unwrap_or_else(|| panic!("{declaration} exists"))
            .1;
        body.split_once("\n}\n").map_or(body, |(body, _)| body)
    }

    fn position(body: &str, needle: &str) -> usize {
        body.find(needle)
            .unwrap_or_else(|| panic!("the function calls {needle}"))
    }

    /// Reading needs `ReadBoards`, publishing `WriteBoards`; each is checked before the flow
    /// is opened, and a publication holds the board's mutation lease.
    #[test]
    fn each_handler_checks_its_permission_before_it_opens_the_flow() {
        let source = include_str!("version_current.rs");
        let read = body(source, "pub async fn get_version_current");
        assert!(
            position(
                read,
                "ensure_permission!(user, &app_id, &state, RolePermissions::ReadBoards)"
            ) < position(read, ".master_board(")
        );
        assert!(!read.contains("publish_") && !read.contains("WriteBoards"));

        let publish = body(source, "pub async fn publish_version_current");
        let permission = position(
            publish,
            "ensure_permission!(user, &app_id, &state, RolePermissions::WriteBoards)",
        );
        assert!(permission < position(publish, ".master_board("));
        assert!(permission < position(publish, "publish_under_lease("));
        assert!(!publish.contains("ReadBoards"));

        let write = body(source, "async fn publish_under_lease");
        let lease = position(write, "state.board_mutation_guard(app_id, board_id)");
        let published = position(write, ".publish_if_changed(None)");
        assert!(lease < published);
        assert!(
            position(write, "super::version_board::after_publication(") > published,
            "the follow-ups of a publication run only after one"
        );
    }

    #[test]
    fn both_operations_are_documented() {
        let spec = serde_json::to_value(<crate::openapi::ApiDoc as utoipa::OpenApi>::openapi())
            .expect("spec serializes");
        let path = &spec["paths"]["/apps/{app_id}/board/{board_id}/version/current"];
        for (method, status) in [("get", "200"), ("post", "423")] {
            let operation = &path[method];
            assert!(operation.is_object(), "no OpenAPI operation {method}");
            assert_eq!(operation["tags"], serde_json::json!(["boards"]));
            assert!(
                operation["description"]
                    .as_str()
                    .is_some_and(|text| !text.is_empty())
            );
            assert!(
                operation["responses"][status].is_object(),
                "{method} {status}"
            );
        }
        for schema in ["BoardVersionCurrent", "BoardVersionPublished"] {
            assert!(
                spec["components"]["schemas"][schema].is_object(),
                "schema {schema} is registered"
            );
        }
    }
}

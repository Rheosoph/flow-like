use axum::{Router, routing::get};

use crate::state::{AppState, BoardMutationGuard};

// Page ids are database-primary-key global, while the canonical files live under boards. Reuse
// the replica-safe mutation primitive with an impossible real app-id scope so upsert/delete can
// serialize one page id before either discovers its owning board.
const PAGE_ID_MUTATION_SCOPE: &str = "\0flow-like.page-id-mutation/v1";

pub(crate) async fn page_id_mutation_guard(
    state: &AppState,
    page_id: &str,
) -> Result<BoardMutationGuard, sea_orm::DbErr> {
    state
        .board_mutation_guard(PAGE_ID_MUTATION_SCOPE, page_id)
        .await
}

pub mod delete_page;
pub mod get_page;
pub mod get_page_by_route;
pub mod get_pages;
pub mod upsert_page;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(get_pages::get_pages))
        .route("/by-route", get(get_page_by_route::get_page_by_route))
        .route(
            "/{page_id}",
            get(get_page::get_page)
                .put(upsert_page::upsert_page)
                .delete(delete_page::delete_page),
        )
}

#[cfg(test)]
mod tests {
    use super::PAGE_ID_MUTATION_SCOPE;
    use crate::state::{board_mutation_lock_id, board_mutation_lock_key};

    #[test]
    fn page_id_mutation_scope_is_stable_and_separate_from_real_boards() {
        assert_eq!(
            board_mutation_lock_key(PAGE_ID_MUTATION_SCOPE, " page "),
            board_mutation_lock_key(PAGE_ID_MUTATION_SCOPE, "page")
        );
        assert_ne!(
            board_mutation_lock_key(PAGE_ID_MUTATION_SCOPE, "page"),
            board_mutation_lock_key("app", "page")
        );
        assert_ne!(
            board_mutation_lock_key(PAGE_ID_MUTATION_SCOPE, "page"),
            board_mutation_lock_key(PAGE_ID_MUTATION_SCOPE, "other")
        );
        assert_ne!(
            board_mutation_lock_id(PAGE_ID_MUTATION_SCOPE, "page"),
            board_mutation_lock_id("app", "page")
        );
        assert_ne!(
            board_mutation_lock_id(PAGE_ID_MUTATION_SCOPE, "page"),
            board_mutation_lock_id(PAGE_ID_MUTATION_SCOPE, "other")
        );
    }
}

use axum::{Router, routing::get};

use crate::state::AppState;

pub mod delete_template;
pub mod get_template;
pub mod get_template_preview;
pub mod get_templates;
pub mod upsert_template;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(get_templates::get_templates))
        .route(
            "/{template_id}",
            get(get_template::get_template)
                .put(upsert_template::upsert_template)
                .delete(delete_template::delete_template),
        )
        .route(
            "/{template_id}/preview",
            get(get_template_preview::get_template_preview),
        )
}

//! External databases an app's queries read as `<alias>.<table>`. The settings are stored as
//! plain JSON and the credentials sealed (`crate::data_source_secrets`); responses name the
//! stored secret fields, never their values.

use crate::state::AppState;
use axum::{Router, routing::get};

pub mod handlers;
pub mod model;
pub mod query_source;
#[cfg(test)]
mod tests;
pub mod validation;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/",
            get(handlers::list_data_sources).post(handlers::create_data_source),
        )
        .route(
            "/{source_id}",
            get(handlers::get_data_source)
                .patch(handlers::update_data_source)
                .delete(handlers::delete_data_source),
        )
}

use axum::{
    Router,
    routing::{delete, get, patch, post, put},
};
use bit::{delete_bit, push_meta, upsert_bit};
use models::{sync_models, upsert_model};

use crate::state::AppState;

pub mod bit;
pub mod models;
pub mod packages;
pub mod profiles;
pub mod publication;
pub mod runs;
pub mod sinks;
pub mod solutions;
pub mod users;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/bit/{bit_id}",
            put(upsert_bit::upsert_bit).delete(delete_bit::delete_bit),
        )
        .route("/bit/{bit_id}/{language}", put(push_meta::push_meta))
        .route("/models/sync", post(sync_models::sync_models))
        .route("/models/{slug}", put(upsert_model::upsert_model))
        .route(
            "/profiles/media",
            get(profiles::get_signed_profile_img_url::get_signed_profile_img_url),
        )
        .route(
            "/profiles/{profile_id}",
            put(profiles::upsert_profile_template::upsert_profile_template)
                .delete(profiles::delete_profile_template::delete_profile_template),
        )
        .route("/solutions", get(solutions::list_solutions::list_solutions))
        .route(
            "/solutions/{solution_id}",
            get(solutions::get_solution::get_solution)
                .patch(solutions::update_solution::update_solution),
        )
        .route(
            "/solutions/{solution_id}/logs",
            post(solutions::add_log::add_solution_log),
        )
        // Publication review routes
        .route(
            "/publication/requests",
            get(publication::get_requests::get_requests),
        )
        .route(
            "/publication/requests/{request_id}",
            patch(publication::upsert_requests::upsert_request),
        )
        .route(
            "/publication/apps/{app_id}/content",
            get(publication::get_app_content::get_app_content),
        )
        .route(
            "/publication/apps/{app_id}/board/{board_id}",
            get(publication::get_board::get_board),
        )
        // Package management routes
        .route("/packages", get(packages::get_packages::get_packages))
        .route("/packages/stats", get(packages::get_stats::get_stats))
        .route(
            "/packages/{package_id}",
            get(packages::get_package::get_package)
                .patch(packages::update_package::update_package)
                .delete(packages::delete_package::delete_package),
        )
        .route(
            "/packages/{package_id}/review",
            post(packages::review_package::review_package),
        )
        // Sink token management routes
        .route(
            "/sinks",
            get(sinks::list_tokens::list_tokens).post(sinks::register_sink::register_sink),
        )
        .route("/sinks/{jti}", delete(sinks::revoke_sink::revoke_sink))
        // User management routes
        .route("/users", get(users::list_users::list_users))
        .route("/users/{user_id}", patch(users::update_user::update_user))
        // Run reconciliation
        .route("/runs/sweep", post(runs::sweep_runs))
}

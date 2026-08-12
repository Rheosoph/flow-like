use axum::{
    Router,
    routing::{delete, get, post, put},
};

use crate::state::AppState;

pub mod create_invite_link;
pub mod get_invite_links;
pub mod get_join_requests;
pub mod get_team;
pub mod invite_user;
pub mod invites;
pub mod join_invite_link;
pub mod manage_join_request;
pub mod purchase;
pub mod remove_invite_link;
pub mod remove_user;
pub mod request_join;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(get_team::get_team))
        .route(
            "/link",
            put(create_invite_link::create_invite_link).get(get_invite_links::get_invite_links),
        )
        .route(
            "/link/{link_id}",
            delete(remove_invite_link::remove_invite_link),
        )
        .route(
            "/link/join/{token}",
            post(join_invite_link::join_invite_link),
        )
        .route(
            "/queue",
            get(get_join_requests::get_join_requests).put(request_join::request_join),
        )
        .route(
            "/queue/{request_id}",
            post(manage_join_request::accept_join_request)
                .delete(manage_join_request::reject_join_request),
        )
        .route("/invite", put(invite_user::invite_user))
        .route("/invites", get(invites::list_app_invites))
        .route("/invites/{invite_id}", delete(invites::revoke_invite))
        .route("/purchase", post(purchase::purchase))
        .route("/{sub}", delete(remove_user::remove_user))
}

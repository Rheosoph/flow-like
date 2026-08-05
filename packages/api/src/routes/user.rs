use crate::{entity::user, state::AppState};
use axum::{
    Router,
    routing::{get, post},
};
use billing::get_billing_session;
use flow_like_types::create_id;
use info::user_info;
use pricing::get_pricing;
use sea_orm::{EntityTrait, sea_query::OnConflict};
use subscribe::create_subscription_checkout;

/// Ensures a user row exists in the DB for the given `sub`.
/// Uses INSERT ... ON CONFLICT DO NOTHING so it's safe to call concurrently.
pub async fn ensure_user_exists(state: &AppState, sub: &str) -> Result<(), crate::error::ApiError> {
    let existing = user::Entity::find_by_id(sub).one(&state.db).await?;
    if existing.is_some() {
        return Ok(());
    }

    let user = user::ActiveModel {
        id: sea_orm::ActiveValue::Set(sub.to_string()),
        tracking_id: sea_orm::ActiveValue::Set(Some(create_id())),
        created_at: sea_orm::ActiveValue::Set(chrono::Utc::now().naive_utc()),
        updated_at: sea_orm::ActiveValue::Set(chrono::Utc::now().naive_utc()),
        ..Default::default()
    };

    let res = user::Entity::insert(user)
        .on_conflict(OnConflict::column(user::Column::Id).do_nothing().to_owned())
        .do_nothing()
        .exec(&state.db)
        .await;

    match res {
        Ok(_) => Ok(()),
        Err(sea_orm::DbErr::RecordNotInserted) => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Uploads are signed for the file's real extension and the media-transformer
/// rewrites them to `.webp`, so the canonical stored object is always `{id}.webp`.
/// Mirrors `routes::profile::sign_profile_image`, tolerating rows that kept an
/// extension.
pub fn avatar_file_name(avatar_id: &str) -> String {
    match avatar_id.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => format!("{}.webp", stem),
        _ => format!("{}.webp", avatar_id),
    }
}

pub async fn sign_avatar(
    sub: &str,
    avatar_id: &str,
    state: &AppState,
) -> flow_like_types::Result<String> {
    let master_store = state.master_credentials().await?;
    let master_store = master_store.to_store(false).await?;
    let path = flow_like_storage::Path::from("media")
        .child("users")
        .child(sub)
        .child(avatar_file_name(avatar_id));
    let url = master_store
        .sign("GET", &path, std::time::Duration::from_secs(60 * 5))
        .await?;
    Ok(url.to_string())
}

pub mod billing;
pub mod bits;
pub mod bootstrap;
pub mod get_invites;
pub mod groups;
pub mod identity;
pub mod info;
pub mod lookup;
pub mod manage_invite;
pub mod notifications;
pub mod pat;
pub mod pricing;
pub mod push_targets;
pub mod subscribe;
pub mod templates;
pub mod upsert_info;
pub mod widgets;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/bootstrap", get(bootstrap::bootstrap))
        .route("/bits", get(bits::list_user_bits))
        .route(
            "/bits/{bit_id}",
            axum::routing::put(bits::upsert_user_bit).delete(bits::delete_user_bit),
        )
        .route(
            "/pat",
            get(pat::get_pats::get_pats).put(pat::create_pat::create_pat),
        )
        .route(
            "/pat/{pat_id}",
            axum::routing::delete(pat::delete_pat::delete_pat),
        )
        .route("/info", get(user_info).put(upsert_info::upsert_info))
        .route("/billing", get(get_billing_session))
        .route("/pricing", get(get_pricing))
        .route("/subscribe", post(create_subscription_checkout))
        .route("/lookup", post(lookup::user_batch_lookup))
        .route("/lookup/{sub}", get(lookup::user_lookup))
        .route("/search/{query}", get(lookup::user_search))
        .route("/invites", get(get_invites::get_invites))
        .route("/templates", get(templates::get_templates))
        .route("/groups", get(groups::get_user_groups))
        .route("/widgets", get(widgets::get_widgets))
        .route("/notifications", get(notifications::get_notifications))
        .route(
            "/push-targets/register",
            post(push_targets::register_push_target),
        )
        .route(
            "/push-targets/{device_id}",
            get(push_targets::get_push_target_status)
                .patch(push_targets::update_push_target_status)
                .delete(push_targets::unregister_push_target),
        )
        .route(
            "/notifications/create",
            post(notifications::create_user_notification),
        )
        .route(
            "/notifications/list",
            get(notifications::list_notifications),
        )
        .route(
            "/notifications/read-all",
            post(notifications::mark_all_read),
        )
        .route(
            "/notifications/{notification_id}",
            post(notifications::mark_notification_read).delete(notifications::delete_notification),
        )
        .route(
            "/invites/{invite_id}",
            post(manage_invite::accept_invite).delete(manage_invite::reject_invite),
        )
}

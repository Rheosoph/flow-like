//! A group's picture: any active member sets or removes it. It is stored as a PNG of at most
//! 256 px under `conversations/<id>/image/`, named by its content hash, so its link changes
//! whenever the picture does. A swap is a compare-and-set on the stored hash, so of two members
//! changing it at once one wins and the other is told to try again; the winner's change lands in
//! every member's history as a `room` entry.

use super::{
    Reader,
    conversations::{member_group, record_room_change},
    files::content_store,
    lock_conversation,
    model::{Conversation, SetConversationImageRequest},
    now_millis,
    view::visible_conversation,
    wake_members,
};
use crate::{entity::conversation, error::ApiError, middleware::jwt::AppUser, state::AppState};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use chrono::{DateTime, FixedOffset};
use flow_like_storage::{
    files::store::FlowLikeStore,
    object_store::{self, ObjectStoreExt, path::Path as ObjectPath},
};
use flow_like_types::{images::inline_image_thumbnail, inbox::RoomEventBody};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseTransaction, EntityTrait, QueryFilter, sea_query::Expr,
};
use std::time::Duration;

const IMAGE_URL_LIFETIME: Duration = Duration::from_secs(24 * 60 * 60);

fn image_path(conversation_id: &str, image: &str) -> ObjectPath {
    ObjectPath::from(super::files::object_key(
        conversation_id,
        &format!("image/{image}.png"),
    ))
}

/// Fills in the links to the pictures of the conversations that have one. A store that can't
/// be reached leaves them without, so the members show instead.
pub(super) async fn sign_images(state: &AppState, conversations: &mut [Conversation]) {
    if conversations
        .iter()
        .all(|conversation| conversation.image.is_none())
    {
        return;
    }
    let store = match content_store(state).await {
        Ok(store) => store,
        Err(error) => {
            tracing::warn!(
                ?error,
                "Could not reach the store holding conversation pictures"
            );
            return;
        }
    };
    for conversation in conversations.iter_mut() {
        let Some(image) = conversation.image.as_deref() else {
            continue;
        };
        match store
            .sign_cached(
                "GET",
                &image_path(&conversation.id, image),
                IMAGE_URL_LIFETIME,
            )
            .await
        {
            Ok(url) => conversation.image_url = Some(url.to_string()),
            Err(error) => {
                tracing::warn!(%error, conversation_id = %conversation.id, "Could not sign a conversation picture");
            }
        }
    }
}

/// The link to one group's picture, for pages outside the inbox such as an invite link's preview.
pub(crate) async fn image_url(
    state: &AppState,
    conversation_id: &str,
    image: &str,
) -> Option<String> {
    let store = content_store(state).await.ok()?;
    store
        .sign_cached(
            "GET",
            &image_path(conversation_id, image),
            IMAGE_URL_LIFETIME,
        )
        .await
        .ok()
        .map(|url| url.to_string())
}

async fn delete_image(store: &FlowLikeStore, conversation_id: &str, image: &str) {
    match store
        .as_generic()
        .delete(&image_path(conversation_id, image))
        .await
    {
        Ok(()) | Err(object_store::Error::NotFound { .. }) => {}
        Err(error) => {
            tracing::warn!(%error, %conversation_id, %image, "Could not delete a replaced conversation picture");
        }
    }
}

/// Points the group at `next` if its picture is still `previous`; true when it did.
pub(super) async fn compare_and_set<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
    previous: Option<&str>,
    next: Option<&str>,
    now: DateTime<FixedOffset>,
) -> Result<bool, ApiError> {
    let expected = match previous {
        Some(image) => conversation::Column::Image.eq(image),
        None => conversation::Column::Image.is_null(),
    };
    let updated = conversation::Entity::update_many()
        .col_expr(
            conversation::Column::Image,
            Expr::value(next.map(str::to_string)),
        )
        .col_expr(conversation::Column::UpdatedAt, Expr::value(now))
        .filter(conversation::Column::Id.eq(conversation_id))
        .filter(expected)
        .exec(db)
        .await?;
    Ok(updated.rows_affected == 1)
}

/// Swaps the group's picture from what the caller read to `next`, whose object is already
/// stored, and removes the object it replaced. Losing a race removes `next` again unless the
/// winner set the same picture.
async fn swap_image(
    state: &AppState,
    store: &FlowLikeStore,
    conversation_id: &str,
    by: &str,
    previous: Option<String>,
    next: Option<String>,
) -> Result<(), ApiError> {
    if previous == next {
        return Ok(());
    }
    let swapped = state
        .transaction(|txn| {
            let (id, by, previous, next) = (
                conversation_id.to_string(),
                by.to_string(),
                previous.clone(),
                next.clone(),
            );
            Box::pin(async move { swap_in(txn, &id, &by, previous, next, now_millis()).await })
        })
        .await?;
    if !swapped {
        let latest = conversation::Entity::find_by_id(conversation_id)
            .one(&state.db)
            .await?
            .and_then(|row| row.image);
        if let Some(image) = next
            .as_deref()
            .filter(|image| latest.as_deref() != Some(image))
        {
            delete_image(store, conversation_id, image).await;
        }
        return Err(ApiError::conflict(format!(
            "The picture of conversation '{conversation_id}' changed meanwhile; try again"
        )));
    }
    if let Some(image) = previous {
        delete_image(store, conversation_id, &image).await;
    }
    wake_members(state, conversation_id).await;
    Ok(())
}

/// Points the group at `next` if its picture is still `previous` and records the change in every
/// member's history; false when someone changed it first.
pub(super) async fn swap_in(
    txn: &DatabaseTransaction,
    conversation_id: &str,
    by: &str,
    previous: Option<String>,
    next: Option<String>,
    now: DateTime<FixedOffset>,
) -> Result<bool, ApiError> {
    lock_conversation(txn, conversation_id).await?;
    member_group(txn, conversation_id, by).await?;
    if !compare_and_set(
        txn,
        conversation_id,
        previous.as_deref(),
        next.as_deref(),
        now,
    )
    .await?
    {
        return Ok(false);
    }
    record_room_change(
        txn,
        conversation_id,
        &RoomEventBody::image(next.is_none(), by),
        now,
    )
    .await?;
    Ok(true)
}

async fn thumbnail(conversation_id: &str, image: String) -> Result<Vec<u8>, ApiError> {
    flow_like_types::tokio::task::spawn_blocking(move || inline_image_thumbnail(&image))
        .await
        .map_err(|error| ApiError::internal_error(error.into()))?
        .map_err(|error| {
            ApiError::bad_request(format!(
                "The picture for conversation '{conversation_id}' is unusable: {error}"
            ))
        })
}

#[utoipa::path(
    put,
    path = "/inbox/conversations/{conversation_id}/image",
    tag = "Inbox",
    description = "Set the picture of a group you are in. Every member sees it in place of the members' avatars.",
    params(("conversation_id" = String, Path, description = "The group")),
    request_body = SetConversationImageRequest,
    responses(
        (status = 200, description = "The group with its new picture", body = Conversation),
        (status = 400, description = "The image is not a PNG, JPEG, GIF or WebP data URL of at most 512 KiB"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "Only people change pictures, not API keys or running flows"),
        (status = 404, description = "You are not in this conversation"),
        (status = 409, description = "INBOX_DIRECT_FIXED: a direct conversation has no picture; or someone else changed the picture meanwhile")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "PUT /inbox/conversations/{conversation_id}/image",
    skip(state, user, body)
)]
pub async fn set_conversation_image(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(conversation_id): Path<String>,
    Json(body): Json<SetConversationImageRequest>,
) -> Result<Json<Conversation>, ApiError> {
    let sub = user.sub()?;
    let current = member_group(&state.db, &conversation_id, &sub).await?;
    let png = thumbnail(&conversation_id, body.image).await?;
    let image = blake3::hash(&png).to_hex().to_string();
    let store = content_store(&state).await?;
    store
        .as_generic()
        .put(&image_path(&conversation_id, &image), png.into())
        .await?;
    swap_image(
        &state,
        &store,
        &conversation_id,
        &sub,
        current.image,
        Some(image),
    )
    .await?;
    Ok(Json(
        visible_conversation(&state, &Reader::own(&sub), &conversation_id).await?,
    ))
}

#[utoipa::path(
    delete,
    path = "/inbox/conversations/{conversation_id}/image",
    tag = "Inbox",
    description = "Remove the picture of a group you are in; its members' avatars show again.",
    params(("conversation_id" = String, Path, description = "The group")),
    responses(
        (status = 200, description = "The group without a picture", body = Conversation),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "Only people change pictures, not API keys or running flows"),
        (status = 404, description = "You are not in this conversation"),
        (status = 409, description = "INBOX_DIRECT_FIXED: a direct conversation has no picture; or someone else changed the picture meanwhile")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "DELETE /inbox/conversations/{conversation_id}/image",
    skip(state, user)
)]
pub async fn remove_conversation_image(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(conversation_id): Path<String>,
) -> Result<Json<Conversation>, ApiError> {
    let sub = user.sub()?;
    let current = member_group(&state.db, &conversation_id, &sub).await?;
    if current.image.is_some() {
        let store = content_store(&state).await?;
        swap_image(&state, &store, &conversation_id, &sub, current.image, None).await?;
    }
    Ok(Json(
        visible_conversation(&state, &Reader::own(&sub), &conversation_id).await?,
    ))
}

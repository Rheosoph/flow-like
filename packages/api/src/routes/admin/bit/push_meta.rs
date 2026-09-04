use crate::{
    entity::meta, error::ApiError, middleware::jwt::AppUser,
    permission::global_permission::GlobalPermission, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like::bit::Metadata;
use flow_like_types::create_id;
use sea_orm::{ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter};
use serde_json::from_slice;

#[tracing::instrument(name = "PUT /admin/bit/{bit_id}/{language}", skip(state, user, meta))]
pub async fn push_meta(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((bit_id, language)): Path<(String, String)>,
    Json(meta): Json<Metadata>,
) -> Result<Json<()>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::WriteBits)
        .await?;

    let existing_meta = meta::Entity::find()
        .filter(meta::Column::BitId.eq(&bit_id))
        .filter(meta::Column::Lang.eq(&language))
        .one(&state.db)
        .await?;

    if let Some(existing_meta) = &existing_meta {
        let mut updated_meta: meta::ActiveModel = existing_meta.clone().into();
        updated_meta.description = Set(Some(meta.description));
        updated_meta.name = Set(meta.name);
        updated_meta.long_description = Set(meta
            .long_description
            .or(existing_meta.long_description.clone()));
        updated_meta.docs_url = Set(meta.docs_url.or(existing_meta.docs_url.clone()));
        updated_meta.age_rating = Set(meta
            .age_rating
            .or(existing_meta.age_rating.map(|r| r as i32))
            .map(|r| r as i64));
        updated_meta.icon = Set(meta.icon.or(existing_meta.icon.clone()));
        updated_meta.organization_specific_values = Set(meta
            .organization_specific_values
            .as_ref()
            .and_then(|v| from_slice(v).ok())
            .or(existing_meta.organization_specific_values.clone()));
        updated_meta.release_notes =
            Set(meta.release_notes.or(existing_meta.release_notes.clone()));
        updated_meta.support_url = Set(meta.support_url.or(existing_meta.support_url.clone()));
        updated_meta.tags = Set(Some(meta.tags.into()));
        updated_meta.thumbnail = Set(meta.thumbnail.or(existing_meta.thumbnail.clone()));
        updated_meta.use_case = Set(meta.use_case.or(existing_meta.use_case.clone()));
        updated_meta.website = Set(meta.website.or(existing_meta.website.clone()));
        updated_meta.updated_at = Set(chrono::Utc::now().fixed_offset());
        updated_meta.update(&state.db).await?;

        return Ok(Json(()));
    }

    let new_meta = meta::ActiveModel {
        id: Set(create_id()),
        bit_id: Set(Some(bit_id.clone())),
        lang: Set(language),
        description: Set(Some(meta.description)),
        name: Set(meta.name),
        long_description: Set(meta.long_description),
        docs_url: Set(meta.docs_url),
        age_rating: Set(meta.age_rating.map(|r| r as i64)),
        icon: Set(meta.icon),
        organization_specific_values: Set(meta
            .organization_specific_values
            .as_ref()
            .and_then(|v| from_slice(v).ok())),
        release_notes: Set(meta.release_notes),
        support_url: Set(meta.support_url),
        tags: Set(Some(meta.tags.into())),
        thumbnail: Set(meta.thumbnail),
        use_case: Set(meta.use_case),
        website: Set(meta.website),
        created_at: Set(chrono::Utc::now().fixed_offset()),
        updated_at: Set(chrono::Utc::now().fixed_offset()),
        app_id: Set(None),
        course_id: Set(None),
        preview_media: Set(None),
        template_id: Set(None),
        widget_id: Set(None),
        wasm_package_id: Set(None),
        group_id: Set(None),
    };

    new_meta.insert(&state.db).await?;

    Ok(Json(()))
}

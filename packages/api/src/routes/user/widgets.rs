use crate::{
    entity::{membership, meta, role, widget},
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::{RolePermissions, has_role_permission},
    routes::LanguageParams,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Query, State},
};
use flow_like::bit::Metadata;
use sea_orm::{
    ColumnTrait, DatabaseTransaction, EntityTrait, JoinType, QueryFilter, QueryOrder, QuerySelect,
    RelationTrait, TransactionTrait,
};

/// Get all widgets accessible to the current user based on their permissions.
/// Returns widgets from all apps where the user has ReadWidgets permission.
#[utoipa::path(
    get,
    path = "/user/widgets",
    tag = "user",
    params(
        ("language" = Option<String>, Query, description = "Language code"),
        ("limit" = Option<u64>, Query, description = "Maximum number of results"),
        ("offset" = Option<u64>, Query, description = "Offset for pagination")
    ),
    responses(
        (status = 200, description = "List of accessible widgets with metadata"),
        (status = 401, description = "Unauthorized")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[tracing::instrument(name = "GET /user/widgets", skip_all)]
pub async fn get_widgets(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(query): Query<LanguageParams>,
) -> Result<Json<Vec<(String, String, Metadata)>>, ApiError> {
    let language = query.language.as_deref().unwrap_or("en");
    let user_id = user.sub()?;

    let txn = state.db.begin().await?;
    let app_ids = get_user_app_ids_with_widget_access(&txn, user_id, &query).await?;
    let widgets = get_widgets_with_metadata(&txn, &app_ids, language, &state).await?;
    txn.commit().await?;

    Ok(Json(widgets))
}

/// Get app IDs where the user has widget read access
async fn get_user_app_ids_with_widget_access(
    txn: &DatabaseTransaction,
    user_id: String,
    query: &LanguageParams,
) -> Result<Vec<String>, ApiError> {
    let limit = query.limit.unwrap_or(100).min(100);

    let app_ids = membership::Entity::find()
        .select_only()
        .columns([role::Column::AppId, role::Column::Permissions])
        .join(JoinType::InnerJoin, membership::Relation::Role.def())
        .filter(membership::Column::UserId.eq(user_id))
        .order_by_desc(membership::Column::UpdatedAt)
        .limit(Some(limit))
        .offset(query.offset)
        .into_tuple::<(String, i64)>()
        .all(txn)
        .await?
        .into_iter()
        .filter_map(|(app_id, permissions)| {
            let permission = RolePermissions::from_bits(permissions)?;
            has_role_permission(&permission, RolePermissions::ReadWidgets).then_some(app_id)
        })
        .collect();

    Ok(app_ids)
}

/// Get widgets with their metadata for the specified app IDs
async fn get_widgets_with_metadata(
    txn: &DatabaseTransaction,
    app_ids: &[String],
    language: &str,
    state: &AppState,
) -> Result<Vec<(String, String, Metadata)>, ApiError> {
    if app_ids.is_empty() {
        return Ok(Vec::new());
    }

    let widgets = widget::Entity::find()
        .find_with_related(meta::Entity)
        .filter(widget::Column::AppId.is_in(app_ids))
        .filter(
            meta::Column::Lang
                .eq(language)
                .or(meta::Column::Lang.eq("en")),
        )
        .all(txn)
        .await?;

    let master_store = state.master_credentials().await?;
    let store = master_store.to_store(false).await?;

    let mut result = Vec::with_capacity(widgets.len());

    for (widget_model, metadata) in widgets {
        if let Some(meta) = find_best_metadata(&metadata, language) {
            let mut metadata = Metadata::from(meta.clone());
            let prefix = flow_like_storage::Path::from("media")
                .child("apps")
                .child(widget_model.app_id.clone());
            metadata.presign(prefix, &store).await;
            result.push((
                widget_model.app_id.clone(),
                widget_model.id.clone(),
                metadata,
            ));
        }
    }

    Ok(result)
}

/// Find the best matching metadata entry, preferring the requested language, then English, then any
fn find_best_metadata<'a>(
    metadata: &'a [meta::Model],
    language: &'a str,
) -> Option<&'a meta::Model> {
    metadata
        .iter()
        .find(|meta| meta.lang == language)
        .or_else(|| metadata.iter().find(|meta| meta.lang == "en"))
        .or_else(|| metadata.first())
}

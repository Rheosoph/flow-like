use crate::{
    entity::{app, membership, meta, sea_orm_active_enums::Status},
    error::ApiError,
    middleware::jwt::AppUser,
    routes::LanguageParams,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Query, State},
};
use flow_like::{app::App, bit::Metadata};
use sea_orm::sea_query::ExprTrait;
use sea_orm::{
    ColumnTrait, EntityTrait, JoinType, QueryFilter, QueryOrder, QuerySelect, RelationTrait, Select,
};

/// Apps paired with their localized metadata, when any exists.
pub type AppsWithMetadata = Vec<(App, Option<Metadata>)>;

/// The apps `sub` may open, newest first.
///
/// `INACTIVE` is written by exactly two paths — the deletion tombstone and the
/// destination of an in-flight fork — so an app that is draining and one that
/// is still being copied both stay out of the library instead of showing up
/// empty or unopenable.
fn library_apps(sub: &str) -> Select<app::Entity> {
    app::Entity::find()
        .order_by_desc(app::Column::UpdatedAt)
        .join(JoinType::InnerJoin, app::Relation::Membership.def())
        .filter(app::Column::Status.ne(Status::Inactive))
        .filter(membership::Column::UserId.eq(sub))
}

#[utoipa::path(
    get,
    path = "/apps",
    tag = "apps",
    params(
        ("language" = Option<String>, Query, description = "Language code (default: en)"),
        ("limit" = Option<u64>, Query, description = "Maximum number of results (max 100)"),
        ("offset" = Option<u64>, Query, description = "Offset for pagination")
    ),
    responses(
        (status = 200, description = "List of user applications with metadata", body = Vec<Object>),
        (status = 401, description = "Unauthorized")
    )
)]
#[tracing::instrument(name = "GET /apps", skip_all)]
pub async fn get_apps(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(query): Query<LanguageParams>,
) -> Result<Json<AppsWithMetadata>, ApiError> {
    let language = query.language.clone().unwrap_or_else(|| "en".to_string());

    let limit = std::cmp::Ord::min(query.limit.unwrap_or(100), 100);

    let sub = user.sub()?;

    let apps_with_meta = library_apps(&sub)
        .find_with_related(meta::Entity)
        .filter(
            meta::Column::Lang
                .eq(&language)
                .or(meta::Column::Lang.eq("en")),
        )
        .limit(Some(std::cmp::Ord::min(limit, 100)))
        .offset(query.offset)
        .all(&state.db)
        .await?;

    let master_store = state.master_credentials().await?;
    let store = master_store.to_store(false).await?;

    let mut apps = Vec::new();

    for (app_model, meta_models) in apps_with_meta {
        let metadata = if let Some(meta) = meta_models
            .iter()
            .find(|meta| meta.lang == language)
            .or_else(|| meta_models.first())
        {
            let mut metadata = Metadata::from(meta.clone());
            let prefix = flow_like_storage::Path::from("media")
                .child("apps")
                .child(app_model.id.clone());
            metadata.presign(prefix, &store).await;
            Some(metadata)
        } else {
            None
        };

        apps.push((App::from(app_model), metadata));
    }

    Ok(Json(apps))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::QueryTrait;
    use sea_orm::sea_query::PostgresQueryBuilder;

    #[test]
    fn library_hides_tombstoned_and_half_forked_apps() {
        let sql = library_apps("user_1")
            .into_query()
            .to_string(PostgresQueryBuilder);

        assert!(sql.contains(r#""App"."status" <> 'INACTIVE'"#), "{sql}");
        assert!(sql.contains(r#""Membership"."userId" = 'user_1'"#), "{sql}");
    }
}

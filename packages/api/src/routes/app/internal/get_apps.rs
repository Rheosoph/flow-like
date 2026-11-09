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
use sea_orm::{
    ColumnTrait, ConnectionTrait, DbErr, EntityTrait, JoinType, QueryFilter, QueryOrder,
    QuerySelect, RelationTrait, Select,
};
use std::collections::HashMap;

/// Apps paired with their localized metadata, when any exists.
pub type AppsWithMetadata = Vec<(App, Option<Metadata>)>;

/// The apps `sub` may open, newest first.
///
/// `INACTIVE` is written by exactly two paths — the deletion tombstone and the
/// destination of an in-flight fork — so an app that is draining and one that
/// is still being copied both stay out of the library instead of showing up
/// empty or unopenable. A bot's workspace belongs to its bot, not the library.
pub(crate) fn library_apps(sub: &str) -> Select<app::Entity> {
    app::Entity::find()
        .order_by_desc(app::Column::UpdatedAt)
        .order_by_asc(app::Column::Id)
        .join(JoinType::InnerJoin, app::Relation::Membership.def())
        .filter(app::Column::Status.ne(Status::Inactive))
        .filter(app::Column::BotId.is_null())
        .filter(membership::Column::UserId.eq(sub))
}

async fn library_page<C: ConnectionTrait>(
    db: &C,
    sub: &str,
    language: &str,
    limit: u64,
    offset: Option<u64>,
) -> Result<Vec<(app::Model, Option<meta::Model>)>, DbErr> {
    // Page apps before reading translations, so missing metadata cannot hide an app and
    // multiple translations cannot consume its neighbors' page slots.
    let apps = library_apps(sub)
        .limit(Some(limit.min(100)))
        .offset(offset)
        .all(db)
        .await?;
    if apps.is_empty() {
        return Ok(Vec::new());
    }
    let metas = meta::Entity::find()
        .filter(meta::Column::AppId.is_in(apps.iter().map(|app| app.id.clone())))
        .order_by_asc(meta::Column::Lang)
        .order_by_asc(meta::Column::Id)
        .all(db)
        .await?;
    let preference = |meta: &meta::Model| match meta.lang.as_str() {
        lang if lang == language => 0,
        "en" => 1,
        _ => 2,
    };
    let mut preferred = HashMap::<String, meta::Model>::new();
    for meta in metas {
        let Some(app_id) = meta.app_id.clone() else {
            continue;
        };
        if preferred
            .get(&app_id)
            .is_none_or(|current| preference(&meta) < preference(current))
        {
            preferred.insert(app_id, meta);
        }
    }
    Ok(apps
        .into_iter()
        .map(|app| {
            let metadata = preferred.remove(&app.id);
            (app, metadata)
        })
        .collect())
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

    let apps_with_meta = library_page(&state.db, &sub, &language, limit, query.offset).await?;

    let master_store = state.master_credentials().await?;
    let store = master_store.to_store(false).await?;

    let mut apps = Vec::new();

    for (app_model, meta_model) in apps_with_meta {
        let metadata = if let Some(meta) = meta_model {
            let mut metadata = Metadata::from(meta);
            let prefix = flow_like_storage::Path::from("media")
                .join("apps")
                .join(app_model.id.clone());
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
    use crate::entity::sea_orm_active_enums::{ExecutionMode, Visibility};
    use sea_orm::{
        ActiveModelTrait,
        ActiveValue::Set,
        ConnectOptions, Database, DatabaseConnection, QueryTrait, Schema,
        sea_query::{PostgresQueryBuilder, Table},
    };

    async fn database() -> DatabaseConnection {
        let mut options = ConnectOptions::new("sqlite::memory:");
        options.max_connections(1);
        let db = Database::connect(options).await.unwrap();
        db.execute_unprepared("ATTACH DATABASE ':memory:' AS public")
            .await
            .unwrap();
        let schema = Schema::new(db.get_database_backend());
        for source in [
            schema.create_table_from_entity(app::Entity),
            schema.create_table_from_entity(meta::Entity),
            schema.create_table_from_entity(membership::Entity),
        ] {
            let mut table = Table::create();
            table.table(source.get_table_name().unwrap().clone());
            for column in source.get_columns() {
                table.col(column.clone());
            }
            db.execute(&table).await.unwrap();
        }
        db
    }

    async fn insert_app(db: &DatabaseConnection, id: &str, owner: &str, languages: &[&str]) {
        let now = chrono::DateTime::from_timestamp(1_800_000_000, 0)
            .unwrap()
            .fixed_offset();
        app::ActiveModel {
            id: Set(id.into()),
            status: Set(Status::Active),
            visibility: Set(Visibility::Private),
            execution_mode: Set(ExecutionMode::Any),
            rating_sum: Set(0),
            rating_count: Set(0),
            download_count: Set(0),
            interactions_count: Set(0),
            total_size: Set(0),
            price: Set(0),
            allow_forking: Set(false),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        }
        .insert(db)
        .await
        .unwrap();
        membership::ActiveModel {
            id: Set(format!("member-{id}")),
            app_id: Set(id.into()),
            user_id: Set(owner.into()),
            role_id: Set("owner-role".into()),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        }
        .insert(db)
        .await
        .unwrap();
        for language in languages {
            meta::ActiveModel {
                id: Set(format!("meta-{id}-{language}")),
                app_id: Set(Some(id.into())),
                lang: Set((*language).into()),
                name: Set(format!("{id} in {language}")),
                created_at: Set(now),
                updated_at: Set(now),
                ..Default::default()
            }
            .insert(db)
            .await
            .unwrap();
        }
    }

    #[tokio::test]
    async fn library_keeps_apps_without_english_metadata() {
        let db = database().await;
        insert_app(&db, "app-empty", "owner", &[]).await;
        insert_app(&db, "app-german", "owner", &["de"]).await;
        insert_app(&db, "app-english", "owner", &["en"]).await;
        insert_app(&db, "app-other-owner", "someone-else", &["en"]).await;

        let page = library_page(&db, "owner", "en", 100, None).await.unwrap();
        assert_eq!(
            page.iter()
                .map(|(app, _)| app.id.as_str())
                .collect::<Vec<_>>(),
            ["app-empty", "app-english", "app-german"]
        );
        assert!(page[0].1.is_none());
        assert_eq!(page[2].1.as_ref().unwrap().name, "app-german in de");
    }

    #[tokio::test]
    async fn library_paginates_apps_before_their_translations() {
        let db = database().await;
        for id in ["app-a", "app-b", "app-c"] {
            insert_app(&db, id, "owner", &["de", "en"]).await;
        }
        let first = library_page(&db, "owner", "de", 2, Some(0)).await.unwrap();
        let second = library_page(&db, "owner", "de", 2, Some(2)).await.unwrap();
        assert_eq!(
            first
                .iter()
                .map(|(app, _)| app.id.as_str())
                .collect::<Vec<_>>(),
            ["app-a", "app-b"]
        );
        assert_eq!(
            second
                .iter()
                .map(|(app, _)| app.id.as_str())
                .collect::<Vec<_>>(),
            ["app-c"]
        );
        assert!(
            first
                .iter()
                .chain(&second)
                .all(|(_, metadata)| metadata.as_ref().is_some_and(|meta| meta.lang == "de"))
        );
    }

    #[tokio::test]
    async fn library_prefers_the_requested_language_then_english() {
        let db = database().await;
        insert_app(&db, "app-requested", "owner", &["fr", "en", "de"]).await;
        insert_app(&db, "app-fallback", "owner", &["fr", "en"]).await;
        let page = library_page(&db, "owner", "de", 100, None).await.unwrap();
        assert_eq!(
            page.iter()
                .map(|(app, metadata)| (app.id.as_str(), metadata.as_ref().unwrap().lang.as_str()))
                .collect::<Vec<_>>(),
            [("app-fallback", "en"), ("app-requested", "de")]
        );
    }

    #[test]
    fn library_hides_tombstoned_and_half_forked_apps_and_bot_workspaces() {
        let sql = library_apps("user_1")
            .into_query()
            .to_string(PostgresQueryBuilder);

        assert!(sql.contains(r#""App"."status" <> 'INACTIVE'"#), "{sql}");
        assert!(sql.contains(r#""App"."botId" IS NULL"#), "{sql}");
        assert!(sql.contains(r#""Membership"."userId" = 'user_1'"#), "{sql}");
    }
}

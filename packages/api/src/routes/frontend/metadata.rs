use axum::{
    Json,
    extract::{Path, Query, State},
    http::header,
    response::{IntoResponse, Response},
};
use flow_like::bit::Metadata;
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder};
use serde::Serialize;

use crate::{entity::meta, error::ApiError, routes::app::meta::MetaMode, state::AppState};

use super::{RouteQuery, load_route};

/// Branding is public for a published hosted route, including its sign-in screen.
#[derive(Debug, Serialize)]
pub(super) struct FrontendMetadata {
    pub name: String,
    pub description: String,
    pub image: Option<String>,
    pub icon: Option<String>,
    pub card: &'static str,
}

pub(super) async fn get(
    State(state): State<AppState>,
    Path(app_id): Path<String>,
    Query(route): Query<RouteQuery>,
) -> Result<Response, ApiError> {
    let metadata = load(&state, &app_id, &route.path()).await?;
    Ok(([(header::CACHE_CONTROL, "no-store")], Json(metadata)).into_response())
}

pub(super) async fn load(
    state: &AppState,
    app_id: &str,
    route: &str,
) -> Result<FrontendMetadata, ApiError> {
    // Publication grants access to branding only. Bootstrap and execution still
    // enforce the Event's visitor authentication and session requirements.
    load_route(state, app_id, route).await?;
    let mut metadata = app_metadata(&state.db, app_id).await?;
    metadata.preview_media.clear();
    if metadata.icon.is_some() || metadata.thumbnail.is_some() {
        let credentials = state.master_credentials().await?;
        let store = credentials.to_store(false).await?;
        metadata
            .presign(MetaMode::App(app_id.into()).media_prefix(app_id), &store)
            .await;
    }
    Ok(branding(metadata))
}

async fn app_metadata<C: ConnectionTrait>(
    db: &C,
    app_id: &str,
) -> Result<Metadata, sea_orm::DbErr> {
    let metadata = meta::Entity::find()
        .filter(meta::Column::AppId.eq(app_id))
        .filter(meta::Column::BitId.is_null())
        .filter(meta::Column::TemplateId.is_null())
        .filter(meta::Column::CourseId.is_null())
        .filter(meta::Column::WidgetId.is_null())
        .filter(meta::Column::WasmPackageId.is_null())
        .filter(meta::Column::GroupId.is_null());
    let english = metadata
        .clone()
        .filter(meta::Column::Lang.eq("en"))
        .one(db)
        .await?;
    let record = match english {
        Some(record) => Some(record),
        None => metadata.order_by_asc(meta::Column::Lang).one(db).await?,
    };
    Ok(record.map(Metadata::from).unwrap_or_else(|| Metadata {
        name: "Flow-Like app".into(),
        description: String::new(),
        ..Metadata::default()
    }))
}

fn safe_image(value: Option<String>) -> Option<String> {
    let value = value?;
    if value.chars().any(char::is_control) || value.contains('\\') {
        return None;
    }
    let url = reqwest::Url::parse(&value).ok()?;
    (matches!(url.scheme(), "http" | "https")
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none())
    .then(|| url.to_string())
}

fn branding(metadata: Metadata) -> FrontendMetadata {
    let icon = safe_image(metadata.icon);
    let thumbnail = safe_image(metadata.thumbnail);
    let card = if thumbnail.is_some() {
        "summary_large_image"
    } else {
        "summary"
    };
    let name: String = metadata.name.trim().chars().take(120).collect();
    FrontendMetadata {
        name: if name.is_empty() {
            "Flow-Like app".into()
        } else {
            name
        },
        description: metadata.description.trim().chars().take(280).collect(),
        image: thumbnail.or_else(|| icon.clone()),
        icon,
        card,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use sea_orm::{ConnectOptions, Database, Schema, Set, sea_query::Table};

    #[tokio::test]
    async fn branding_prefers_english_and_never_uses_another_app_or_childs_metadata() {
        let mut options = ConnectOptions::new("sqlite::memory:");
        options.max_connections(1);
        let db = Database::connect(options).await.unwrap();
        db.execute_unprepared("ATTACH DATABASE ':memory:' AS public")
            .await
            .unwrap();
        let source = Schema::new(db.get_database_backend()).create_table_from_entity(meta::Entity);
        let mut table = Table::create();
        table.table(source.get_table_name().unwrap().clone());
        for column in source.get_columns() {
            table.col(column.clone());
        }
        db.execute(&table).await.unwrap();
        for (id, app_id, language, name, widget_id) in [
            ("other", "other-app", "en", "Unrelated secret", None),
            ("widget", "app", "en", "Private widget", Some("widget")),
            ("fr", "app", "fr", "French name", None),
            ("de", "app", "de", "German name", None),
        ] {
            meta::Entity::insert(meta::ActiveModel {
                id: Set(id.into()),
                app_id: Set(Some(app_id.into())),
                widget_id: Set(widget_id.map(String::from)),
                lang: Set(language.into()),
                name: Set(name.into()),
                description: Set(Some("App description".into())),
                created_at: Set(Utc::now().fixed_offset()),
                updated_at: Set(Utc::now().fixed_offset()),
                ..Default::default()
            })
            .exec(&db)
            .await
            .unwrap();
        }
        assert_eq!(app_metadata(&db, "app").await.unwrap().name, "German name");
        meta::Entity::insert(meta::ActiveModel {
            id: Set("en".into()),
            app_id: Set(Some("app".into())),
            lang: Set("en".into()),
            name: Set("English name".into()),
            created_at: Set(Utc::now().fixed_offset()),
            updated_at: Set(Utc::now().fixed_offset()),
            ..Default::default()
        })
        .exec(&db)
        .await
        .unwrap();
        assert_eq!(app_metadata(&db, "app").await.unwrap().name, "English name");
        let missing = app_metadata(&db, "missing").await.unwrap();
        assert_eq!(missing.name, "Flow-Like app");
        assert!(missing.description.is_empty());
    }

    #[test]
    fn cards_prefer_the_thumbnail_then_the_icon_and_include_only_branding() {
        let mut metadata = Metadata {
            name: "  Example app  ".into(),
            description: "  Description  ".into(),
            icon: Some("https://media.example/icon.webp".into()),
            thumbnail: Some("https://media.example/cover.webp?signature=a&b=2".into()),
            long_description: Some("Unrelated details".into()),
            preview_media: vec!["Private screenshot".into()],
            ..Metadata::default()
        };
        let cover = branding(metadata.clone());
        assert_eq!(cover.name, "Example app");
        assert_eq!(cover.description, "Description");
        assert_eq!(cover.image, metadata.thumbnail);
        assert_eq!(cover.card, "summary_large_image");
        let json = serde_json::to_value(cover).unwrap();
        assert_eq!(json.as_object().unwrap().len(), 5);
        assert!(json.get("long_description").is_none());
        assert!(json.get("preview_media").is_none());

        metadata.thumbnail = None;
        let icon = branding(metadata.clone());
        assert_eq!(icon.image, metadata.icon);
        assert_eq!(icon.card, "summary");
        metadata.icon = None;
        assert!(branding(metadata).image.is_none());
    }

    #[test]
    fn invalid_media_is_omitted_and_text_is_bounded_without_splitting_unicode() {
        for invalid in [
            "javascript:alert(1)",
            "data:image/png;base64,AAAA",
            "//evil.test/image",
            "unsigned-media-id",
            "https://user:secret@media.example/icon.webp",
            "https://media.example/image\n.webp",
            "https://media.example\\image.webp",
        ] {
            assert!(safe_image(Some(invalid.into())).is_none(), "{invalid}");
        }
        let result = branding(Metadata {
            name: "ü".repeat(140),
            description: "é".repeat(300),
            thumbnail: Some("javascript:alert(1)".into()),
            icon: Some("https://media.example/icon.webp".into()),
            ..Metadata::default()
        });
        assert_eq!(result.name.chars().count(), 120);
        assert_eq!(result.description.chars().count(), 280);
        assert_eq!(result.card, "summary");
        assert_eq!(
            result.image.as_deref(),
            Some("https://media.example/icon.webp")
        );
        assert_eq!(
            branding(Metadata {
                name: " \n ".into(),
                ..Metadata::default()
            })
            .name,
            "Flow-Like app"
        );
    }
}

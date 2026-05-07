use crate::{
    entity::{
        course, course_module, lesson, meta,
        sea_orm_active_enums::{CourseCategory, CourseDifficulty},
    },
    error::ApiError,
    middleware::jwt::AppUser,
    permission::global_permission::GlobalPermission,
    routes::LanguageParams,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like_storage::Path as FlowPath;
use flow_like_types::{anyhow, create_id};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter,
    QueryOrder, QuerySelect, TransactionTrait,
};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use utoipa::ToSchema;

#[derive(Clone, Serialize, Deserialize, ToSchema)]
pub struct CourseListItem {
    pub id: String,
    pub language: String,
    pub slug: Option<String>,
    pub difficulty: String,
    pub category: String,
    pub estimated_minutes: i32,
    pub is_published: bool,
    pub icon_url: Option<String>,
    pub banner_url: Option<String>,
    pub tags: Vec<String>,
    pub position: Option<i32>,
    pub name: Option<String>,
    pub description: Option<String>,
}

#[derive(Clone, Serialize, Deserialize, ToSchema)]
pub struct CourseUpsertBody {
    pub language: String,
    pub slug: Option<String>,
    pub difficulty: Option<String>,
    pub category: Option<String>,
    pub estimated_minutes: Option<i32>,
    pub is_published: Option<bool>,
    #[serde(default)]
    pub icon_url: Option<String>,
    #[serde(default)]
    pub banner_url: Option<String>,
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub position: Option<i32>,
    pub name: String,
    pub description: Option<String>,
    pub long_description: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema, Default)]
pub struct ListCoursesQuery {
    pub language: Option<String>,
    pub category: Option<String>,
    pub difficulty: Option<String>,
    pub include_unpublished: Option<bool>,
    pub limit: Option<u64>,
    pub offset: Option<u64>,
}

fn parse_difficulty(s: &str) -> CourseDifficulty {
    match s.to_uppercase().as_str() {
        "INTERMEDIATE" => CourseDifficulty::Intermediate,
        "ADVANCED" => CourseDifficulty::Advanced,
        "EXPERT" => CourseDifficulty::Expert,
        _ => CourseDifficulty::Beginner,
    }
}

fn parse_category(s: &str) -> CourseCategory {
    match s.to_uppercase().as_str() {
        "GETTING_STARTED" => CourseCategory::GettingStarted,
        "FLOWS" => CourseCategory::Flows,
        "PAGES" => CourseCategory::Pages,
        "EVENTS" => CourseCategory::Events,
        "DATA" => CourseCategory::Data,
        "AI" => CourseCategory::Ai,
        "INTEGRATIONS" => CourseCategory::Integrations,
        "DEPLOYMENT" => CourseCategory::Deployment,
        "ADVANCED" => CourseCategory::Advanced,
        "EXPERT" => CourseCategory::Expert,
        _ => CourseCategory::General,
    }
}

fn difficulty_to_string(d: &CourseDifficulty) -> &'static str {
    match d {
        CourseDifficulty::Beginner => "BEGINNER",
        CourseDifficulty::Intermediate => "INTERMEDIATE",
        CourseDifficulty::Advanced => "ADVANCED",
        CourseDifficulty::Expert => "EXPERT",
    }
}

fn category_to_string(c: &CourseCategory) -> &'static str {
    match c {
        CourseCategory::General => "GENERAL",
        CourseCategory::GettingStarted => "GETTING_STARTED",
        CourseCategory::Flows => "FLOWS",
        CourseCategory::Pages => "PAGES",
        CourseCategory::Events => "EVENTS",
        CourseCategory::Data => "DATA",
        CourseCategory::Ai => "AI",
        CourseCategory::Integrations => "INTEGRATIONS",
        CourseCategory::Deployment => "DEPLOYMENT",
        CourseCategory::Advanced => "ADVANCED",
        CourseCategory::Expert => "EXPERT",
    }
}

fn default_language() -> String {
    "en".to_string()
}

fn normalize_media_extension(extension: &str) -> Result<String, ApiError> {
    let extension = extension
        .trim()
        .trim_start_matches('.')
        .to_ascii_lowercase();
    if extension.is_empty()
        || extension.len() > 10
        || !extension.chars().all(|ch| ch.is_ascii_alphanumeric())
    {
        return Err(ApiError::bad_request("Invalid media extension"));
    }
    Ok(extension)
}

fn transformed_course_media_file_name(value: &str) -> String {
    let media_id = value
        .rsplit_once('.')
        .map_or(value, |(stem, _extension)| stem);
    format!("{media_id}.webp")
}

fn course_media_storage_path(course_id: &str, file_name: &str) -> FlowPath {
    FlowPath::from("media")
        .child("courses")
        .child(course_id)
        .child(file_name)
}

async fn resolve_course_media_url(
    state: &AppState,
    course_id: &str,
    media_id_or_url: Option<&str>,
    fallback: Option<String>,
) -> Option<String> {
    let Some(media_id_or_url) = media_id_or_url.filter(|value| !value.is_empty()) else {
        return fallback;
    };

    if media_id_or_url.starts_with("http://") || media_id_or_url.starts_with("https://") {
        return Some(media_id_or_url.to_string());
    }

    let Ok(master_creds) = state.master_credentials().await else {
        return fallback;
    };
    let Ok(store) = master_creds.to_store(false).await else {
        return fallback;
    };

    let path = course_media_storage_path(
        course_id,
        &transformed_course_media_file_name(media_id_or_url),
    );

    match store
        .sign("GET", &path, Duration::from_secs(60 * 60 * 24))
        .await
    {
        Ok(url) => Some(url.to_string()),
        Err(_) => fallback,
    }
}

async fn check_course_read_access(
    state: &AppState,
    user: &AppUser,
) -> Result<GlobalPermission, ApiError> {
    let permission = user.global_permission(state.clone()).await?;
    if permission.contains(GlobalPermission::ReadCourses)
        || permission.contains(GlobalPermission::WriteCourses)
        || permission.contains(GlobalPermission::Admin)
    {
        Ok(permission)
    } else {
        Err(ApiError::FORBIDDEN)
    }
}

#[utoipa::path(
    get,
    path = "/courses",
    tag = "courses",
    params(
        ("language" = Option<String>, Query, description = "Filter by language code (default: en)"),
        ("category" = Option<String>, Query, description = "Filter by category"),
        ("difficulty" = Option<String>, Query, description = "Filter by difficulty"),
        ("include_unpublished" = Option<bool>, Query, description = "Include unpublished (requires ReadCourses, WriteCourses, or Admin)"),
        ("limit" = Option<u64>, Query, description = "Maximum results (max 100)"),
        ("offset" = Option<u64>, Query, description = "Offset for pagination")
    ),
    responses(
        (status = 200, description = "Returns the list of available learning courses", body = Vec<CourseListItem>)
    )
)]
#[tracing::instrument(name = "GET /courses", skip(state, user))]
pub async fn list_courses(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(q): Query<ListCoursesQuery>,
) -> Result<Json<Vec<CourseListItem>>, ApiError> {
    let language = q.language.clone().unwrap_or_else(|| "en".to_string());
    let limit = q.limit.unwrap_or(100).min(100);

    let mut query = course::Entity::find()
        .order_by_asc(course::Column::Position)
        .order_by_desc(course::Column::UpdatedAt);

    let include_unpublished = q.include_unpublished.unwrap_or(false);
    if include_unpublished {
        check_course_read_access(&state, &user).await?;
    } else {
        query = query.filter(course::Column::IsPublished.eq(true));
    }

    if let Some(cat) = q.category.as_deref() {
        query = query.filter(course::Column::Category.eq(parse_category(cat)));
    }
    if let Some(diff) = q.difficulty.as_deref() {
        query = query.filter(course::Column::Difficulty.eq(parse_difficulty(diff)));
    }

    let courses_with_meta = query
        .find_with_related(meta::Entity)
        .filter(
            meta::Column::Lang
                .eq(&language)
                .or(meta::Column::Lang.eq("en")),
        )
        .limit(Some(limit))
        .offset(q.offset)
        .all(&state.db)
        .await?;

    let mut items = Vec::with_capacity(courses_with_meta.len());
    for (c, metas) in courses_with_meta {
        let chosen = metas
            .iter()
            .find(|m| m.lang == language)
            .or_else(|| metas.first());
        let icon_url = resolve_course_media_url(
            &state,
            &c.id,
            chosen.and_then(|m| m.icon.as_deref()),
            c.icon_url,
        )
        .await;
        let banner_url = resolve_course_media_url(
            &state,
            &c.id,
            chosen.and_then(|m| m.thumbnail.as_deref()),
            c.banner_url,
        )
        .await;

        items.push(CourseListItem {
            id: c.id,
            language: c.language,
            slug: c.slug,
            difficulty: difficulty_to_string(&c.difficulty).into(),
            category: category_to_string(&c.category).into(),
            estimated_minutes: c.estimated_minutes,
            is_published: c.is_published,
            icon_url,
            banner_url,
            tags: c.tags.unwrap_or_default(),
            position: c.position,
            name: chosen.map(|m| m.name.clone()),
            description: chosen.and_then(|m| m.description.clone()),
        });
    }

    items.sort_by(|a, b| {
        match (a.position, b.position) {
            (Some(ap), Some(bp)) => ap.cmp(&bp),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        }
        .then_with(|| {
            a.name
                .as_deref()
                .unwrap_or("")
                .to_lowercase()
                .cmp(&b.name.as_deref().unwrap_or("").to_lowercase())
        })
    });

    Ok(Json(items))
}

#[derive(Clone, Serialize, Deserialize, ToSchema)]
pub struct CourseDetail {
    pub id: String,
    pub language: String,
    pub slug: Option<String>,
    pub difficulty: String,
    pub category: String,
    pub estimated_minutes: i32,
    pub is_published: bool,
    pub icon_url: Option<String>,
    pub banner_url: Option<String>,
    pub tags: Vec<String>,
    pub position: Option<i32>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub long_description: Option<String>,
}

#[utoipa::path(
    get,
    path = "/courses/{course_id}",
    tag = "courses",
    params(
        ("course_id" = String, Path, description = "Course identifier"),
        ("language" = Option<String>, Query, description = "Preferred language (default: en)")
    ),
    responses(
        (status = 200, description = "Returns the course details with localized metadata", body = CourseDetail),
        (status = 404, description = "Course not found")
    )
)]
#[tracing::instrument(name = "GET /courses/{course_id}", skip(state, user))]
pub async fn get_course(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(course_id): Path<String>,
    Query(q): Query<LanguageParams>,
) -> Result<Json<CourseDetail>, ApiError> {
    let language = q.language.clone().unwrap_or_else(|| "en".to_string());

    let (c, metas) = course::Entity::find_by_id(&course_id)
        .find_with_related(meta::Entity)
        .filter(
            meta::Column::Lang
                .eq(&language)
                .or(meta::Column::Lang.eq("en")),
        )
        .all(&state.db)
        .await?
        .into_iter()
        .next()
        .ok_or(ApiError::NOT_FOUND)?;

    if !c.is_published {
        check_course_read_access(&state, &user).await?;
    }

    let chosen = metas
        .iter()
        .find(|m| m.lang == language)
        .or_else(|| metas.first());
    let icon_url = resolve_course_media_url(
        &state,
        &c.id,
        chosen.and_then(|m| m.icon.as_deref()),
        c.icon_url,
    )
    .await;
    let banner_url = resolve_course_media_url(
        &state,
        &c.id,
        chosen.and_then(|m| m.thumbnail.as_deref()),
        c.banner_url,
    )
    .await;

    Ok(Json(CourseDetail {
        id: c.id,
        language: c.language,
        slug: c.slug,
        difficulty: difficulty_to_string(&c.difficulty).into(),
        category: category_to_string(&c.category).into(),
        estimated_minutes: c.estimated_minutes,
        is_published: c.is_published,
        icon_url,
        banner_url,
        tags: c.tags.unwrap_or_default(),
        position: c.position,
        name: chosen.map(|m| m.name.clone()),
        description: chosen.and_then(|m| m.description.clone()),
        long_description: chosen.and_then(|m| m.long_description.clone()),
    }))
}

#[utoipa::path(
    put,
    path = "/courses/{course_id}",
    tag = "courses",
    params(("course_id" = String, Path, description = "Course identifier")),
    request_body = CourseUpsertBody,
    responses(
        (status = 200, description = "Course created or updated", body = CourseDetail),
        (status = 403, description = "Forbidden")
    )
)]
#[tracing::instrument(name = "PUT /courses/{course_id}", skip(state, user, body))]
pub async fn upsert_course(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(course_id): Path<String>,
    Json(body): Json<CourseUpsertBody>,
) -> Result<Json<CourseDetail>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::WriteCourses)
        .await?;
    let sub = user.sub()?;

    let now = chrono::Utc::now().naive_utc();
    let existing = course::Entity::find_by_id(&course_id)
        .one(&state.db)
        .await?;

    let difficulty = body
        .difficulty
        .as_deref()
        .map(parse_difficulty)
        .unwrap_or(CourseDifficulty::Beginner);
    let category = body
        .category
        .as_deref()
        .map(parse_category)
        .unwrap_or(CourseCategory::General);

    let saved = if let Some(existing) = existing {
        let mut active = existing.into_active_model();
        active.language = Set(body.language.clone());
        active.slug = Set(body.slug.clone());
        active.difficulty = Set(difficulty);
        active.category = Set(category);
        active.estimated_minutes = Set(body.estimated_minutes.unwrap_or(0));
        active.is_published = Set(body.is_published.unwrap_or(false));
        active.tags = Set(body.tags.clone());
        active.position = Set(body.position);
        active.updated_at = Set(now);
        active.update(&state.db).await?
    } else {
        let active = course::ActiveModel {
            id: Set(course_id.clone()),
            language: Set(body.language.clone()),
            slug: Set(body.slug.clone()),
            difficulty: Set(difficulty),
            category: Set(category),
            estimated_minutes: Set(body.estimated_minutes.unwrap_or(0)),
            is_published: Set(body.is_published.unwrap_or(false)),
            author_id: Set(Some(sub.clone())),
            icon_url: Set(None),
            banner_url: Set(None),
            tags: Set(body.tags.clone()),
            position: Set(body.position),
            created_at: Set(now),
            updated_at: Set(now),
        };
        active.insert(&state.db).await?
    };

    let existing_meta = meta::Entity::find()
        .filter(meta::Column::CourseId.eq(&course_id))
        .filter(meta::Column::Lang.eq(&body.language))
        .one(&state.db)
        .await?;

    if let Some(m) = existing_meta {
        let mut active = m.into_active_model();
        active.name = Set(body.name.clone());
        active.description = Set(body.description.clone());
        active.long_description = Set(body.long_description.clone());
        active.updated_at = Set(now);
        active.update(&state.db).await?;
    } else {
        let active = meta::ActiveModel {
            id: Set(create_id()),
            lang: Set(body.language.clone()),
            name: Set(body.name.clone()),
            description: Set(body.description.clone()),
            long_description: Set(body.long_description.clone()),
            course_id: Set(Some(course_id.clone())),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        };
        active.insert(&state.db).await?;
    }

    let saved_meta = meta::Entity::find()
        .filter(meta::Column::CourseId.eq(&course_id))
        .filter(meta::Column::Lang.eq(&body.language))
        .one(&state.db)
        .await?;
    let icon_url = resolve_course_media_url(
        &state,
        &saved.id,
        saved_meta.as_ref().and_then(|m| m.icon.as_deref()),
        saved.icon_url.clone(),
    )
    .await;
    let banner_url = resolve_course_media_url(
        &state,
        &saved.id,
        saved_meta.as_ref().and_then(|m| m.thumbnail.as_deref()),
        saved.banner_url.clone(),
    )
    .await;

    Ok(Json(CourseDetail {
        id: saved.id,
        language: saved.language,
        slug: saved.slug,
        difficulty: difficulty_to_string(&saved.difficulty).into(),
        category: category_to_string(&saved.category).into(),
        estimated_minutes: saved.estimated_minutes,
        is_published: saved.is_published,
        icon_url,
        banner_url,
        tags: saved.tags.unwrap_or_default(),
        position: saved.position,
        name: Some(body.name),
        description: body.description,
        long_description: body.long_description,
    }))
}

#[derive(Clone, Serialize, Deserialize, ToSchema)]
pub struct LessonSummary {
    pub id: String,
    pub module_id: String,
    pub title: String,
    pub position: i32,
    pub estimated_minutes: i32,
    pub is_optional: bool,
    pub has_video: bool,
}

#[derive(Clone, Serialize, Deserialize, ToSchema)]
pub struct ModuleWithLessons {
    pub id: String,
    pub course_id: String,
    pub title: String,
    pub description: Option<String>,
    pub position: i32,
    pub lessons: Vec<LessonSummary>,
}

#[derive(Clone, Serialize, Deserialize, ToSchema)]
pub struct CourseStructure {
    pub course: CourseDetail,
    pub modules: Vec<ModuleWithLessons>,
}

#[utoipa::path(
    get,
    path = "/courses/{course_id}/structure",
    tag = "courses",
    params(
        ("course_id" = String, Path, description = "Course identifier"),
        ("language" = Option<String>, Query, description = "Preferred language (default: en)")
    ),
    responses(
        (status = 200, description = "Returns the full course structure with modules and lesson summaries", body = CourseStructure),
        (status = 404, description = "Course not found")
    )
)]
#[tracing::instrument(name = "GET /courses/{course_id}/structure", skip(state, user))]
pub async fn get_course_structure(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(course_id): Path<String>,
    Query(q): Query<LanguageParams>,
) -> Result<Json<CourseStructure>, ApiError> {
    let language = q.language.clone().unwrap_or_else(|| "en".to_string());

    let (c, metas) = course::Entity::find_by_id(&course_id)
        .find_with_related(meta::Entity)
        .filter(
            meta::Column::Lang
                .eq(&language)
                .or(meta::Column::Lang.eq("en")),
        )
        .all(&state.db)
        .await?
        .into_iter()
        .next()
        .ok_or(ApiError::NOT_FOUND)?;

    if !c.is_published {
        check_course_read_access(&state, &user).await?;
    }

    let chosen = metas
        .iter()
        .find(|m| m.lang == language)
        .or_else(|| metas.first());
    let icon_url = resolve_course_media_url(
        &state,
        &c.id,
        chosen.and_then(|m| m.icon.as_deref()),
        c.icon_url.clone(),
    )
    .await;
    let banner_url = resolve_course_media_url(
        &state,
        &c.id,
        chosen.and_then(|m| m.thumbnail.as_deref()),
        c.banner_url.clone(),
    )
    .await;

    let detail = CourseDetail {
        id: c.id.clone(),
        language: c.language.clone(),
        slug: c.slug.clone(),
        difficulty: difficulty_to_string(&c.difficulty).into(),
        category: category_to_string(&c.category).into(),
        estimated_minutes: c.estimated_minutes,
        is_published: c.is_published,
        icon_url,
        banner_url,
        tags: c.tags.clone().unwrap_or_default(),
        position: c.position,
        name: chosen.map(|m| m.name.clone()),
        description: chosen.and_then(|m| m.description.clone()),
        long_description: chosen.and_then(|m| m.long_description.clone()),
    };

    let modules = course_module::Entity::find()
        .filter(course_module::Column::CourseId.eq(&course_id))
        .order_by_asc(course_module::Column::Position)
        .all(&state.db)
        .await?;

    let module_ids: Vec<String> = modules.iter().map(|m| m.id.clone()).collect();
    let lessons = if module_ids.is_empty() {
        Vec::new()
    } else {
        lesson::Entity::find()
            .filter(lesson::Column::ModuleId.is_in(module_ids))
            .order_by_asc(lesson::Column::Position)
            .all(&state.db)
            .await?
    };

    let modules_out: Vec<ModuleWithLessons> = modules
        .into_iter()
        .map(|m| {
            let lessons = lessons
                .iter()
                .filter(|l| l.module_id == m.id)
                .map(|l| LessonSummary {
                    id: l.id.clone(),
                    module_id: l.module_id.clone(),
                    title: l.title.clone(),
                    position: l.position,
                    estimated_minutes: l.estimated_minutes,
                    is_optional: l.is_optional,
                    has_video: l.video_url.is_some(),
                })
                .collect();
            ModuleWithLessons {
                id: m.id,
                course_id: m.course_id,
                title: m.title,
                description: m.description,
                position: m.position,
                lessons,
            }
        })
        .collect();

    Ok(Json(CourseStructure {
        course: detail,
        modules: modules_out,
    }))
}

#[derive(Clone, Copy, Debug, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum CourseMediaItem {
    Icon,
    Thumbnail,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CourseMediaQuery {
    #[serde(default = "default_language")]
    pub language: String,
    pub item: CourseMediaItem,
    pub extension: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PushCourseMediaResponse {
    pub signed_url: String,
}

#[utoipa::path(
    put,
    path = "/courses/{course_id}/meta/media",
    tag = "courses",
    description = "Get a signed upload URL for course metadata media.",
    params(
        ("course_id" = String, Path, description = "Course identifier"),
        ("language" = Option<String>, Query, description = "Language code (default en)"),
        ("item" = String, Query, description = "Media item: icon or thumbnail"),
        ("extension" = String, Query, description = "File extension (e.g. png, webp)")
    ),
    responses(
        (status = 200, description = "Signed upload URL", body = PushCourseMediaResponse),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Course or metadata not found")
    )
)]
#[tracing::instrument(name = "PUT /courses/{course_id}/meta/media", skip(state, user))]
pub async fn push_course_media(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(course_id): Path<String>,
    Query(query): Query<CourseMediaQuery>,
) -> Result<Json<PushCourseMediaResponse>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::WriteCourses)
        .await?;

    course::Entity::find_by_id(&course_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    let txn = state.db.begin().await?;
    let existing_meta = meta::Entity::find()
        .filter(meta::Column::CourseId.eq(&course_id))
        .filter(meta::Column::Lang.eq(&query.language))
        .one(&txn)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    let mut model: meta::ActiveModel = existing_meta.clone().into();
    model.updated_at = Set(chrono::Utc::now().naive_utc());
    let item_id = create_id();
    let extension = normalize_media_extension(&query.extension)?;
    let item_name = format!("{item_id}.{extension}");

    let master_store = state.master_credentials().await?;
    let master_store = master_store.to_store(false).await?;

    match query.item {
        CourseMediaItem::Icon => {
            if let Some(icon) = &existing_meta.icon
                && !icon.starts_with("http://")
                && !icon.starts_with("https://")
            {
                let path = course_media_storage_path(
                    &course_id,
                    &transformed_course_media_file_name(icon),
                );
                if let Err(err) = master_store.as_generic().delete(&path).await {
                    tracing::error!(
                        "Failed to delete existing course icon at {}: {:?}",
                        path,
                        err
                    );
                }
            }
            model.icon = Set(Some(item_id.clone()));
        }
        CourseMediaItem::Thumbnail => {
            if let Some(thumbnail) = &existing_meta.thumbnail
                && !thumbnail.starts_with("http://")
                && !thumbnail.starts_with("https://")
            {
                let path = course_media_storage_path(
                    &course_id,
                    &transformed_course_media_file_name(thumbnail),
                );
                if let Err(err) = master_store.as_generic().delete(&path).await {
                    tracing::error!(
                        "Failed to delete existing course thumbnail at {}: {:?}",
                        path,
                        err
                    );
                }
            }
            model.thumbnail = Set(Some(item_id.clone()));
        }
    }

    model.update(&txn).await?;

    let path = course_media_storage_path(&course_id, &item_name);
    let signed_url = master_store
        .sign("PUT", &path, Duration::from_secs(60 * 60 * 24))
        .await
        .map_err(|e| {
            let id = create_id();
            tracing::error!(
                "[{}] Failed to sign URL for course media item '{}' - {:?}",
                id,
                item_name,
                e
            );
            ApiError::internal_error(anyhow!("Failed to create signed URL, reference ID: {}", id))
        })?;

    txn.commit().await?;
    Ok(Json(PushCourseMediaResponse {
        signed_url: signed_url.to_string(),
    }))
}

#[utoipa::path(
    delete,
    path = "/courses/{course_id}",
    tag = "courses",
    params(("course_id" = String, Path, description = "Course identifier")),
    responses(
        (status = 200, description = "Course deleted"),
        (status = 403, description = "Forbidden")
    )
)]
#[tracing::instrument(name = "DELETE /courses/{course_id}", skip(state, user))]
pub async fn delete_course(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(course_id): Path<String>,
) -> Result<Json<()>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::WriteCourses)
        .await?;
    course::Entity::delete_by_id(course_id)
        .exec(&state.db)
        .await?;
    Ok(Json(()))
}

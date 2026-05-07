use crate::{
    entity::{course, learning_path, learning_path_course, meta},
    error::ApiError,
    middleware::jwt::AppUser,
    permission::global_permission::GlobalPermission,
    routes::course::{access::has_course_read_grant, courses::CourseListItem},
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter,
    QueryOrder,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;

#[derive(Clone, Serialize, Deserialize, ToSchema)]
pub struct LearningPathStepView {
    pub course_id: String,
    pub position: i32,
    pub course: Option<CourseListItem>,
}

#[derive(Clone, Serialize, Deserialize, ToSchema)]
pub struct LearningPathView {
    pub id: String,
    pub slug: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub position: i32,
    pub is_published: bool,
    pub steps: Vec<LearningPathStepView>,
}

#[derive(Clone, Debug, Default, Deserialize, ToSchema)]
pub struct ListLearningPathsQuery {
    pub language: Option<String>,
    pub include_unpublished: Option<bool>,
}

#[derive(Clone, Debug, Default, Deserialize, ToSchema)]
pub struct LanguageOnlyQuery {
    pub language: Option<String>,
}

#[derive(Clone, Debug, Deserialize, ToSchema)]
pub struct LearningPathUpsertBody {
    pub title: String,
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub position: Option<i32>,
    #[serde(default)]
    pub is_published: Option<bool>,
}

#[derive(Clone, Debug, Deserialize, ToSchema)]
pub struct LearningPathStepUpsertBody {
    pub position: i32,
}

async fn course_list_items_by_id(
    state: &AppState,
    course_ids: &[String],
    language: &str,
    include_unpublished: bool,
) -> Result<HashMap<String, CourseListItem>, ApiError> {
    if course_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let mut query =
        course::Entity::find().filter(course::Column::Id.is_in(course_ids.iter().cloned()));
    if !include_unpublished {
        query = query.filter(course::Column::IsPublished.eq(true));
    }
    let courses_with_meta = query
        .find_with_related(meta::Entity)
        .filter(
            meta::Column::Lang
                .eq(language)
                .or(meta::Column::Lang.eq("en")),
        )
        .all(&state.db)
        .await?;

    let mut out = HashMap::with_capacity(courses_with_meta.len());
    for (c, metas) in courses_with_meta {
        let chosen = metas
            .iter()
            .find(|m| m.lang == language)
            .or_else(|| metas.first());
        out.insert(
            c.id.clone(),
            CourseListItem {
                id: c.id,
                language: c.language,
                slug: c.slug,
                difficulty: format!("{:?}", c.difficulty).to_uppercase(),
                category: format!("{:?}", c.category).to_uppercase(),
                estimated_minutes: c.estimated_minutes,
                is_published: c.is_published,
                icon_url: c.icon_url,
                banner_url: c.banner_url,
                tags: c.tags.unwrap_or_default(),
                position: c.position,
                name: chosen.map(|m| m.name.clone()),
                description: chosen.and_then(|m| m.description.clone()),
            },
        );
    }
    Ok(out)
}

fn build_path_view(
    model: learning_path::Model,
    steps: Vec<learning_path_course::Model>,
    courses: &HashMap<String, CourseListItem>,
    include_missing_courses: bool,
) -> LearningPathView {
    let mut steps: Vec<learning_path_course::Model> = steps;
    steps.sort_by_key(|s| s.position);
    let step_views = steps
        .into_iter()
        .filter_map(|s| {
            let course = courses.get(&s.course_id).cloned();
            if course.is_none() && !include_missing_courses {
                return None;
            }
            Some(LearningPathStepView {
                course,
                course_id: s.course_id,
                position: s.position,
            })
        })
        .collect();
    LearningPathView {
        id: model.id,
        slug: model.slug,
        title: model.title,
        description: model.description,
        position: model.position,
        is_published: model.is_published,
        steps: step_views,
    }
}

#[utoipa::path(
    get,
    path = "/courses/paths",
    tag = "courses",
    params(
        ("language" = Option<String>, Query, description = "Preferred language (default: en)"),
        ("include_unpublished" = Option<bool>, Query, description = "Include unpublished paths (requires ReadCourses, WriteCourses, or Admin)")
    ),
    responses(
        (status = 200, description = "Returns all published learning paths with their steps", body = Vec<LearningPathView>)
    )
)]
#[tracing::instrument(name = "GET /courses/paths", skip(state, user, q))]
pub async fn list_learning_paths(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(q): Query<ListLearningPathsQuery>,
) -> Result<Json<Vec<LearningPathView>>, ApiError> {
    let language = q.language.clone().unwrap_or_else(|| "en".to_string());
    let include_unpublished = q.include_unpublished.unwrap_or(false);

    if include_unpublished {
        let permission = user.global_permission(state.clone()).await?;
        if !(permission.contains(GlobalPermission::ReadCourses)
            || permission.contains(GlobalPermission::WriteCourses)
            || permission.contains(GlobalPermission::Admin))
        {
            return Err(ApiError::FORBIDDEN);
        }
    }

    let mut query = learning_path::Entity::find()
        .order_by_asc(learning_path::Column::Position)
        .order_by_asc(learning_path::Column::Title);
    if !include_unpublished {
        query = query.filter(learning_path::Column::IsPublished.eq(true));
    }

    let paths_with_steps = query
        .find_with_related(learning_path_course::Entity)
        .all(&state.db)
        .await?;

    let course_ids: Vec<String> = paths_with_steps
        .iter()
        .flat_map(|(_, steps)| steps.iter().map(|s| s.course_id.clone()))
        .collect();
    let courses =
        course_list_items_by_id(&state, &course_ids, &language, include_unpublished).await?;

    let views = paths_with_steps
        .into_iter()
        .map(|(p, steps)| build_path_view(p, steps, &courses, include_unpublished))
        .collect();

    Ok(Json(views))
}

#[utoipa::path(
    get,
    path = "/courses/paths/{path_id}",
    tag = "courses",
    params(
        ("path_id" = String, Path, description = "Learning path identifier"),
        ("language" = Option<String>, Query, description = "Preferred language (default: en)")
    ),
    responses(
        (status = 200, description = "Returns a single learning path with its steps", body = LearningPathView),
        (status = 404, description = "Learning path not found")
    )
)]
#[tracing::instrument(name = "GET /courses/paths/{path_id}", skip(state, user, q))]
pub async fn get_learning_path(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(path_id): Path<String>,
    Query(q): Query<LanguageOnlyQuery>,
) -> Result<Json<LearningPathView>, ApiError> {
    let language = q.language.clone().unwrap_or_else(|| "en".to_string());
    let mut paths_with_steps = learning_path::Entity::find_by_id(&path_id)
        .find_with_related(learning_path_course::Entity)
        .all(&state.db)
        .await?;
    let (model, steps) = paths_with_steps.pop().ok_or(ApiError::NOT_FOUND)?;
    let include_unpublished = has_course_read_grant(&state, &user).await;
    if !model.is_published && !include_unpublished {
        return Err(ApiError::FORBIDDEN);
    }

    let course_ids: Vec<String> = steps.iter().map(|s| s.course_id.clone()).collect();
    let courses =
        course_list_items_by_id(&state, &course_ids, &language, include_unpublished).await?;
    Ok(Json(build_path_view(
        model,
        steps,
        &courses,
        include_unpublished,
    )))
}

#[utoipa::path(
    put,
    path = "/courses/paths/{path_id}",
    tag = "courses",
    params(("path_id" = String, Path, description = "Learning path identifier")),
    request_body = LearningPathUpsertBody,
    responses(
        (status = 200, description = "Created or updated the learning path", body = LearningPathView),
        (status = 403, description = "Forbidden — requires WriteCourses permission")
    )
)]
#[tracing::instrument(name = "PUT /courses/paths/{path_id}", skip(state, user, body))]
pub async fn upsert_learning_path(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(path_id): Path<String>,
    Json(body): Json<LearningPathUpsertBody>,
) -> Result<Json<LearningPathView>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::WriteCourses)
        .await?;

    let now = chrono::Utc::now().naive_utc();
    let existing = learning_path::Entity::find_by_id(&path_id)
        .one(&state.db)
        .await?;

    let saved = if let Some(existing) = existing {
        let mut active = existing.into_active_model();
        active.title = Set(body.title.clone());
        active.slug = Set(body.slug.clone());
        active.description = Set(body.description.clone());
        if let Some(position) = body.position {
            active.position = Set(position);
        }
        if let Some(is_published) = body.is_published {
            active.is_published = Set(is_published);
        }
        active.updated_at = Set(now);
        active.update(&state.db).await?
    } else {
        let active = learning_path::ActiveModel {
            id: Set(path_id.clone()),
            title: Set(body.title.clone()),
            slug: Set(body.slug.clone()),
            description: Set(body.description.clone()),
            position: Set(body.position.unwrap_or(0)),
            is_published: Set(body.is_published.unwrap_or(false)),
            created_at: Set(now),
            updated_at: Set(now),
        };
        active.insert(&state.db).await?
    };

    let steps = learning_path_course::Entity::find()
        .filter(learning_path_course::Column::PathId.eq(&saved.id))
        .all(&state.db)
        .await?;
    let course_ids: Vec<String> = steps.iter().map(|s| s.course_id.clone()).collect();
    let courses = course_list_items_by_id(&state, &course_ids, "en", true).await?;
    Ok(Json(build_path_view(saved, steps, &courses, true)))
}

#[utoipa::path(
    delete,
    path = "/courses/paths/{path_id}",
    tag = "courses",
    params(("path_id" = String, Path, description = "Learning path identifier")),
    responses(
        (status = 204, description = "Deleted the learning path"),
        (status = 403, description = "Forbidden — requires WriteCourses permission")
    )
)]
#[tracing::instrument(name = "DELETE /courses/paths/{path_id}", skip(state, user))]
pub async fn delete_learning_path(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(path_id): Path<String>,
) -> Result<Json<()>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::WriteCourses)
        .await?;
    learning_path::Entity::delete_by_id(path_id)
        .exec(&state.db)
        .await?;
    Ok(Json(()))
}

#[utoipa::path(
    put,
    path = "/courses/paths/{path_id}/courses/{course_id}",
    tag = "courses",
    params(
        ("path_id" = String, Path, description = "Learning path identifier"),
        ("course_id" = String, Path, description = "Course identifier")
    ),
    request_body = LearningPathStepUpsertBody,
    responses(
        (status = 200, description = "Added or updated a course in the learning path"),
        (status = 403, description = "Forbidden — requires WriteCourses permission")
    )
)]
#[tracing::instrument(
    name = "PUT /courses/paths/{path_id}/courses/{course_id}",
    skip(state, user, body)
)]
pub async fn upsert_learning_path_step(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((path_id, course_id)): Path<(String, String)>,
    Json(body): Json<LearningPathStepUpsertBody>,
) -> Result<Json<()>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::WriteCourses)
        .await?;
    learning_path::Entity::find_by_id(&path_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    course::Entity::find_by_id(&course_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    let existing = learning_path_course::Entity::find_by_id((path_id.clone(), course_id.clone()))
        .one(&state.db)
        .await?;

    if let Some(existing) = existing {
        let mut active = existing.into_active_model();
        active.position = Set(body.position);
        active.update(&state.db).await?;
    } else {
        let active = learning_path_course::ActiveModel {
            path_id: Set(path_id),
            course_id: Set(course_id),
            position: Set(body.position),
        };
        active.insert(&state.db).await?;
    }
    Ok(Json(()))
}

#[utoipa::path(
    delete,
    path = "/courses/paths/{path_id}/courses/{course_id}",
    tag = "courses",
    params(
        ("path_id" = String, Path, description = "Learning path identifier"),
        ("course_id" = String, Path, description = "Course identifier")
    ),
    responses(
        (status = 204, description = "Removed the course from the learning path"),
        (status = 403, description = "Forbidden — requires WriteCourses permission")
    )
)]
#[tracing::instrument(
    name = "DELETE /courses/paths/{path_id}/courses/{course_id}",
    skip(state, user)
)]
pub async fn delete_learning_path_step(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((path_id, course_id)): Path<(String, String)>,
) -> Result<Json<()>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::WriteCourses)
        .await?;
    learning_path::Entity::find_by_id(&path_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    course::Entity::find_by_id(&course_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    learning_path_course::Entity::find_by_id((path_id.clone(), course_id.clone()))
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    learning_path_course::Entity::delete_by_id((path_id, course_id))
        .exec(&state.db)
        .await?;
    Ok(Json(()))
}

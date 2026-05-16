use crate::{
    entity::{challenge, course, course_app_link, course_module, lesson, lesson_app_ref},
    error::ApiError,
    middleware::jwt::AppUser,
    permission::global_permission::GlobalPermission,
    state::AppState,
};
use sea_orm::EntityTrait;

pub async fn has_course_read_grant(state: &AppState, user: &AppUser) -> bool {
    match user.global_permission(state.clone()).await {
        Ok(permission) => {
            permission.contains(GlobalPermission::ReadCourses)
                || permission.contains(GlobalPermission::WriteCourses)
                || permission.contains(GlobalPermission::Admin)
        }
        Err(_) => false,
    }
}

pub async fn ensure_course_readable(
    state: &AppState,
    user: &AppUser,
    course_id: &str,
) -> Result<course::Model, ApiError> {
    let course = course::Entity::find_by_id(course_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    if course.is_published || has_course_read_grant(state, user).await {
        Ok(course)
    } else {
        Err(ApiError::FORBIDDEN)
    }
}

pub async fn ensure_course_exists(
    state: &AppState,
    course_id: &str,
) -> Result<course::Model, ApiError> {
    course::Entity::find_by_id(course_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)
}

pub async fn ensure_module_in_course(
    state: &AppState,
    course_id: &str,
    module_id: &str,
) -> Result<course_module::Model, ApiError> {
    ensure_course_exists(state, course_id).await?;
    let module = course_module::Entity::find_by_id(module_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    if module.course_id == course_id {
        Ok(module)
    } else {
        Err(ApiError::NOT_FOUND)
    }
}

pub async fn ensure_lesson_in_module(
    state: &AppState,
    course_id: &str,
    module_id: &str,
    lesson_id: &str,
) -> Result<lesson::Model, ApiError> {
    ensure_module_in_course(state, course_id, module_id).await?;
    let lesson = lesson::Entity::find_by_id(lesson_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    if lesson.module_id == module_id {
        Ok(lesson)
    } else {
        Err(ApiError::NOT_FOUND)
    }
}

pub async fn ensure_lesson_in_course(
    state: &AppState,
    course_id: &str,
    lesson_id: &str,
) -> Result<lesson::Model, ApiError> {
    ensure_course_exists(state, course_id).await?;
    let lesson = lesson::Entity::find_by_id(lesson_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    let module = course_module::Entity::find_by_id(&lesson.module_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    if module.course_id == course_id {
        Ok(lesson)
    } else {
        Err(ApiError::NOT_FOUND)
    }
}

pub async fn ensure_challenge_in_lesson(
    state: &AppState,
    course_id: &str,
    lesson_id: &str,
    challenge_id: &str,
) -> Result<challenge::Model, ApiError> {
    ensure_lesson_in_course(state, course_id, lesson_id).await?;
    let challenge = challenge::Entity::find_by_id(challenge_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    if challenge.lesson_id == lesson_id {
        Ok(challenge)
    } else {
        Err(ApiError::NOT_FOUND)
    }
}

pub async fn ensure_app_link_in_course(
    state: &AppState,
    course_id: &str,
    link_id: &str,
) -> Result<course_app_link::Model, ApiError> {
    ensure_course_exists(state, course_id).await?;
    let link = course_app_link::Entity::find_by_id(link_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    if link.course_id == course_id {
        Ok(link)
    } else {
        Err(ApiError::NOT_FOUND)
    }
}

pub async fn ensure_app_ref_in_lesson(
    state: &AppState,
    course_id: &str,
    lesson_id: &str,
    ref_id: &str,
) -> Result<lesson_app_ref::Model, ApiError> {
    ensure_lesson_in_course(state, course_id, lesson_id).await?;
    let app_ref = lesson_app_ref::Entity::find_by_id(ref_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    if app_ref.lesson_id == lesson_id {
        Ok(app_ref)
    } else {
        Err(ApiError::NOT_FOUND)
    }
}

pub async fn ensure_challenge_course_readable(
    state: &AppState,
    user: &AppUser,
    challenge_id: &str,
) -> Result<challenge::Model, ApiError> {
    let challenge = challenge::Entity::find_by_id(challenge_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    let lesson = lesson::Entity::find_by_id(&challenge.lesson_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    let module = course_module::Entity::find_by_id(&lesson.module_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    ensure_course_readable(state, user, &module.course_id).await?;
    Ok(challenge)
}

pub async fn ensure_lesson_course_readable(
    state: &AppState,
    user: &AppUser,
    lesson_id: &str,
) -> Result<(lesson::Model, course_module::Model), ApiError> {
    let lesson = lesson::Entity::find_by_id(lesson_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    let module = course_module::Entity::find_by_id(&lesson.module_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    ensure_course_readable(state, user, &module.course_id).await?;
    Ok((lesson, module))
}

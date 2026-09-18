use crate::{
    ensure_in_project,
    entity::{app, comment},
    error::ApiError,
    middleware::jwt::AppUser,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like_types::create_id;
use sea_orm::sea_query::ExprTrait;
use sea_orm::sea_query::{Alias, Expr};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel,
    QueryFilter,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Deserialize, Debug, ToSchema)]
pub struct CommentBody {
    pub text: String,
    pub rating: i64,
}

#[derive(Serialize, Debug, ToSchema)]
pub struct CommentResponse {
    pub comment_id: String,
}

#[utoipa::path(
    put,
    path = "/apps/{app_id}/comments",
    tag = "comments",
    description = "Create or update a review for an app. One review per user per app.",
    params(
        ("app_id" = String, Path, description = "Application ID")
    ),
    request_body = CommentBody,
    responses(
        (status = 200, description = "Comment upserted", body = CommentResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "PUT /apps/{app_id}/comments", skip(state, user, body))]
pub async fn upsert_comment(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(body): Json<CommentBody>,
) -> Result<Json<CommentResponse>, ApiError> {
    let permission = ensure_in_project!(user, &app_id, &state);
    let sub = permission.sub()?;
    let new_rating = body.rating.clamp(1, 5);
    let new_comment_id = create_id();

    let comment_id = state
        .transaction(|txn| {
            let app_id = app_id.clone();
            let sub = sub.clone();
            let text = body.text.clone();
            let new_comment_id = new_comment_id.clone();
            Box::pin(async move {
                let existing = comment::Entity::find()
                    .filter(comment::Column::UserId.eq(&sub))
                    .filter(comment::Column::AppId.eq(&app_id))
                    .one(txn)
                    .await?;

                let comment_id = if let Some(existing) = existing {
                    let id = existing.id.clone();
                    let old_rating = existing.rating;
                    let mut active = existing.into_active_model();
                    active.text = Set(text);
                    active.rating = Set(new_rating);
                    active.updated_at = Set(chrono::Utc::now().fixed_offset());
                    active.update(txn).await?;
                    adjust_app_ratings(txn, &app_id, new_rating - old_rating, 0).await?;
                    id
                } else {
                    let model = comment::Model {
                        id: new_comment_id.clone(),
                        text,
                        rating: new_rating,
                        user_id: sub,
                        app_id: Some(app_id.clone()),
                        template_id: None,
                        package_id: None,
                        created_at: chrono::Utc::now().fixed_offset(),
                        updated_at: chrono::Utc::now().fixed_offset(),
                    };
                    let active = comment::ActiveModel::from(model).reset_all();
                    active.insert(txn).await?;
                    adjust_app_ratings(txn, &app_id, new_rating, 1).await?;
                    new_comment_id
                };
                Ok::<_, ApiError>(comment_id)
            })
        })
        .await?;

    Ok(Json(CommentResponse { comment_id }))
}

/// Atomically adjusts the rating counters on an App by the given deltas.
/// For a new comment: sum_delta = rating, count_delta = 1
/// For an updated comment: sum_delta = new_rating - old_rating, count_delta = 0
/// For a deleted comment: sum_delta = -rating, count_delta = -1
pub(super) async fn adjust_app_ratings(
    db: &impl ConnectionTrait,
    app_id: &str,
    sum_delta: i64,
    count_delta: i64,
) -> Result<(), ApiError> {
    let new_sum = Expr::col(app::Column::RatingSum).add(sum_delta);
    let new_count = Expr::col(app::Column::RatingCount).add(count_delta);
    let new_avg = Expr::case(
        new_count.clone().gt(0i64),
        new_sum
            .clone()
            .cast_as(Alias::new("DOUBLE PRECISION"))
            // CockroachDB has no float / int operator and casts nothing implicitly.
            .div(new_count.clone().cast_as(Alias::new("DOUBLE PRECISION"))),
    )
    .finally(Expr::null());

    let result = app::Entity::update_many()
        .col_expr(app::Column::RatingSum, new_sum)
        .col_expr(app::Column::RatingCount, new_count)
        .col_expr(app::Column::AvgRating, new_avg.into())
        .filter(app::Column::Id.eq(app_id))
        .exec(db)
        .await?;

    if result.rows_affected == 0 {
        return Err(ApiError::NOT_FOUND);
    }

    Ok(())
}

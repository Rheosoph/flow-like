use crate::{
    entity::{comment, wasm_package},
    error::ApiError,
    middleware::jwt::AppUser,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like_types::create_id;
use sea_orm::sea_query::Expr;
use sea_orm::sea_query::ExprTrait;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel,
    QueryFilter, TransactionTrait,
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
    path = "/registry/package/{package_id}/comments",
    tag = "package-comments",
    description = "Create or update a review for a WASM package. One review per user per package.",
    params(
        ("package_id" = String, Path, description = "Package ID")
    ),
    request_body = CommentBody,
    responses(
        (status = 200, description = "Comment upserted", body = CommentResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Package not found")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "PUT /registry/package/{package_id}/comments",
    skip(state, user, body)
)]
pub async fn upsert_comment(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(package_id): Path<String>,
    Json(body): Json<CommentBody>,
) -> Result<Json<CommentResponse>, ApiError> {
    let sub = user
        .sub()
        .map_err(|_| ApiError::unauthorized("Authentication required"))?;

    wasm_package::Entity::find_by_id(&package_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    let txn = state.db.begin().await?;

    let existing = comment::Entity::find()
        .filter(comment::Column::UserId.eq(&sub))
        .filter(comment::Column::PackageId.eq(&package_id))
        .one(&txn)
        .await?;

    let new_rating = body.rating.clamp(1, 5);

    let comment_id = if let Some(existing) = existing {
        let id = existing.id.clone();
        let old_rating = existing.rating;
        let mut active = existing.into_active_model();
        active.text = Set(body.text);
        active.rating = Set(new_rating);
        active.updated_at = Set(chrono::Utc::now().naive_utc());
        active.update(&txn).await?;
        adjust_package_ratings(&txn, &package_id, new_rating - old_rating, 0).await?;
        id
    } else {
        let id = create_id();
        let model = comment::Model {
            id: id.clone(),
            text: body.text,
            rating: new_rating,
            user_id: sub,
            app_id: None,
            template_id: None,
            package_id: Some(package_id.clone()),
            created_at: chrono::Utc::now().naive_utc(),
            updated_at: chrono::Utc::now().naive_utc(),
        };
        let mut active = comment::ActiveModel::from(model);
        active = active.reset_all();
        active.insert(&txn).await?;
        adjust_package_ratings(&txn, &package_id, new_rating, 1).await?;
        id
    };
    txn.commit().await?;

    Ok(Json(CommentResponse { comment_id }))
}

pub(super) async fn adjust_package_ratings(
    db: &impl ConnectionTrait,
    package_id: &str,
    sum_delta: i64,
    count_delta: i64,
) -> Result<(), ApiError> {
    wasm_package::Entity::update_many()
        .col_expr(
            wasm_package::Column::RatingSum,
            Expr::col(wasm_package::Column::RatingSum).add(sum_delta),
        )
        .col_expr(
            wasm_package::Column::RatingCount,
            Expr::col(wasm_package::Column::RatingCount).add(count_delta),
        )
        .filter(wasm_package::Column::Id.eq(package_id))
        .exec(db)
        .await?;

    let package = wasm_package::Entity::find_by_id(package_id)
        .one(db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    let avg = if package.rating_count > 0 {
        Some(package.rating_sum as f64 / package.rating_count as f64)
    } else {
        None
    };

    let mut active = package.into_active_model();
    active.avg_rating = Set(avg);
    active.update(db).await?;

    Ok(())
}

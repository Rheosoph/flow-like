use crate::{entity::pat, error::ApiError, middleware::jwt::AppUser, state::AppState};
use axum::{Extension, Json, extract::State};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QuerySelect};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct PatOut {
    pub name: String,
    pub created_at: i64,
    pub valid_until: Option<i64>,
    pub permissions: i64,
    pub id: String,
}

#[utoipa::path(
    get,
    path = "/user/pat",
    tag = "user",
    responses(
        (status = 200, description = "List of personal access tokens", body = Vec<PatOut>),
        (status = 401, description = "Unauthorized")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[tracing::instrument(name = "GET /user/pat", skip(state, user))]
pub async fn get_pats(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
) -> Result<Json<Vec<PatOut>>, ApiError> {
    let sub = user.sub()?;

    let pats = pat::Entity::find()
        .filter(pat::Column::UserId.eq(sub))
        .limit(1000)
        .all(&state.db)
        .await?
        .into_iter()
        .map(|pat| {
            Ok(PatOut {
                name: pat.name,
                created_at: pat.created_at.timestamp_millis(),
                valid_until: pat.valid_until.map(|dt| dt.timestamp_millis()),
                permissions: pat.permissions,
                id: pat.id,
            })
        })
        .collect::<Result<Vec<PatOut>, ApiError>>()?;

    Ok(Json(pats))
}

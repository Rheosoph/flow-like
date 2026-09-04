use crate::{
    audit_branch,
    db::{DEFAULT_WRITE_CHUNK, delete_in_batches, update_in_batches},
    ensure_permission,
    entity::{
        embedding_usage_tracking, execution_run, execution_usage_tracking, llm_usage_tracking,
        technical_user,
    },
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use sea_orm::{
    ColumnTrait, Condition, DbErr, EntityTrait, QueryFilter, QuerySelect, sea_query::Expr,
};

/// Null every usage row's reference to the key in bounded batches, so the
/// row delete never drags an unbounded `SET NULL` cascade into one
/// transaction.
async fn detach_technical_user_usage(
    state: &AppState,
    technical_user_id: &str,
) -> Result<(), DbErr> {
    let null = Expr::value(Option::<String>::None);
    update_in_batches::<llm_usage_tracking::Entity>(
        &state.db,
        state.db_dialect,
        Condition::all().add(llm_usage_tracking::Column::TechnicalUserId.eq(technical_user_id)),
        vec![(llm_usage_tracking::Column::TechnicalUserId, null.clone())],
        DEFAULT_WRITE_CHUNK,
    )
    .await?;
    update_in_batches::<embedding_usage_tracking::Entity>(
        &state.db,
        state.db_dialect,
        Condition::all()
            .add(embedding_usage_tracking::Column::TechnicalUserId.eq(technical_user_id)),
        vec![(
            embedding_usage_tracking::Column::TechnicalUserId,
            null.clone(),
        )],
        DEFAULT_WRITE_CHUNK,
    )
    .await?;
    update_in_batches::<execution_usage_tracking::Entity>(
        &state.db,
        state.db_dialect,
        Condition::all()
            .add(execution_usage_tracking::Column::TechnicalUserId.eq(technical_user_id)),
        vec![(
            execution_usage_tracking::Column::TechnicalUserId,
            null.clone(),
        )],
        DEFAULT_WRITE_CHUNK,
    )
    .await?;
    update_in_batches::<execution_run::Entity>(
        &state.db,
        state.db_dialect,
        Condition::all().add(execution_run::Column::TechnicalUserId.eq(technical_user_id)),
        vec![(execution_run::Column::TechnicalUserId, null)],
        DEFAULT_WRITE_CHUNK,
    )
    .await?;
    Ok(())
}

/// Delete every technical user matching `condition`, detaching each one's
/// usage rows first and deleting the key rows in bounded batches.
pub(crate) async fn delete_technical_users_where(
    state: &AppState,
    condition: Condition,
) -> Result<u64, DbErr> {
    let ids: Vec<String> = technical_user::Entity::find()
        .filter(condition.clone())
        .select_only()
        .column(technical_user::Column::Id)
        .into_tuple()
        .all(&state.db)
        .await?;
    for id in &ids {
        detach_technical_user_usage(state, id).await?;
    }
    let outcome = delete_in_batches::<technical_user::Entity>(
        &state.db,
        state.db_dialect,
        condition,
        DEFAULT_WRITE_CHUNK,
        None,
    )
    .await?;
    Ok(outcome.rows)
}

#[utoipa::path(
    delete,
    path = "/apps/{app_id}/api/{key_id}",
    tag = "api-keys",
    description = "Delete an API key.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("key_id" = String, Path, description = "API key ID")
    ),
    responses(
        (status = 200, description = "API key deleted", body = ()),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "DELETE /apps/{app_id}/api/{key_id}", skip(state, user))]
pub async fn delete_api_key(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, key_id)): Path<(String, String)>,
) -> Result<Json<()>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::Admin);

    technical_user::Entity::find_by_id(&key_id)
        .filter(technical_user::Column::AppId.eq(&app_id))
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    delete_technical_users_where(
        &state,
        Condition::all()
            .add(technical_user::Column::Id.eq(&key_id))
            .add(technical_user::Column::AppId.eq(&app_id)),
    )
    .await?;

    // Invalidate any cached auth for this key
    state.auth_cache.invalidate_all();

    audit_branch!(
        state,
        user,
        app_id,
        "apikey.delete",
        "ApiKey",
        key_id,
        "API key deleted"
    );
    Ok(Json(()))
}

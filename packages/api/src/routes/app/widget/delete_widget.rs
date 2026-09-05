use crate::{
    audit_branch, ensure_permission, entity::widget, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use sea_orm::sea_query::ExprTrait;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

#[utoipa::path(
    delete,
    path = "/apps/{app_id}/widgets/{widget_id}",
    tag = "widgets",
    description = "Delete a widget.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("widget_id" = String, Path, description = "Widget ID")
    ),
    responses(
        (status = 200, description = "Widget deleted", body = ()),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "DELETE /apps/{app_id}/widgets/{widget_id}", skip(state, user))]
pub async fn delete_widget(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, widget_id)): Path<(String, String)>,
) -> Result<Json<()>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::WriteWidgets);

    let mut app = state
        .scoped_app(
            &user.sub()?,
            &app_id,
            &state,
            crate::credentials::CredentialsAccess::EditApp,
        )
        .await?;

    // Delete from bucket storage
    app.delete_widget(&widget_id).await?;

    app.widget_ids.retain(|id| id != &widget_id);
    app.save().await?;

    // Delete from DB (cascades to meta)
    widget::Entity::delete_many()
        .filter(
            widget::Column::AppId
                .eq(app_id.clone())
                .and(widget::Column::Id.eq(widget_id.clone())),
        )
        .exec(&state.db)
        .await?;

    audit_branch!(
        state,
        user,
        app_id,
        "widget.delete",
        "Widget",
        widget_id,
        "Deleted a widget"
    );

    Ok(Json(()))
}

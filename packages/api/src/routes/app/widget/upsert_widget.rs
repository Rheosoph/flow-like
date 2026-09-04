use crate::{
    ensure_permission,
    entity::{meta, widget},
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like::a2ui::widget::Widget;
use flow_like_types::create_id;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Deserialize, Serialize, ToSchema)]
pub struct WidgetUpsert {
    #[schema(value_type = Object)]
    pub widget: Widget,
}

/// Mirrors the widget's name onto its English meta row so a rename made through
/// the widget record alone still shows up in every listing that reads names from
/// metadata. `PUT /apps/{app_id}/meta` owns the remaining descriptive fields, so
/// `seed_description` is only written when the row has to be created — mirroring
/// it on every save would let a builder autosave wipe a description entered in
/// the app store sheet.
async fn upsert_widget_meta(
    db: &DatabaseConnection,
    widget_id: &str,
    name: &str,
    seed_description: Option<&str>,
) -> Result<(), ApiError> {
    let now = chrono::Utc::now().fixed_offset();
    let existing = meta::Entity::find()
        .filter(meta::Column::WidgetId.eq(widget_id))
        .filter(meta::Column::Lang.eq("en"))
        .one(db)
        .await?;

    if let Some(existing) = existing {
        if existing.name == name {
            return Ok(());
        }
        let mut model: meta::ActiveModel = existing.into();
        model.name = Set(name.to_string());
        model.updated_at = Set(now);
        model.update(db).await?;
        return Ok(());
    }

    meta::ActiveModel {
        id: Set(create_id()),
        lang: Set("en".to_string()),
        name: Set(name.to_string()),
        description: Set(seed_description.map(str::to_string)),
        widget_id: Set(Some(widget_id.to_string())),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(db)
    .await?;

    Ok(())
}

#[utoipa::path(
    put,
    path = "/apps/{app_id}/widgets/{widget_id}",
    tag = "widgets",
    description = "Create or update a widget.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("widget_id" = String, Path, description = "Widget ID")
    ),
    request_body = WidgetUpsert,
    responses(
        (status = 200, description = "Widget saved", body = String, content_type = "application/json"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "PUT /apps/{app_id}/widgets/{widget_id}",
    skip(state, user, widget_data)
)]
pub async fn upsert_widget(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, widget_id)): Path<(String, String)>,
    Json(widget_data): Json<WidgetUpsert>,
) -> Result<Json<Widget>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::WriteWidgets);

    if widget_id.is_empty() || app_id.is_empty() {
        return Err(ApiError::FORBIDDEN);
    }

    let mut app = state
        .scoped_app(
            &user.sub()?,
            &app_id,
            &state,
            crate::credentials::CredentialsAccess::EditApp,
        )
        .await?;

    let mut widget = widget_data.widget;
    widget.id = widget_id.clone();

    app.save_widget(&widget).await?;

    // Check if widget exists in DB
    let existing = widget::Entity::find_by_id(&widget_id)
        .filter(widget::Column::AppId.eq(&app_id))
        .one(&state.db)
        .await?;

    if existing.is_none() {
        // Create new widget record in DB
        let new_widget = widget::ActiveModel {
            id: Set(widget_id.clone()),
            app_id: Set(app_id.to_string()),
            version: Set(widget.version.map(|v| format!("{}.{}.{}", v.0, v.1, v.2))),
            created_at: Set(chrono::Utc::now().fixed_offset()),
            updated_at: Set(chrono::Utc::now().fixed_offset()),
        };

        widget::Entity::insert(new_widget)
            .exec_with_returning(&state.db)
            .await?;

        upsert_widget_meta(
            &state.db,
            &widget_id,
            &widget.name,
            widget.description.as_deref(),
        )
        .await?;

        if !app.widget_ids.contains(&widget_id) {
            app.widget_ids.push(widget_id);
            app.save().await?;
        }
    } else {
        // Update existing widget record
        let update_widget = widget::ActiveModel {
            id: Set(widget_id.clone()),
            app_id: Set(app_id.to_string()),
            version: Set(widget.version.map(|v| format!("{}.{}.{}", v.0, v.1, v.2))),
            updated_at: Set(chrono::Utc::now().fixed_offset()),
            ..Default::default()
        };

        update_widget.update(&state.db).await?;

        upsert_widget_meta(
            &state.db,
            &widget_id,
            &widget.name,
            widget.description.as_deref(),
        )
        .await?;
    }

    Ok(Json(widget))
}

use crate::{
    audit_branch,
    ensure_permission,
    entity::{meta, template},
    error::ApiError,
    middleware::jwt::{AppPermissionResponse, AppUser},
    permission::role_permission::RolePermissions,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like::flow::board::VersionType;
use flow_like_types::create_id;
use sea_orm::{ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Deserialize, Serialize, ToSchema)]
pub struct TemplateUpsert {
    pub changelog: Option<String>,
    #[schema(value_type = Option<String>)]
    pub version_type: Option<VersionType>,
    pub board_id: String,
    pub board_version: Option<(u32, u32, u32)>,
}

async fn create_template(
    user: AppUser,
    state: AppState,
    permission: &AppPermissionResponse,
    app_id: &str,
    template_data: &TemplateUpsert,
) -> Result<(String, (u32, u32, u32)), ApiError> {
    if !permission.has_permission(RolePermissions::ReadBoards) {
        return Err(ApiError::FORBIDDEN);
    }
    let template_id = create_id();
    let sub = user.sub()?;
    let mut app = state
        .scoped_app(
            &sub,
            app_id,
            &state,
            crate::credentials::CredentialsAccess::EditApp,
        )
        .await?;
    let board_id = template_data.board_id.clone();
    let (template_id, version) = app
        .upsert_template(
            Some(template_id.clone()),
            template_data
                .version_type
                .clone()
                .unwrap_or(VersionType::Minor),
            board_id,
            template_data.board_version,
        )
        .await?;
    let new_template = template::ActiveModel {
        id: Set(template_id.clone()),
        app_id: Set(app_id.to_string()),
        version: Set(Some(format!("{}.{}.{}", version.0, version.1, version.2))),
        changelog: Set(template_data.changelog.clone()),
        created_at: Set(chrono::Utc::now().naive_utc()),
        updated_at: Set(chrono::Utc::now().naive_utc()),
        ..Default::default()
    };

    template::Entity::insert(new_template)
        .exec_with_returning(&state.db)
        .await?;

    let meta = meta::ActiveModel {
        id: Set(create_id()),
        lang: Set("en".to_string()),
        name: Set("New Template".to_string()),
        template_id: Set(Some(template_id.clone())),
        created_at: Set(chrono::Utc::now().naive_utc()),
        updated_at: Set(chrono::Utc::now().naive_utc()),
        ..Default::default()
    };

    meta::Entity::insert(meta).exec(&state.db).await?;
    Ok((template_id, version))
}

#[tracing::instrument(
    name = "PUT /apps/{app_id}/templates/{template_id}",
    skip(state, user, template_data)
)]
#[utoipa::path(
    put,
    path = "/apps/{app_id}/templates/{template_id}",
    tag = "templates",
    description = "Create or update a template.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("template_id" = String, Path, description = "Template ID")
    ),
    request_body = TemplateUpsert,
    responses(
        (status = 200, description = "Template saved", body = String, content_type = "application/json"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
pub async fn upsert_template(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, template_id)): Path<(String, String)>,
    Json(template_data): Json<TemplateUpsert>,
) -> Result<Json<(String, (u32, u32, u32))>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::WriteTemplates);

    if template_id.is_empty() || app_id.is_empty() {
        return Err(ApiError::FORBIDDEN);
    }

    let template = template::Entity::find()
        .filter(
            template::Column::AppId
                .eq(app_id.clone())
                .and(template::Column::Id.eq(template_id.clone())),
        )
        .one(&state.db)
        .await?;

    let mut template: template::ActiveModel = match template {
        Some(t) => t.into(),
        None => {
            let new_template =
                create_template(user, state, &permission, &app_id, &template_data).await?;

            return Ok(Json(new_template));
        }
    };

    if let Some(changelog) = template_data.changelog {
        template.changelog = Set(Some(changelog));
    }

    let version_type = template_data.version_type.unwrap_or(VersionType::Patch);

    if !permission.has_permission(RolePermissions::ReadBoards) {
        return Err(ApiError::FORBIDDEN);
    }

    // Let´s create a new template version
    let sub = user.sub()?;
    let mut app = state
        .scoped_app(
            &sub,
            &app_id,
            &state,
            crate::credentials::CredentialsAccess::EditApp,
        )
        .await?;
    let app_upsert = app
        .upsert_template(
            Some(template_id),
            version_type,
            template_data.board_id,
            template_data.board_version,
        )
        .await?;
    template.version = Set(Some(format!(
        "{}.{}.{}",
        app_upsert.1.0, app_upsert.1.1, app_upsert.1.2
    )));

    template.updated_at = Set(chrono::Utc::now().naive_utc());
    template.update(&state.db).await?;

    audit_branch!(state, user, app_id, "template.update", "Template", app_upsert.0, "Template updated");
    Ok(Json(app_upsert))
}

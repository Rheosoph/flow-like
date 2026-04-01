use crate::{
    audit,
    error::ApiError,
    mail::{EmailMessage, templates::solution_log_added},
    middleware::jwt::AppUser,
    permission::global_permission::GlobalPermission,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like_types::{anyhow, create_id};
use sea_orm::{ActiveModelTrait, ActiveValue::Set, EntityTrait};
use serde::{Deserialize, Serialize};

#[derive(Clone, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AddLogBody {
    pub action: String,
    pub details: Option<String>,
    #[serde(default)]
    pub notify_customer: bool,
}

#[derive(Clone, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AddLogResponse {
    pub success: bool,
    pub log_id: String,
}

#[tracing::instrument(
    name = "POST /admin/solutions/{solution_id}/logs",
    skip(state, user, body)
)]
pub async fn add_solution_log(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(solution_id): Path<String>,
    Json(body): Json<AddLogBody>,
) -> Result<Json<AddLogResponse>, ApiError> {
    use crate::entity::{solution_log, solution_request};

    user.check_global_permission(&state, GlobalPermission::WriteSolutions)
        .await?;

    let solution = solution_request::Entity::find_by_id(&solution_id)
        .one(&state.db)
        .await?
        .ok_or_else(|| anyhow!("Solution request not found"))?;

    let log_id = create_id();
    let actor = user.sub().ok();

    let new_log = solution_log::ActiveModel {
        id: Set(log_id.clone()),
        solution_id: Set(solution_id.clone()),
        action: Set(body.action.clone()),
        details: Set(body.details.clone()),
        actor: Set(actor),
        created_at: Set(chrono::Utc::now().naive_utc()),
    };

    new_log.insert(&state.db).await?;

    if body.notify_customer
        && let Some(mail_client) = &state.mail_client
    {
        let frontend_url = std::env::var("FRONTEND_URL").unwrap_or_else(|_| {
            format!(
                "https://{}",
                state
                    .platform_config
                    .web
                    .clone()
                    .unwrap_or_else(|| state.platform_config.domain.clone())
            )
        });
        let tracking_url = format!("{}/solutions/track/{}", frontend_url, solution_id);
        let (html, text) = solution_log_added(
            &solution.name,
            &solution_id,
            &body.action,
            body.details.as_deref(),
            &tracking_url,
        );

        let email = EmailMessage {
            to: solution.email.clone(),
            subject: format!("Solution Update: {}", body.action),
            body_html: Some(html),
            body_text: Some(text),
        };

        if let Err(e) = mail_client.send(email).await {
            tracing::warn!(error = %e, "Failed to send log notification email");
        }
    }

    tracing::info!(
        solution_id = %solution_id,
        log_id = %log_id,
        "Solution log added by admin"
    );

    audit!(
        state,
        user,
        "admin.solution.log",
        "solution_log",
        log_id,
        format!("Solution log added: {}", body.action)
    );
    Ok(Json(AddLogResponse {
        success: true,
        log_id,
    }))
}

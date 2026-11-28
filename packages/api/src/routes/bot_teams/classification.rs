//! People's classification per org, set by the admins of the org's top-level team.

use super::{
    access::{active, admin_team, org_of, visible_team},
    lock_scope,
    model::ClassificationRequest,
};
use crate::{
    audit,
    entity::{bot_org_classification, bot_team, bot_team_person},
    error::ApiError,
    middleware::jwt::AppUser,
    routes::inbox::now_millis,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::{DateTime, FixedOffset};
use sea_orm::{
    ActiveValue::Set, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, sea_query::OnConflict,
};

async fn org_people<C: ConnectionTrait>(
    db: &C,
    root: &bot_team::Model,
) -> Result<Vec<String>, ApiError> {
    let org = org_of(db, root).await?;
    let mut people: Vec<String> = bot_team_person::Entity::find()
        .filter(bot_team_person::Column::TeamId.is_in(org.ids()))
        .filter(bot_team_person::Column::Status.eq(active()))
        .all(db)
        .await?
        .into_iter()
        .map(|row| row.user_id)
        .collect();
    people.sort();
    people.dedup();
    Ok(people)
}

/// Deletes the classifications of people who are no longer active in any team of the org under
/// `org_id`, or of the whole org when `org_id` is no longer a top-level team.
pub(crate) async fn prune_classifications<C: ConnectionTrait>(
    db: &C,
    org_id: &str,
) -> Result<(), ApiError> {
    let rows = bot_org_classification::Entity::delete_many()
        .filter(bot_org_classification::Column::OrgId.eq(org_id));
    let root = bot_team::Entity::find_by_id(org_id).one(db).await?;
    let Some(root) = root.filter(|root| root.parent_id.is_none()) else {
        rows.exec(db).await?;
        return Ok(());
    };
    let insiders = org_people(db, &root).await?;
    rows.filter(bot_org_classification::Column::UserId.is_not_in(insiders))
        .exec(db)
        .await?;
    Ok(())
}

/// Deletes every classification keyed by the team, which stopped being a top-level team.
pub(crate) async fn drop_classifications<C: ConnectionTrait>(
    db: &C,
    org_id: &str,
) -> Result<(), ApiError> {
    bot_org_classification::Entity::delete_many()
        .filter(bot_org_classification::Column::OrgId.eq(org_id))
        .exec(db)
        .await?;
    Ok(())
}

pub(crate) async fn set_classification_in<C: ConnectionTrait>(
    db: &C,
    sub: &str,
    team_id: &str,
    user_id: &str,
    request: &ClassificationRequest,
    now: DateTime<FixedOffset>,
) -> Result<(), ApiError> {
    let (team, org) = visible_team(db, sub, team_id).await?;
    if team.parent_id.is_some() {
        return Err(ApiError::bad_request(format!(
            "Classifications belong to an org's top-level team; '{}' sits in '{}'",
            team.name, org.root.name
        )));
    }
    admin_team(db, sub, team_id).await?;
    if !org_people(db, &org.root)
        .await?
        .iter()
        .any(|id| id == user_id)
    {
        return Err(ApiError::not_found(format!(
            "'{user_id}' is not a person of '{}'",
            org.root.name
        )));
    }
    let Some(level) = request.classification else {
        bot_org_classification::Entity::delete_many()
            .filter(bot_org_classification::Column::OrgId.eq(&team.id))
            .filter(bot_org_classification::Column::UserId.eq(user_id))
            .exec(db)
            .await?;
        return Ok(());
    };
    let row = bot_org_classification::ActiveModel {
        org_id: Set(team.id.clone()),
        user_id: Set(user_id.to_string()),
        classification: Set(level.as_str().to_string()),
        set_by: Set(sub.to_string()),
        updated_at: Set(now),
    };
    bot_org_classification::Entity::insert(row)
        .on_conflict(
            OnConflict::columns([
                bot_org_classification::Column::OrgId,
                bot_org_classification::Column::UserId,
            ])
            .update_columns([
                bot_org_classification::Column::Classification,
                bot_org_classification::Column::SetBy,
                bot_org_classification::Column::UpdatedAt,
            ])
            .to_owned(),
        )
        .exec_without_returning(db)
        .await?;
    Ok(())
}

#[utoipa::path(
    put,
    path = "/bot-teams/{team_id}/classifications/{user_id}",
    tag = "Bot teams",
    params(
        ("team_id" = String, Path, description = "The organization's top-level team"),
        ("user_id" = String, Path, description = "The person")
    ),
    request_body = ClassificationRequest,
    responses(
        (status = 204, description = "The person's classification in this organization is set; null makes them internal again. Bots above it stay silent where they are."),
        (status = 400, description = "The team is not a top-level team; classifications belong to the organization's top-level team"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "TEAM_FORBIDDEN: only an admin of the top-level team classifies people"),
        (status = 404, description = "TEAM_NOT_FOUND, or the person is not in the organization")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "PUT /bot-teams/{team_id}/classifications/{user_id}",
    skip(state, user, body)
)]
pub async fn set_classification(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((team_id, user_id)): Path<(String, String)>,
    Json(body): Json<ClassificationRequest>,
) -> Result<StatusCode, ApiError> {
    let sub = user.sub()?;
    state
        .transaction(|txn| {
            let (sub, team_id, user_id, body) =
                (sub.clone(), team_id.clone(), user_id.clone(), body.clone());
            Box::pin(async move {
                lock_scope(txn, &[team_id.as_str()], &[]).await?;
                set_classification_in(txn, &sub, &team_id, &user_id, &body, now_millis()).await
            })
        })
        .await?;
    audit!(
        state,
        user,
        "bot.team.classification.set",
        "BotTeam",
        team_id,
        serde_json::json!({
            "user_id": user_id,
            "classification": body.classification.map(|level| level.as_str()),
        })
    );
    Ok(StatusCode::NO_CONTENT)
}

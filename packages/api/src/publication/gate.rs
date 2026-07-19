use std::collections::HashMap;

use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};

use crate::{
    entity::{
        ai_act_assessment, app_group_member,
        sea_orm_active_enums::{AiActAssessmentStatus, AppGroupMemberStatus, Visibility},
    },
    error::ApiError,
    state::AppState,
};

/// Whether a visibility makes the entity reachable by people outside its team.
pub fn is_public_target(visibility: &Visibility) -> bool {
    matches!(
        visibility,
        Visibility::Public | Visibility::PublicRequestAccess
    )
}

/// EU AI Act gate for a single app. Returns the id of the assessment to bind to
/// the review, or an error explaining what the owner still has to do.
pub async fn require_app_assessment(
    state: &AppState,
    app_id: &str,
) -> Result<Option<String>, ApiError> {
    if !state.platform_config.features.ai_act {
        return Ok(None);
    }

    let assessment = ai_act_assessment::Entity::find()
        .filter(ai_act_assessment::Column::AppId.eq(app_id))
        .order_by_desc(ai_act_assessment::Column::Version)
        .one(&state.db)
        .await?;

    match assessment {
        None => Err(ApiError::bad_request(
            "An EU AI Act assessment must be completed before publishing this app.".to_string(),
        )),
        Some(a) if a.status == AiActAssessmentStatus::Blocked => Err(ApiError::bad_request(
            "This app declares a prohibited AI practice and cannot be published.".to_string(),
        )),
        Some(a) if a.status == AiActAssessmentStatus::Draft => Err(ApiError::bad_request(
            "The EU AI Act assessment must be submitted before publishing this app.".to_string(),
        )),
        Some(a) => Ok(Some(a.id)),
    }
}

/// One member app's AI Act standing, for the suite review panel.
#[derive(Debug, Clone)]
pub struct MemberAssessment {
    pub app_id: String,
    /// `None` when the app has never started an assessment.
    pub status: Option<AiActAssessmentStatus>,
}

impl MemberAssessment {
    pub fn is_clear(&self) -> bool {
        matches!(
            self.status,
            Some(AiActAssessmentStatus::Submitted)
                | Some(AiActAssessmentStatus::Approved)
                | Some(AiActAssessmentStatus::Rejected)
        )
    }

    pub fn blocks_publication(&self) -> bool {
        !self.is_clear()
    }
}

/// Latest AI Act assessment status per ACTIVE member app of a suite.
///
/// A suite cannot own an assessment (`AiActAssessment.appId` is NOT NULL and
/// unique per version), so its risk profile is the union of its members'.
pub async fn group_member_assessments(
    state: &AppState,
    group_id: &str,
) -> Result<Vec<MemberAssessment>, ApiError> {
    let member_app_ids: Vec<String> = app_group_member::Entity::find()
        .filter(app_group_member::Column::GroupId.eq(group_id))
        .filter(app_group_member::Column::Status.eq(AppGroupMemberStatus::Active))
        .all(&state.db)
        .await?
        .into_iter()
        .map(|m| m.app_id)
        .collect();

    if member_app_ids.is_empty() {
        return Ok(vec![]);
    }

    // One query for every member; keep the highest version per app.
    let mut latest: HashMap<String, ai_act_assessment::Model> = HashMap::new();
    for assessment in ai_act_assessment::Entity::find()
        .filter(ai_act_assessment::Column::AppId.is_in(member_app_ids.clone()))
        .all(&state.db)
        .await?
    {
        match latest.get(&assessment.app_id) {
            Some(existing) if existing.version >= assessment.version => {}
            _ => {
                latest.insert(assessment.app_id.clone(), assessment);
            }
        }
    }

    Ok(member_app_ids
        .into_iter()
        .map(|app_id| MemberAssessment {
            status: latest.get(&app_id).map(|a| a.status.clone()),
            app_id,
        })
        .collect())
}

/// EU AI Act gate for a suite: every active member app must have a submitted,
/// non-blocked assessment. Suites bind no assessment id of their own.
pub async fn require_group_assessments(state: &AppState, group_id: &str) -> Result<(), ApiError> {
    if !state.platform_config.features.ai_act {
        return Ok(());
    }

    let assessments = group_member_assessments(state, group_id).await?;
    if assessments.is_empty() {
        return Err(ApiError::bad_request(
            "A suite needs at least one member app before it can be published.".to_string(),
        ));
    }

    if let Some(blocked) = assessments
        .iter()
        .find(|a| a.status == Some(AiActAssessmentStatus::Blocked))
    {
        return Err(ApiError::bad_request(format!(
            "App {} declares a prohibited AI practice, so this suite cannot be published.",
            blocked.app_id
        )));
    }

    let pending: Vec<&str> = assessments
        .iter()
        .filter(|a| a.blocks_publication())
        .map(|a| a.app_id.as_str())
        .collect();

    if !pending.is_empty() {
        return Err(ApiError::bad_request(format!(
            "Every app in a suite needs a submitted EU AI Act assessment before the suite can be published. Still outstanding: {}.",
            pending.join(", ")
        )));
    }

    Ok(())
}

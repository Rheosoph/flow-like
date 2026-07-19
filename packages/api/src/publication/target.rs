use sea_orm::ActiveValue::{NotSet, Set};

use crate::{entity::publication_request, error::ApiError};

/// What a [`publication_request`] is reviewing.
///
/// `PublicationRequest` carries a nullable `appId` and a nullable `groupId`;
/// exactly one must be present. Postgres cannot hold that invariant for us
/// (Prisma's `db push` has no way to emit a `CHECK` constraint), so this type
/// is the only sanctioned way to read or write the target. Constructing rows
/// through [`PublicationTarget::apply`] guarantees the other column is cleared,
/// and reading through [`PublicationTarget::from_model`] turns a malformed row
/// into a loud 500 instead of a silently-skipped review.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublicationTarget {
    App(String),
    Group(String),
}

impl PublicationTarget {
    pub fn from_model(model: &publication_request::Model) -> Result<Self, ApiError> {
        match (&model.app_id, &model.group_id) {
            (Some(app_id), None) => Ok(Self::App(app_id.clone())),
            (None, Some(group_id)) => Ok(Self::Group(group_id.clone())),
            _ => Err(ApiError::internal_error(flow_like_types::anyhow!(
                "Publication request {} targets neither exactly one app nor one suite",
                model.id
            ))),
        }
    }

    /// Writes this target onto an active model, clearing the other column.
    pub fn apply(&self, model: &mut publication_request::ActiveModel) {
        match self {
            Self::App(app_id) => {
                model.app_id = Set(Some(app_id.clone()));
                model.group_id = Set(None);
            }
            Self::Group(group_id) => {
                model.app_id = Set(None);
                model.group_id = Set(Some(group_id.clone()));
            }
        }
    }

    pub fn app_id(&self) -> Option<&str> {
        match self {
            Self::App(id) => Some(id),
            Self::Group(_) => None,
        }
    }

    pub fn group_id(&self) -> Option<&str> {
        match self {
            Self::App(_) => None,
            Self::Group(id) => Some(id),
        }
    }

    pub fn is_group(&self) -> bool {
        matches!(self, Self::Group(_))
    }

    /// Audit chains are app-scoped, so a suite chains on its anchor app.
    pub fn audit_chain_id(&self, owner_app_id: &str) -> String {
        match self {
            Self::App(id) => id.clone(),
            Self::Group(_) => owner_app_id.to_string(),
        }
    }

    /// `"app"` / `"suite"` — used in user-facing copy and API payloads.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::App(_) => "app",
            Self::Group(_) => "suite",
        }
    }
}

/// Builds the active model for a brand-new pending request against `target`.
pub fn new_request(
    id: String,
    target: &PublicationTarget,
    target_visibility: crate::entity::sea_orm_active_enums::Visibility,
    ai_act_assessment_id: Option<String>,
    now: chrono::NaiveDateTime,
) -> publication_request::ActiveModel {
    let mut model = publication_request::ActiveModel {
        id: Set(id),
        app_id: NotSet,
        group_id: NotSet,
        target_visibility: Set(target_visibility),
        status: Set(crate::entity::sea_orm_active_enums::PublicationRequestStatus::Pending),
        approver_id: Set(None),
        ai_act_assessment_id: Set(ai_act_assessment_id),
        created_at: Set(now),
        updated_at: Set(now),
    };
    target.apply(&mut model);
    model
}

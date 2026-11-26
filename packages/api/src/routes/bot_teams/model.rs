use flow_like_types::{bot_effective::BotRoomLimits, bot_team::TeamRole};
use serde::{Deserialize, Deserializer, Serialize};
use utoipa::ToSchema;

/// Tells a field sent as `null` from a field left out.
fn present<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateTeamRequest {
    pub name: String,
    #[serde(default)]
    pub purpose: Option<String>,
    /// The team to create this one in; you must be its admin.
    #[serde(default)]
    pub parent_id: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
pub struct PatchTeamRequest {
    #[serde(default)]
    pub name: Option<String>,
    /// `null` clears the purpose.
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>)]
    pub purpose: Option<Option<String>>,
    /// `null` makes the team a top-level team.
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>)]
    pub parent_id: Option<Option<String>>,
    /// `null` clears the lead.
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>)]
    pub lead_bot_id: Option<Option<String>>,
    #[serde(default)]
    pub direct_asks: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct AddPeopleRequest {
    pub user_ids: Vec<String>,
    /// `member` (default) or `admin`.
    #[serde(default)]
    #[schema(value_type = Option<String>)]
    pub role: Option<TeamRole>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct SetRoleRequest {
    /// `member` or `admin`.
    #[schema(value_type = String)]
    pub role: TeamRole,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct AddBotRequest {
    pub bot_id: String,
    /// What the bot may do in the team room, on top of its own restrictions.
    #[serde(default)]
    #[schema(value_type = Option<Object>)]
    pub limits: Option<BotRoomLimits>,
}

#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
#[serde(default)]
pub struct AcceptBotRequest {
    /// What the bot may do in the team room, on top of its own restrictions.
    #[schema(value_type = Option<Object>)]
    pub limits: Option<BotRoomLimits>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct DirectAgreementRequest {
    pub agreed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BriefingResponse {
    pub bot_id: String,
    /// What the bot reads about its team and organization before every turn.
    pub text: String,
}

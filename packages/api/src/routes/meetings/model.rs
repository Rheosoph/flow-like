use crate::entity::{meeting, meeting_message, meeting_participant};
use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Deserialize, ToSchema)]
pub struct CreateMeeting {
    pub conversation_id: String,
    pub title: String,
    pub starts_at: DateTime<FixedOffset>,
    pub ends_at: DateTime<FixedOffset>,
    #[serde(default = "enabled")]
    pub guests_allowed: bool,
}

fn enabled() -> bool {
    true
}

#[derive(Deserialize)]
pub struct ListQuery {
    pub conversation_id: String,
}

#[derive(Clone, Deserialize, Default)]
pub struct PreviewRequest {
    pub invite_token: Option<String>,
}

#[derive(Clone, Deserialize)]
pub struct GuestJoin {
    pub invite_token: String,
    pub display_name: String,
}

#[derive(Clone, Deserialize)]
pub struct SendMessage {
    pub body: String,
}

#[derive(Clone, Serialize, ToSchema)]
pub struct Meeting {
    pub id: String,
    pub conversation_id: String,
    pub host_id: String,
    pub title: String,
    pub starts_at: DateTime<FixedOffset>,
    pub ends_at: DateTime<FixedOffset>,
    pub ended_at: Option<DateTime<FixedOffset>>,
    pub guests_allowed: bool,
}

impl Meeting {
    pub(super) fn of(row: &meeting::Model, now: DateTime<FixedOffset>) -> Self {
        Self {
            id: row.id.clone(),
            conversation_id: row.conversation_id.clone(),
            host_id: row.host_id.clone(),
            title: row.title.clone(),
            starts_at: row.starts_at,
            ends_at: row.ends_at,
            ended_at: row
                .ended_at
                .or_else(|| (now >= row.ends_at).then_some(row.ends_at)),
            guests_allowed: row.guests_allowed,
        }
    }
}

#[derive(Serialize)]
pub struct MeetingPreview {
    pub id: String,
    pub title: String,
    pub starts_at: DateTime<FixedOffset>,
    pub ends_at: DateTime<FixedOffset>,
    pub ended_at: Option<DateTime<FixedOffset>>,
    pub guests_allowed: bool,
}

impl From<Meeting> for MeetingPreview {
    fn from(value: Meeting) -> Self {
        Self {
            id: value.id,
            title: value.title,
            starts_at: value.starts_at,
            ends_at: value.ends_at,
            ended_at: value.ended_at,
            guests_allowed: value.guests_allowed,
        }
    }
}

#[derive(Serialize)]
pub struct MeetingList {
    pub meetings: Vec<Meeting>,
}

#[derive(Serialize)]
pub struct CreatedMeeting {
    pub meeting: Meeting,
    pub invite_token: String,
}

#[derive(Serialize)]
pub struct InviteToken {
    pub invite_token: String,
}

#[derive(Serialize)]
pub struct Joined {
    pub session_token: String,
    pub participant_id: String,
}

#[derive(Serialize, Clone, ToSchema)]
pub struct Participant {
    pub id: String,
    pub display_name: String,
    pub guest: bool,
    pub status: String,
    pub is_host: bool,
}

impl Participant {
    pub(super) fn of(row: &meeting_participant::Model, host: &str) -> Self {
        Self {
            id: row.id.clone(),
            display_name: row.display_name.clone(),
            guest: row.user_id.is_none(),
            status: row.status.to_ascii_lowercase(),
            is_host: row.user_id.as_deref() == Some(host),
        }
    }
}

#[derive(Serialize, Clone, ToSchema)]
pub struct Message {
    pub id: String,
    pub participant_id: String,
    pub display_name: String,
    pub body: String,
    pub created_at: DateTime<FixedOffset>,
}

impl From<meeting_message::Model> for Message {
    fn from(row: meeting_message::Model) -> Self {
        Self {
            id: row.id,
            participant_id: row.participant_id,
            display_name: row.display_name,
            body: row.body,
            created_at: row.created_at,
        }
    }
}

#[derive(Serialize)]
pub struct Session {
    pub meeting: Meeting,
    pub me: Participant,
    pub participants: Vec<Participant>,
    pub messages: Vec<Message>,
    pub server_time: DateTime<FixedOffset>,
    pub room_epoch: i32,
}

#[derive(Serialize)]
pub struct Realtime {
    #[serde(flatten)]
    pub access: crate::routes::inbox::presence::ConversationRealtimeParams,
    /// Logical conversation claim ID. The signaling topic is `inbox:{room_id}:room-v1`.
    pub room_id: String,
    pub expires_at: i64,
    pub expires_in: i64,
    pub server_time: DateTime<FixedOffset>,
    pub signaling_servers: Vec<String>,
}

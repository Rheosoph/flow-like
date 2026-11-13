//! A bot bringing a person into the group chat its turn runs in. The bot names the person by the
//! exact username or email address it was given, never through a search. The owner approves every
//! invitation on a card that says who and why, then the person accepts or declines. The host binds
//! the room, so model arguments cannot name another conversation.

use std::sync::Arc;

use async_trait::async_trait;
use flow_like_types::Result;
use flow_like_types::bot_message::{MAX_INVITE_PERSON_CHARS, MAX_SUMMARY_CHARS, RoomInviteAsk};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

use crate::env::ToolEnv;
use crate::error::{ToolError, ToolResult, code};
use crate::outcome::ToolOutput;
use crate::spec::{Approval, ToolSpec, schema_of};

pub const ROOM_INVITE: &str = flow_like_types::bot_message::ROOM_INVITE_TOOL;

/// The hub's refusal codes for an invitation it did not send.
pub const NO_SUCH_PERSON_CODE: &str = "BOT_INVITE_NO_SUCH_PERSON";
pub const REFUSED_CODE: &str = "BOT_INVITE_REFUSED";

const DESCRIPTION: &str = "Invite a person into this group chat. `person` is their exact Flow-Like username or their exact email address, as someone here gave it to you: there is no search, so never guess or complete one. Your owner approves every invitation on a card that shows `reason`, then the person accepts or declines; they read nothing of the room before that. Only for people, in group chats.";

/// Who a bot invites: a username (a leading `@` dropped) or an email address, compared exactly
/// apart from letter case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InviteTarget {
    Username(String),
    Email(String),
}

impl InviteTarget {
    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim();
        let usable = !value.is_empty()
            && value.chars().count() <= MAX_INVITE_PERSON_CHARS
            && !value
                .chars()
                .any(|c| c.is_whitespace() || c.is_control() || c == ',');
        if !usable {
            return None;
        }
        if let Some(handle) = value.strip_prefix('@') {
            return (!handle.is_empty() && !handle.contains('@'))
                .then(|| Self::Username(handle.to_string()));
        }
        match value.rsplit_once('@') {
            None => Some(Self::Username(value.to_string())),
            Some((local, domain))
                if !local.is_empty()
                    && !local.contains('@')
                    && domain.contains('.')
                    && !domain.starts_with('.')
                    && !domain.ends_with('.') =>
            {
                Some(Self::Email(value.to_string()))
            }
            Some(_) => None,
        }
    }

    pub fn value(&self) -> &str {
        match self {
            Self::Username(value) | Self::Email(value) => value,
        }
    }

    pub fn is_email(&self) -> bool {
        matches!(self, Self::Email(_))
    }
}

/// What became of one invitation. `name` is the person's public display name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InviteOutcome {
    Invited {
        name: String,
    },
    AlreadyMember {
        name: String,
    },
    AlreadyInvited {
        name: String,
    },
    /// No account has this username or email address.
    NoSuchPerson,
    /// The person can't be invited here (a block, or the room takes no more invitations).
    NotInvitable {
        why: String,
    },
}

/// What the hub's invitation route answered with a success: `{status, name}`.
pub fn hub_reply(reply: &serde_json::Value) -> Result<InviteOutcome> {
    let name = reply
        .get("name")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("This person")
        .to_string();
    match reply.get("status").and_then(serde_json::Value::as_str) {
        Some("invited") => Ok(InviteOutcome::Invited { name }),
        Some("already_member") => Ok(InviteOutcome::AlreadyMember { name }),
        Some("already_invited") => Ok(InviteOutcome::AlreadyInvited { name }),
        other => Err(flow_like_types::anyhow!(
            "The hub answered the invitation with the unknown status {other:?}"
        )),
    }
}

/// What a refusal of the hub's invitation route means, when its code is one of
/// [`NO_SUCH_PERSON_CODE`] and [`REFUSED_CODE`].
pub fn hub_refusal(code: &str, message: &str) -> Option<InviteOutcome> {
    match code {
        NO_SUCH_PERSON_CODE => Some(InviteOutcome::NoSuchPerson),
        REFUSED_CODE => Some(InviteOutcome::NotInvitable {
            why: message.trim().to_string(),
        }),
        _ => None,
    }
}

/// How a host invites into the conversation its turn runs in: the server in process, a desktop or
/// device through the hub.
#[async_trait]
pub trait RoomInvites: Send + Sync {
    async fn invite(&self, person: &InviteTarget) -> Result<InviteOutcome>;
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RoomInviteArgs {
    /// The person's exact username (a leading @ is fine) or exact email address.
    pub person: String,
    /// One or two sentences your owner reads before approving: why this person should join.
    pub reason: String,
}

pub fn specs() -> Vec<ToolSpec> {
    vec![ToolSpec {
        name: ROOM_INVITE,
        description: DESCRIPTION,
        capability: None,
        module: "rooms",
        approval: Approval::Ask,
        approval_args: &[],
        mutating: true,
        input_schema: schema_of::<RoomInviteArgs>(),
    }]
}

/// Checks the arguments a `room_invite` card is built from.
pub fn checked_invite(args: &RoomInviteArgs) -> ToolResult<(InviteTarget, String)> {
    let target = InviteTarget::parse(&args.person).ok_or_else(|| {
        ToolError::invalid(format!(
            "'person' must be one exact username or one email address, not '{}'",
            args.person.trim()
        ))
    })?;
    let reason = args.reason.trim();
    let chars = reason.chars().count();
    if chars == 0 || chars > MAX_SUMMARY_CHARS || reason.chars().any(char::is_control) {
        return Err(ToolError::invalid(format!(
            "'reason' is one line of 1 to {MAX_SUMMARY_CHARS} characters"
        )));
    }
    Ok((target, reason.to_string()))
}

/// What an approval card shows for a `room_invite` call; `None` for invalid arguments.
pub fn invite_ask(args: &serde_json::Value) -> Option<RoomInviteAsk> {
    let args: RoomInviteArgs = serde_json::from_value(args.clone()).ok()?;
    let (target, reason) = checked_invite(&args).ok()?;
    Some(RoomInviteAsk {
        person: target.value().to_string(),
        reason,
    })
}

/// The line an approval card's title is made of.
pub fn ask_message(args: &serde_json::Value) -> String {
    let person = args
        .get("person")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .unwrap_or("?");
    format!("The bot wants to invite {person} to this group chat.")
}

fn backend(env: &dyn ToolEnv) -> ToolResult<Arc<dyn RoomInvites>> {
    env.rooms().ok_or_else(|| {
        ToolError::new(
            code::NOT_YET,
            "Inviting people works only in a group chat on a Flow-Like account; this turn runs in none",
        )
    })
}

pub async fn room_invite(env: &dyn ToolEnv, args: RoomInviteArgs) -> ToolResult<ToolOutput> {
    let rooms = backend(env)?;
    let (target, _) = checked_invite(&args)?;
    let person = target.value();
    let (status, name, message) = match rooms.invite(&target).await? {
        InviteOutcome::Invited { name } => (
            "invited",
            name,
            "Invited. They join once they accept; until then they read nothing of this room.",
        ),
        InviteOutcome::AlreadyMember { name } => {
            ("already_member", name, "They are already in this room.")
        }
        InviteOutcome::AlreadyInvited { name } => (
            "already_invited",
            name,
            "They already hold an invitation to this room and have not answered yet.",
        ),
        InviteOutcome::NoSuchPerson => {
            let kind = if target.is_email() {
                "email address"
            } else {
                "username"
            };
            return Err(ToolError::not_found(format!(
                "No Flow-Like account has the {kind} '{person}'. Ask for their exact username or email address; there is no search. Nobody was invited."
            )));
        }
        InviteOutcome::NotInvitable { why } => {
            return Err(ToolError::refused(format!(
                "'{person}' can't be invited to this room: {why}"
            )));
        }
    };
    Ok(ToolOutput::new(json!({
        "status": status,
        "person": { "name": name },
        "message": message,
    })))
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::testing::FakeEnv;

    #[derive(Default)]
    struct FakeRooms {
        asked: Mutex<Vec<InviteTarget>>,
        outcome: Mutex<Option<InviteOutcome>>,
    }

    #[async_trait]
    impl RoomInvites for FakeRooms {
        async fn invite(&self, person: &InviteTarget) -> Result<InviteOutcome> {
            self.asked.lock().unwrap().push(person.clone());
            Ok(self
                .outcome
                .lock()
                .unwrap()
                .clone()
                .unwrap_or(InviteOutcome::Invited {
                    name: "Ana Lima".into(),
                }))
        }
    }

    async fn env_with(rooms: Arc<FakeRooms>) -> FakeEnv {
        let env = FakeEnv::new().await;
        *env.rooms.lock().unwrap() = Some(rooms);
        env
    }

    #[test]
    fn a_person_is_one_username_or_one_address() {
        assert_eq!(
            InviteTarget::parse(" @ana.lima "),
            Some(InviteTarget::Username("ana.lima".into()))
        );
        assert_eq!(
            InviteTarget::parse("ana_lima-2"),
            Some(InviteTarget::Username("ana_lima-2".into()))
        );
        assert_eq!(
            InviteTarget::parse("@example.org"),
            Some(InviteTarget::Username("example.org".into()))
        );
        assert_eq!(
            InviteTarget::parse("Ana@Example.org"),
            Some(InviteTarget::Email("Ana@Example.org".into()))
        );
        for value in [
            "",
            "@",
            "Ana Lima",
            "ana,ben",
            "@ana@example.org",
            "ana@localhost",
            "ana@example.",
            "a@b@example.org",
            &"a".repeat(MAX_INVITE_PERSON_CHARS + 1),
        ] {
            assert_eq!(InviteTarget::parse(value), None, "{value:?}");
        }
    }

    #[test]
    fn hub_answers_map_to_outcomes() {
        assert_eq!(
            hub_reply(&json!({ "status": "already_invited", "name": "Ana" })).unwrap(),
            InviteOutcome::AlreadyInvited { name: "Ana".into() }
        );
        assert!(hub_reply(&json!({ "status": "maybe" })).is_err());
        assert_eq!(
            hub_refusal(NO_SUCH_PERSON_CODE, "No account"),
            Some(InviteOutcome::NoSuchPerson)
        );
        assert_eq!(
            hub_refusal(REFUSED_CODE, " the room is full "),
            Some(InviteOutcome::NotInvitable {
                why: "the room is full".into()
            })
        );
        assert_eq!(hub_refusal("NOT_FOUND", "x"), None);
    }

    #[test]
    fn it_asks_every_time_and_needs_no_capability() {
        let spec = crate::spec(ROOM_INVITE).expect("registered");
        assert_eq!(spec.approval, Approval::Ask);
        assert_eq!(spec.capability, None);
        assert!(spec.mutating);
    }

    #[test]
    fn the_card_shows_the_person_and_the_trimmed_reason() {
        assert_eq!(
            invite_ask(&json!({ "person": "@ana", "reason": "  She owns the sheet. " })),
            Some(RoomInviteAsk {
                person: "ana".into(),
                reason: "She owns the sheet.".into(),
            })
        );
        assert_eq!(
            invite_ask(&json!({ "person": "Ana Lima", "reason": "x" })),
            None
        );
        assert_eq!(invite_ask(&json!({ "person": "ana", "reason": " " })), None);
        assert_eq!(
            ask_message(&json!({ "person": "ana@example.org" })),
            "The bot wants to invite ana@example.org to this group chat."
        );
    }

    #[tokio::test]
    async fn an_invitation_names_the_person_and_says_what_happens_next() {
        let rooms = Arc::new(FakeRooms::default());
        let env = env_with(rooms.clone()).await;
        let outcome = crate::run(
            ROOM_INVITE,
            json!({ "person": "ana@example.org", "reason": "Budget owner" }),
            &env,
        )
        .await;
        assert_eq!(outcome.status, crate::Status::Ok, "{:?}", outcome.error);
        assert_eq!(outcome.result["status"], "invited");
        assert_eq!(outcome.result["person"]["name"], "Ana Lima");
        assert_eq!(
            *rooms.asked.lock().unwrap(),
            [InviteTarget::Email("ana@example.org".into())]
        );
    }

    #[tokio::test]
    async fn members_and_open_invitations_are_reported_not_failed() {
        for (outcome, status) in [
            (
                InviteOutcome::AlreadyMember { name: "Ana".into() },
                "already_member",
            ),
            (
                InviteOutcome::AlreadyInvited { name: "Ana".into() },
                "already_invited",
            ),
        ] {
            let rooms = Arc::new(FakeRooms {
                outcome: Mutex::new(Some(outcome)),
                ..Default::default()
            });
            let env = env_with(rooms).await;
            let result =
                crate::run(ROOM_INVITE, json!({ "person": "ana", "reason": "x" }), &env).await;
            assert_eq!(result.status, crate::Status::Ok, "{:?}", result.error);
            assert_eq!(result.result["status"], status);
        }
    }

    #[tokio::test]
    async fn an_unknown_person_says_there_is_no_search() {
        let rooms = Arc::new(FakeRooms {
            outcome: Mutex::new(Some(InviteOutcome::NoSuchPerson)),
            ..Default::default()
        });
        let env = env_with(rooms).await;
        let error = crate::run(ROOM_INVITE, json!({ "person": "ana", "reason": "x" }), &env)
            .await
            .error
            .expect("refused");
        assert_eq!(error.code, code::NOT_FOUND);
        assert!(
            error.message.contains("username 'ana'"),
            "{}",
            error.message
        );
        assert!(error.message.contains("no search"), "{}", error.message);
    }

    #[tokio::test]
    async fn bad_arguments_never_reach_the_backend() {
        let rooms = Arc::new(FakeRooms::default());
        let env = env_with(rooms.clone()).await;
        for args in [
            json!({ "person": "Ana Lima", "reason": "x" }),
            json!({ "person": "ana", "reason": "" }),
            json!({ "person": "ana", "reason": "x", "conversation_id": "elsewhere" }),
        ] {
            let error = crate::run(ROOM_INVITE, args, &env)
                .await
                .error
                .expect("refused");
            assert_eq!(error.code, code::INVALID_ARGS);
        }
        assert!(rooms.asked.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_host_without_rooms_says_so() {
        let env = FakeEnv::new().await;
        let error = crate::run(ROOM_INVITE, json!({ "person": "ana", "reason": "x" }), &env)
            .await
            .error
            .expect("refused");
        assert_eq!(error.code, code::NOT_YET);
    }
}

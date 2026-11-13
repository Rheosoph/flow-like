//! The desktop's `room_invite`: an account bot invites into the hub group chat its turn runs in.
//! The hub resolves the person and writes the invitation. A bot kept only on this device has no
//! rooms with other people.

use std::sync::Arc;

use async_trait::async_trait;
use flow_like_bot_tools::rooms::{
    InviteOutcome, InviteTarget, NO_SUCH_PERSON_CODE, REFUSED_CODE, RoomInvites, hub_refusal,
    hub_reply,
};
use flow_like_types::{Result, anyhow};
use reqwest::Method;
use serde_json::json;
use tauri::AppHandle;

use super::schedule::TaskApi;

struct HubRooms {
    api: Arc<dyn TaskApi>,
    bot_id: String,
    conversation_id: String,
}

/// The refusal in a desktop hub error, written as `… HTTP <status> … <CODE>: <message>`.
fn refusal_in(error: &str) -> Option<InviteOutcome> {
    [NO_SUCH_PERSON_CODE, REFUSED_CODE]
        .into_iter()
        .find_map(|code| {
            let (_, rest) = error.split_once(&format!("{code}:"))?;
            hub_refusal(code, rest)
        })
}

#[async_trait]
impl RoomInvites for HubRooms {
    async fn invite(&self, person: &InviteTarget) -> Result<InviteOutcome> {
        let path = format!(
            "bots/{}/rooms/{}/invitations",
            super::super::native_platform_tools::segment(&self.bot_id),
            super::super::native_platform_tools::segment(&self.conversation_id)
        );
        match self
            .api
            .request(
                Method::POST,
                &path,
                Some(&json!({ "person": person.value() })),
            )
            .await
        {
            Ok(reply) => hub_reply(&reply),
            Err(error) => refusal_in(&error).ok_or_else(|| anyhow!(error)),
        }
    }
}

pub(crate) async fn rooms_for(
    app: &AppHandle,
    bot_id: &str,
    conversation_id: Option<&str>,
    api: Arc<dyn TaskApi>,
) -> Option<Arc<dyn RoomInvites>> {
    let conversation_id = conversation_id?.to_owned();
    if super::held_locally(app, bot_id).await {
        return None;
    }
    Some(Arc::new(HubRooms {
        api,
        bot_id: bot_id.to_owned(),
        conversation_id,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::sync::Mutex;

    struct Recorder {
        calls: Mutex<Vec<(String, Value)>>,
        answer: Mutex<Option<std::result::Result<Value, String>>>,
    }

    #[async_trait]
    impl TaskApi for Recorder {
        async fn request(
            &self,
            _method: Method,
            path: &str,
            body: Option<&Value>,
        ) -> std::result::Result<Value, String> {
            self.calls
                .lock()
                .unwrap()
                .push((path.to_string(), body.cloned().unwrap_or(Value::Null)));
            self.answer.lock().unwrap().take().expect("one answer")
        }
    }

    fn rooms(answer: std::result::Result<Value, String>) -> (Arc<Recorder>, HubRooms) {
        let api = Arc::new(Recorder {
            calls: Mutex::default(),
            answer: Mutex::new(Some(answer)),
        });
        let rooms = HubRooms {
            api: api.clone(),
            bot_id: "bot_1".into(),
            conversation_id: "room-1".into(),
        };
        (api, rooms)
    }

    #[tokio::test]
    async fn the_hub_gets_the_person_for_the_turns_room() {
        let (api, rooms) = rooms(Ok(json!({ "status": "invited", "name": "Ana" })));
        let target = InviteTarget::parse("@ana").unwrap();
        assert_eq!(
            rooms.invite(&target).await.unwrap(),
            InviteOutcome::Invited { name: "Ana".into() }
        );
        assert_eq!(
            *api.calls.lock().unwrap(),
            [(
                "bots/bot_1/rooms/room-1/invitations".to_string(),
                json!({ "person": "ana" })
            )]
        );
    }

    #[tokio::test]
    async fn hub_refusals_become_outcomes_and_other_errors_stay_errors() {
        let target = InviteTarget::parse("ana@example.org").unwrap();
        let (_, missing) = rooms(Err(format!(
            "The hub returned HTTP 404 for this tool request. {NO_SUCH_PERSON_CODE}: No account has the email address 'ana@example.org'"
        )));
        assert_eq!(
            missing.invite(&target).await.unwrap(),
            InviteOutcome::NoSuchPerson
        );
        let (_, refused) = rooms(Err(format!(
            "The hub returned HTTP 409 for this tool request. {REFUSED_CODE}: they don't take invitations from this bot"
        )));
        assert_eq!(
            refused.invite(&target).await.unwrap(),
            InviteOutcome::NotInvitable {
                why: "they don't take invitations from this bot".into()
            }
        );
        let (_, offline) = rooms(Err("The hub request could not be completed.".into()));
        assert!(offline.invite(&target).await.is_err());
    }
}

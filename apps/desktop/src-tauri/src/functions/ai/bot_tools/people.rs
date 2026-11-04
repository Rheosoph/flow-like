//! The desktop's `PeopleBackend`: an account bot reaches its owner's contacts through the hub,
//! which holds the contacts, the bot's rooms with people, their pushes and the hourly limit. A bot
//! kept only on this device has no people.

use std::sync::Arc;

use async_trait::async_trait;
use flow_like_bot_tools::{Contact, Delivery, PeopleBackend, PersonMessage};
use flow_like_types::{Result, anyhow};
use reqwest::Method;
use serde_json::{Value, json};
use tauri::AppHandle;

use super::schedule::TaskApi;

struct HubPeople {
    api: Arc<dyn TaskApi>,
    bot_id: String,
}

fn status_of(error: &str) -> Option<u16> {
    let (_, rest) = error.split_once("HTTP ")?;
    rest.split_whitespace().next()?.parse().ok()
}

fn contacts_of(value: &Value) -> Vec<Contact> {
    value
        .get("people")
        .and_then(Value::as_array)
        .map(|people| {
            people
                .iter()
                .filter_map(|person| {
                    Some(Contact {
                        id: person.get("id")?.as_str()?.to_owned(),
                        name: person.get("name")?.as_str()?.to_owned(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn message_of(value: &Value) -> Result<PersonMessage> {
    let text = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_owned);
    Ok(PersonMessage {
        conversation_id: text("conversation_id")
            .ok_or_else(|| anyhow!("The hub sent a message receipt without a conversation"))?,
        message_id: text("message_id").unwrap_or_default(),
        name: text("name").unwrap_or_default(),
        notified: value
            .get("notified")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

impl HubPeople {
    fn path(&self, tail: &str) -> String {
        format!(
            "bots/{}/people{tail}",
            super::super::native_platform_tools::segment(&self.bot_id)
        )
    }

    async fn name_of(&self, person_id: &str) -> String {
        self.contacts(None)
            .await
            .ok()
            .and_then(|people| people.into_iter().find(|person| person.id == person_id))
            .map(|person| person.name)
            .unwrap_or_else(|| "This person".to_owned())
    }
}

#[async_trait]
impl PeopleBackend for HubPeople {
    async fn contacts(&self, query: Option<&str>) -> Result<Vec<Contact>> {
        let tail = match query {
            Some(query) => format!("?q={}", urlencoding::encode(query)),
            None => String::new(),
        };
        let reply = self
            .api
            .request(Method::GET, &self.path(&tail), None)
            .await
            .map_err(|error| anyhow!("Listing the owner's people failed: {error}"))?;
        Ok(contacts_of(&reply))
    }

    /// The hub enforces the hourly limit and answers 429 past it.
    async fn sent_since(&self, _since_ms: i64) -> Result<usize> {
        Ok(0)
    }

    async fn message(&self, person_id: &str, text: &str) -> Result<Delivery> {
        let body = json!({ "person": person_id, "text": text });
        match self
            .api
            .request(Method::POST, &self.path("/message"), Some(&body))
            .await
        {
            Ok(reply) => Ok(Delivery::Sent(message_of(&reply)?)),
            Err(error) => match status_of(&error) {
                Some(404) => Ok(Delivery::NotAContact),
                Some(409) => Ok(Delivery::Left {
                    name: self.name_of(person_id).await,
                }),
                Some(429) => Err(anyhow!(
                    "This bot already sent {} messages to people in the last hour; tell your owner what is left instead",
                    flow_like_bot_tools::people::MAX_PER_WINDOW
                )),
                _ => Err(anyhow!("Messaging '{person_id}' failed: {error}")),
            },
        }
    }
}

/// The hub's people for `bot_id`, or `None` when this device holds the bot.
pub(crate) async fn people_for(
    app: &AppHandle,
    bot_id: &str,
    api: Arc<dyn TaskApi>,
) -> Option<Arc<dyn PeopleBackend>> {
    if super::held_locally(app, bot_id).await {
        return None;
    }
    Some(Arc::new(HubPeople {
        api,
        bot_id: bot_id.to_string(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct Recorder {
        calls: Mutex<Vec<(Method, String, Value)>>,
        answers: Mutex<Vec<std::result::Result<Value, String>>>,
    }

    impl Recorder {
        fn answering(answers: Vec<std::result::Result<Value, String>>) -> Arc<Self> {
            Arc::new(Self {
                calls: Mutex::default(),
                answers: Mutex::new(answers),
            })
        }
    }

    #[async_trait]
    impl TaskApi for Recorder {
        async fn request(
            &self,
            method: Method,
            path: &str,
            body: Option<&Value>,
        ) -> std::result::Result<Value, String> {
            self.calls.lock().unwrap().push((
                method,
                path.to_string(),
                body.cloned().unwrap_or(Value::Null),
            ));
            self.answers.lock().unwrap().remove(0)
        }
    }

    fn people(api: Arc<Recorder>) -> HubPeople {
        HubPeople {
            api,
            bot_id: "bot_1".into(),
        }
    }

    fn http(status: u16) -> std::result::Result<Value, String> {
        Err(format!(
            "The hub returned HTTP {status} for this tool request."
        ))
    }

    #[tokio::test]
    async fn contacts_are_read_with_the_encoded_query() {
        let api = Recorder::answering(vec![Ok(json!({
            "people": [{ "id": "user-anna", "name": "Anna" }, { "id": 7 }]
        }))]);
        let found = people(api.clone()).contacts(Some("Anna S")).await.unwrap();
        assert_eq!(
            found,
            [Contact {
                id: "user-anna".into(),
                name: "Anna".into()
            }]
        );
        let calls = api.calls.lock().unwrap();
        assert_eq!(calls[0].0, Method::GET);
        assert_eq!(calls[0].1, "bots/bot_1/people?q=Anna%20S");
    }

    #[tokio::test]
    async fn a_sent_message_returns_its_room() {
        let api = Recorder::answering(vec![Ok(json!({
            "conversation_id": "room-1", "message_id": "bp-1", "name": "Anna", "notified": true
        }))]);
        let delivery = people(api.clone())
            .message("user-anna", "hi")
            .await
            .unwrap();
        assert_eq!(
            delivery,
            Delivery::Sent(PersonMessage {
                conversation_id: "room-1".into(),
                message_id: "bp-1".into(),
                name: "Anna".into(),
                notified: true,
            })
        );
        let calls = api.calls.lock().unwrap();
        assert_eq!(calls[0].1, "bots/bot_1/people/message");
        assert_eq!(calls[0].2, json!({ "person": "user-anna", "text": "hi" }));
    }

    #[tokio::test]
    async fn hub_refusals_become_deliveries_or_errors() {
        let stranger = Recorder::answering(vec![http(404)]);
        assert_eq!(
            people(stranger).message("user-zoe", "hi").await.unwrap(),
            Delivery::NotAContact
        );
        let left = Recorder::answering(vec![
            http(409),
            Ok(json!({ "people": [{ "id": "user-ben", "name": "Ben" }] })),
        ]);
        assert_eq!(
            people(left).message("user-ben", "hi").await.unwrap(),
            Delivery::Left { name: "Ben".into() }
        );
        let limited = Recorder::answering(vec![http(429)]);
        let error = people(limited).message("user-ben", "hi").await.unwrap_err();
        assert!(format!("{error}").contains("in the last hour"), "{error}");
        let down = Recorder::answering(vec![http(502)]);
        assert!(people(down).message("user-ben", "hi").await.is_err());
    }
}

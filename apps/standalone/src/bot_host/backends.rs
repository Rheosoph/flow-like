//! The owner-facing services of a device-hosted turn (notify, tasks, people, mail): thin
//! clients of the routes the hub opens to the turn's bot token. The hub enforces rights, rate
//! limits and mutes; these only shape requests and read the receipts.

use async_trait::async_trait;
use base64::{Engine, engine::general_purpose::STANDARD};
use flow_like_bot_host::hub::TurnHub;
use flow_like_bot_tools::{
    Contact, Delivery, NoticeReceipt, OwnerNotice, OwnerNotifier, PeopleBackend, PersonMessage,
    Scheduler,
    email::{MailBackend, MailFailure, MailFile, OutgoingMail, ReplyMail, SentMail, failure},
};
use flow_like_types::{
    Result, anyhow,
    bot_task::{BotTask, CreateBotTask},
};
use serde_json::{Value, json};
use std::sync::Arc;

/// The turn's bot-token hub and the bot it acts as.
#[derive(Clone)]
pub struct BotRoutes {
    pub hub: Arc<dyn TurnHub>,
    pub bot_id: String,
    pub conversation_id: Option<String>,
}

impl BotRoutes {
    fn path(&self, tail: &str) -> String {
        format!("bots/{}/{tail}", self.bot_id)
    }
}

pub struct HubNotifier(pub BotRoutes);

#[async_trait]
impl OwnerNotifier for HubNotifier {
    async fn sent_since(&self, _since_ms: i64) -> Result<usize> {
        Ok(0)
    }

    async fn deliver(&self, notice: OwnerNotice) -> Result<NoticeReceipt> {
        let body = json!({
            "text": notice.text,
            "title": notice.title,
            "conversation_id": self.0.conversation_id,
        });
        let answer = self
            .0
            .hub
            .write(&self.0.path("notify"), Vec::new(), body)
            .await
            .map_err(|failure| anyhow!("Messaging the owner failed: {failure}"))?;
        Ok(NoticeReceipt {
            id: answer["id"].as_str().unwrap_or_default().to_owned(),
            muted: answer["muted"].as_bool().unwrap_or(false),
            posted: answer["posted"].as_bool().unwrap_or(false),
            in_owner_chat: answer["in_owner_chat"].as_bool().unwrap_or(false),
        })
    }
}

pub struct HubScheduler(pub BotRoutes);

#[async_trait]
impl Scheduler for HubScheduler {
    async fn create(&self, request: CreateBotTask) -> Result<BotTask> {
        let body = json!({
            "kind": request.kind,
            "at": request.at,
            "cron": request.cron,
            "tz": request.tz,
            "instruction": request.instruction,
            "title": request.title,
        });
        let created = self
            .0
            .hub
            .write(&self.0.path("tasks"), Vec::new(), body)
            .await
            .map_err(|failure| anyhow!("Creating the task failed: {failure}"))?;
        serde_json::from_value(created)
            .map_err(|error| anyhow!("The hub sent an unusable task: {error}"))
    }

    async fn list(&self) -> Result<Vec<BotTask>> {
        let listed = self
            .0
            .hub
            .read(&self.0.path("tasks"), Vec::new())
            .await
            .map_err(|failure| anyhow!("Listing the tasks failed: {failure}"))?;
        serde_json::from_value(
            listed
                .get("tasks")
                .cloned()
                .unwrap_or(Value::Array(Vec::new())),
        )
        .map_err(|error| anyhow!("The hub sent unusable tasks: {error}"))
    }

    async fn cancel(&self, task_id: &str) -> Result<bool> {
        if !self.list().await?.iter().any(|task| task.id == task_id) {
            return Ok(false);
        }
        self.0
            .hub
            .delete(&self.0.path(&format!("tasks/{task_id}")), Vec::new())
            .await
            .map(|_| true)
            .map_err(|failure| anyhow!("Cancelling task '{task_id}' failed: {failure}"))
    }
}

pub struct HubPeople(pub BotRoutes);

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
        let query = query.map(|q| vec![("q", q.to_owned())]).unwrap_or_default();
        let reply = self
            .0
            .hub
            .read(&self.0.path("people"), query)
            .await
            .map_err(|failure| anyhow!("Listing the owner's people failed: {failure}"))?;
        Ok(contacts_of(&reply))
    }

    async fn sent_since(&self, _since_ms: i64) -> Result<usize> {
        Ok(0)
    }

    async fn message(&self, person_id: &str, text: &str) -> Result<Delivery> {
        let body = json!({ "person": person_id, "text": text });
        match self
            .0
            .hub
            .write(&self.0.path("people/message"), Vec::new(), body)
            .await
        {
            Ok(reply) => Ok(Delivery::Sent(message_of(&reply)?)),
            Err(failure) => match failure.status {
                Some(404) => Ok(Delivery::NotAContact),
                Some(409) => Ok(Delivery::Left {
                    name: self.name_of(person_id).await,
                }),
                Some(429) => Err(anyhow!(
                    "This bot already sent {} messages to people in the last hour; tell your owner what is left instead",
                    flow_like_bot_tools::people::MAX_PER_WINDOW
                )),
                _ => Err(anyhow!("Messaging '{person_id}' failed: {failure}")),
            },
        }
    }
}

pub struct HubMail(pub BotRoutes);

fn attachments_json(files: &[MailFile]) -> Value {
    Value::Array(
        files
            .iter()
            .map(|file| {
                json!({
                    "filename": file.filename,
                    "content_type": file.content_type,
                    "content": STANDARD.encode(&file.bytes),
                })
            })
            .collect(),
    )
}

fn failure_of(message: &str) -> MailFailure {
    let lowered = message.to_ascii_lowercase();
    let code =
        if lowered.contains("429") || lowered.contains("limit") || lowered.contains("too many") {
            failure::LIMIT
        } else if lowered.contains("404") || lowered.contains("no mail thread") {
            failure::NO_THREAD
        } else if lowered.contains("no email address") || lowered.contains("no address") {
            failure::NO_ADDRESS
        } else if lowered.contains("400") || lowered.contains("403") {
            failure::REFUSED
        } else {
            failure::UNAVAILABLE
        };
    MailFailure::new(code, message)
}

fn sent_of(value: &Value) -> std::result::Result<SentMail, MailFailure> {
    let unusable = || {
        MailFailure::new(
            failure::UNAVAILABLE,
            "The hub sent an unusable mail receipt",
        )
    };
    let text = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_owned);
    Ok(SentMail {
        message_id: text("message_id").ok_or_else(unusable)?,
        thread: text("thread").ok_or_else(unusable)?,
        from: text("from").ok_or_else(unusable)?,
        to: value
            .get("to")
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
        subject: text("subject").unwrap_or_default(),
    })
}

impl HubMail {
    async fn post(&self, tail: &str, body: Value) -> std::result::Result<SentMail, MailFailure> {
        let reply = self
            .0
            .hub
            .write(&self.0.path(&format!("mail/{tail}")), Vec::new(), body)
            .await
            .map_err(|failure| failure_of(&failure.to_string()))?;
        sent_of(&reply)
    }
}

#[async_trait]
impl MailBackend for HubMail {
    async fn send(&self, mail: OutgoingMail) -> std::result::Result<SentMail, MailFailure> {
        self.post(
            "send",
            json!({
                "to": mail.to, "cc": mail.cc, "subject": mail.subject,
                "text": mail.text, "html": mail.html,
                "attachments": attachments_json(&mail.attachments),
            }),
        )
        .await
    }

    async fn reply(&self, mail: ReplyMail) -> std::result::Result<SentMail, MailFailure> {
        self.post(
            "reply",
            json!({
                "thread": mail.thread, "text": mail.text, "html": mail.html,
                "attachments": attachments_json(&mail.attachments),
            }),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_bot_host::hub::HubFailure;
    use flow_like_types::bot_turn::BotTurn;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Calls(
        Mutex<Vec<(String, String, Value)>>,
        Mutex<Vec<Result<Value, HubFailure>>>,
    );

    impl Calls {
        fn answering(answers: Vec<Result<Value, HubFailure>>) -> Arc<Self> {
            Arc::new(Self(Mutex::default(), Mutex::new(answers)))
        }

        fn next(&self) -> Result<Value, HubFailure> {
            self.1.lock().unwrap().remove(0)
        }
    }

    #[async_trait]
    impl TurnHub for Calls {
        async fn list(&self) -> Result<Vec<BotTurn>, HubFailure> {
            Ok(Vec::new())
        }
        async fn claim(&self, _: &str, _: &str) -> Result<BotTurn, HubFailure> {
            Err(HubFailure::offline("unused"))
        }
        async fn heartbeat(&self, _: &str, _: &str) -> Result<BotTurn, HubFailure> {
            Err(HubFailure::offline("unused"))
        }
        async fn entries(
            &self,
            _: &str,
            _: &str,
        ) -> Result<Vec<flow_like_local_bots::InboxEntry>, HubFailure> {
            Ok(Vec::new())
        }
        async fn send(
            &self,
            _: &str,
            _: &str,
            _: Value,
            _: &str,
            _: &str,
        ) -> Result<(), HubFailure> {
            Ok(())
        }
        async fn desktop_bots(&self) -> Result<Vec<String>, HubFailure> {
            Ok(Vec::new())
        }
        async fn report_host(&self, _: &str, _: &[String]) -> Result<Vec<String>, HubFailure> {
            Ok(Vec::new())
        }
        async fn delete_host(&self, _: &str) -> Result<(), HubFailure> {
            Ok(())
        }
        async fn cancel_turn(&self, _: &str, _: &str) -> Result<(), HubFailure> {
            Ok(())
        }
        async fn read(
            &self,
            path: &str,
            query: Vec<(&'static str, String)>,
        ) -> Result<Value, HubFailure> {
            self.0
                .lock()
                .unwrap()
                .push(("GET".into(), path.into(), json!(query)));
            self.next()
        }
        async fn write(
            &self,
            path: &str,
            _: Vec<(&'static str, String)>,
            body: Value,
        ) -> Result<Value, HubFailure> {
            self.0
                .lock()
                .unwrap()
                .push(("POST".into(), path.into(), body));
            self.next()
        }
    }

    fn routes(calls: &Arc<Calls>) -> BotRoutes {
        BotRoutes {
            hub: calls.clone(),
            bot_id: "bot1".into(),
            conversation_id: Some("conv".into()),
        }
    }

    #[tokio::test]
    async fn a_notice_goes_to_the_bots_notify_route_with_its_conversation() {
        let calls = Calls::answering(vec![Ok(json!({ "id": "m1", "posted": true }))]);
        let receipt = HubNotifier(routes(&calls))
            .deliver(OwnerNotice {
                title: Some("T".into()),
                text: "hi".into(),
            })
            .await
            .unwrap();
        assert!(receipt.posted && !receipt.muted);
        let (method, path, body) = calls.0.lock().unwrap()[0].clone();
        assert_eq!(
            (method.as_str(), path.as_str()),
            ("POST", "bots/bot1/notify")
        );
        assert_eq!(body["conversation_id"], "conv");
        assert_eq!(body["title"], "T");
    }

    #[tokio::test]
    async fn people_refusals_become_deliveries() {
        let calls = Calls::answering(vec![Err(HubFailure::answered(404, None, "no"))]);
        let delivery = HubPeople(routes(&calls)).message("p", "hi").await.unwrap();
        assert!(matches!(delivery, Delivery::NotAContact));
        let limited = Calls::answering(vec![Err(HubFailure::answered(429, None, "slow"))]);
        assert!(
            HubPeople(routes(&limited))
                .message("p", "hi")
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn contacts_pass_the_query_as_a_parameter() {
        let calls = Calls::answering(vec![Ok(
            json!({ "people": [{ "id": "p1", "name": "Ada" }] }),
        )]);
        let people = HubPeople(routes(&calls))
            .contacts(Some("a b"))
            .await
            .unwrap();
        assert_eq!(people.len(), 1);
        assert_eq!(calls.0.lock().unwrap()[0].2, json!([["q", "a b"]]));
    }

    #[test]
    fn mail_failures_keep_their_meaning() {
        assert_eq!(failure_of("429 limit").code, failure::LIMIT);
        assert_eq!(failure_of("404 no mail thread").code, failure::NO_THREAD);
        assert_eq!(failure_of("boom").code, failure::UNAVAILABLE);
    }
}

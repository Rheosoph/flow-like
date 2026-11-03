//! The desktop's `MailBackend`: an account bot's mail goes through the hub, which holds the mail
//! provider, the limits and the bot's threads. A bot kept only on this device has no mail.

use std::sync::Arc;

use async_trait::async_trait;
use flow_like_bot_tools::email::{
    MailBackend, MailFailure, MailFile, OutgoingMail, ReplyMail, SentMail, failure,
};
use flow_like_types::base64::{Engine, engine::general_purpose::STANDARD};
use reqwest::Method;
use serde_json::{Value, json};
use tauri::{AppHandle, Manager};

use super::schedule::TaskApi;
use crate::functions::local_bots::{LocalBots, on_store};

struct HubMail {
    api: Arc<dyn TaskApi>,
    bot_id: String,
}

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

pub(crate) fn failure_of(message: &str) -> MailFailure {
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

fn sent_of(value: &Value) -> Result<SentMail, MailFailure> {
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

fn unusable() -> MailFailure {
    MailFailure::new(
        failure::UNAVAILABLE,
        "The hub sent an unusable mail receipt",
    )
}

impl HubMail {
    fn path(&self, tail: &str) -> String {
        format!(
            "bots/{}/mail/{tail}",
            super::super::native_platform_tools::segment(&self.bot_id)
        )
    }

    async fn post(&self, tail: &str, body: Value) -> Result<SentMail, MailFailure> {
        let reply = self
            .api
            .request(Method::POST, &self.path(tail), Some(&body))
            .await
            .map_err(|error| failure_of(&error))?;
        sent_of(&reply)
    }
}

#[async_trait]
impl MailBackend for HubMail {
    async fn send(&self, mail: OutgoingMail) -> Result<SentMail, MailFailure> {
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

    async fn reply(&self, mail: ReplyMail) -> Result<SentMail, MailFailure> {
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

/// The hub's mail for `bot_id`, or `None` when this device holds the bot.
pub(crate) async fn mail_for(
    app: &AppHandle,
    bot_id: &str,
    api: Arc<dyn TaskApi>,
) -> Option<Arc<dyn MailBackend>> {
    if let Some(bots) = app.try_state::<LocalBots>() {
        let lookup = bot_id.to_string();
        let local = on_store(&bots, move |store| {
            store.get_bot(&lookup).map(|bot| bot.is_some())
        })
        .await
        .unwrap_or(false);
        if local {
            return None;
        }
    }
    Some(Arc::new(HubMail {
        api,
        bot_id: bot_id.to_string(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct Recorder {
        calls: Mutex<Vec<(String, Value)>>,
        answer: Result<Value, String>,
    }

    #[async_trait]
    impl TaskApi for Recorder {
        async fn request(
            &self,
            method: Method,
            path: &str,
            body: Option<&Value>,
        ) -> Result<Value, String> {
            assert_eq!(method, Method::POST);
            self.calls
                .lock()
                .unwrap()
                .push((path.to_string(), body.cloned().unwrap_or(Value::Null)));
            self.answer.clone()
        }
    }

    fn mail() -> OutgoingMail {
        OutgoingMail {
            to: vec!["a@b.com".into()],
            cc: vec![],
            subject: "S".into(),
            text: "x".into(),
            html: "<p>x</p>".into(),
            attachments: vec![MailFile {
                filename: "a.txt".into(),
                content_type: "text/plain".into(),
                bytes: b"hi".to_vec().into(),
            }],
        }
    }

    #[tokio::test]
    async fn a_send_goes_to_the_hub_with_base64_attachments_and_returns_the_receipt() {
        let api = Arc::new(Recorder {
            calls: Mutex::new(Vec::new()),
            answer: Ok(
                json!({"message_id": "m", "thread": "t", "from": "b@flow-like.me",
                              "to": ["a@b.com"], "subject": "S"}),
            ),
        });
        let backend = HubMail {
            api: api.clone(),
            bot_id: "bot_1".into(),
        };
        let sent = backend.send(mail()).await.unwrap();
        assert_eq!(sent.thread, "t");
        let calls = api.calls.lock().unwrap();
        assert!(calls[0].0.ends_with("bot_1/mail/send"));
        assert_eq!(calls[0].1["attachments"][0]["content"], "aGk=");
    }

    #[tokio::test]
    async fn hub_refusals_keep_their_meaning() {
        let backend = HubMail {
            api: Arc::new(Recorder {
                calls: Mutex::new(Vec::new()),
                answer: Err("429 Too Many Requests: This bot reached its limit".into()),
            }),
            bot_id: "bot_1".into(),
        };
        assert_eq!(backend.send(mail()).await.unwrap_err().code, failure::LIMIT);
        assert_eq!(failure_of("No mail thread 'x'").code, failure::NO_THREAD);
        assert_eq!(failure_of("boom").code, failure::UNAVAILABLE);
    }
}

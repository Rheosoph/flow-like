use std::sync::Arc;

use async_trait::async_trait;
use flow_like_types::Result;
use flow_like_types::bot_notify::{MESSAGE_MAX_CHARS, TITLE_MAX_CHARS, TOOL_MAX_PER_WINDOW};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

use crate::env::ToolEnv;
use crate::error::{ToolError, ToolResult, code};
use crate::outcome::ToolOutput;
use crate::spec::{Approval, ToolSpec, schema_of};

pub const MESSAGE_OWNER: &str = "message_owner";

/// Where a turn runs, as far as `message_owner` needs to know.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnSite {
    /// A headless turn in this conversation.
    Conversation(String),
    /// A chat the owner is reading live.
    OwnerLive,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerNotice {
    pub title: Option<String>,
    pub text: String,
}

impl OwnerNotice {
    /// The message as it is written into the owner's chat.
    pub fn message(&self) -> String {
        match &self.title {
            Some(title) => format!("{title}\n\n{}", self.text),
            None => self.text.clone(),
        }
    }

    /// The one-line push body.
    pub fn push_body(&self) -> String {
        match &self.title {
            Some(title) => format!("{title}: {}", self.text),
            None => self.text.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NoticeReceipt {
    pub id: String,
    /// The owner muted this bot, so no push was sent.
    pub muted: bool,
    /// The owner is reading this turn live or it runs in their direct chat; nothing was sent.
    pub in_owner_chat: bool,
    /// The message is in the owner's chat; `false` when the chat cannot be written to (encrypted).
    pub posted: bool,
}

/// How a host reaches the owner of the bot that runs the tool. `deliver` writes the message into
/// the bot's direct chat with the owner and pushes it, grouped under the bot; counting what the
/// bot sent lets the tool enforce its rate limit across turns.
#[async_trait]
pub trait OwnerNotifier: Send + Sync {
    async fn sent_since(&self, since_ms: i64) -> Result<usize>;
    async fn deliver(&self, notice: OwnerNotice) -> Result<NoticeReceipt>;
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MessageOwnerArgs {
    /// What the owner should read. At most 4000 characters.
    pub text: String,
    /// A short headline shown above the text. At most 120 characters.
    #[serde(default)]
    pub title: Option<String>,
}

pub fn specs() -> Vec<ToolSpec> {
    vec![ToolSpec {
        name: MESSAGE_OWNER,
        description: "Write a message to the owner in your direct chat with them, from any turn (a schedule, a mail, another bot, a group room); they get a push notification for it. Not for the chat you are already answering in: reply there instead. Use it sparingly for what needs their attention; at most 10 per hour.",
        capability: Some("push_notifications"),
        module: "notify",
        approval: Approval::None,
        approval_args: &[],
        mutating: true,
        input_schema: schema_of::<MessageOwnerArgs>(),
    }]
}

fn checked(value: &str, field: &str, max: usize) -> ToolResult<String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(ToolError::invalid(format!("'{field}' is empty")));
    }
    let chars = value.chars().count();
    if chars > max {
        return Err(ToolError::invalid(format!(
            "'{field}' has {chars} characters; at most {max} are allowed"
        )));
    }
    Ok(value.to_string())
}

pub fn notice_from(text: &str, title: Option<&str>) -> ToolResult<OwnerNotice> {
    let title = match title.map(str::trim).filter(|title| !title.is_empty()) {
        Some(title) => Some(checked(title, "title", TITLE_MAX_CHARS)?),
        None => None,
    };
    Ok(OwnerNotice {
        title,
        text: checked(text, "text", MESSAGE_MAX_CHARS)?,
    })
}

pub(crate) fn window_ms() -> i64 {
    flow_like_types::bot_notify::TOOL_WINDOW_SECS * 1000
}

pub async fn message_owner(env: &dyn ToolEnv, args: MessageOwnerArgs) -> ToolResult<ToolOutput> {
    let notifier: Arc<dyn OwnerNotifier> = env
        .notifier()
        .ok_or_else(|| ToolError::new(code::FAILED, "This host cannot message the owner"))?;
    let notice = notice_from(&args.text, args.title.as_deref())?;
    let since = chrono::Utc::now().timestamp_millis() - window_ms();
    let sent = notifier.sent_since(since).await?;
    if sent >= TOOL_MAX_PER_WINDOW {
        return Err(ToolError::new(
            code::RATE_LIMITED,
            format!(
                "This bot already sent {sent} messages to its owner in the last hour; at most {TOOL_MAX_PER_WINDOW} are allowed. Put the rest in your reply instead."
            ),
        ));
    }
    let receipt = notifier.deliver(notice).await?;
    if receipt.in_owner_chat {
        return Err(ToolError::new(
            code::ALREADY_HERE,
            "The owner is reading this chat (or this turn already runs in their direct chat); just answer here instead of calling message_owner",
        ));
    }
    Ok(ToolOutput::new(json!({
        "messaged": receipt.posted,
        "notified": !receipt.muted,
        "muted": receipt.muted,
        "message_id": receipt.id,
        "remaining_this_hour": TOOL_MAX_PER_WINDOW.saturating_sub(sent + 1),
    })))
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::testing::FakeEnv;
    use serde_json::{Value, json};

    #[derive(Default)]
    struct FakeNotifier {
        prior: AtomicUsize,
        muted: bool,
        in_owner_chat: bool,
        delivered: Mutex<Vec<OwnerNotice>>,
    }

    #[async_trait]
    impl OwnerNotifier for FakeNotifier {
        async fn sent_since(&self, _since_ms: i64) -> Result<usize> {
            Ok(self.prior.load(Ordering::SeqCst) + self.delivered.lock().unwrap().len())
        }

        async fn deliver(&self, notice: OwnerNotice) -> Result<NoticeReceipt> {
            if self.in_owner_chat {
                return Ok(NoticeReceipt {
                    in_owner_chat: true,
                    ..Default::default()
                });
            }
            self.delivered.lock().unwrap().push(notice);
            Ok(NoticeReceipt {
                id: "m1".into(),
                muted: self.muted,
                posted: true,
                ..Default::default()
            })
        }
    }

    async fn env_with(notifier: Arc<FakeNotifier>) -> FakeEnv {
        let env = FakeEnv::new().await;
        *env.notifier.lock().unwrap() = Some(notifier);
        env
    }

    async fn call(env: &FakeEnv, args: Value) -> crate::ToolOutcome {
        crate::run(MESSAGE_OWNER, args, env).await
    }

    #[test]
    fn the_spec_needs_the_push_capability() {
        let spec = crate::spec(MESSAGE_OWNER).expect("registered");
        assert_eq!(spec.capability, Some("push_notifications"));
        assert!(spec.mutating);
        assert!(crate::spec("notify_owner").is_none());
    }

    #[tokio::test]
    async fn a_message_reaches_the_notifier_trimmed() {
        let notifier = Arc::new(FakeNotifier::default());
        let env = env_with(notifier.clone()).await;
        let outcome = call(
            &env,
            json!({ "title": " Report ready ", "text": " Q3 is done " }),
        )
        .await;
        assert_eq!(outcome.status, crate::Status::Ok, "{:?}", outcome.error);
        assert_eq!(outcome.result["messaged"], true);
        assert_eq!(outcome.result["notified"], true);
        let delivered = notifier.delivered.lock().unwrap();
        assert_eq!(delivered.len(), 1);
        assert_eq!(delivered[0].message(), "Report ready\n\nQ3 is done");
        assert_eq!(delivered[0].push_body(), "Report ready: Q3 is done");
    }

    #[tokio::test]
    async fn the_title_is_optional() {
        let notifier = Arc::new(FakeNotifier::default());
        let env = env_with(notifier.clone()).await;
        let outcome = call(&env, json!({ "text": "Mail from Ada about the invoice" })).await;
        assert_eq!(outcome.status, crate::Status::Ok);
        let delivered = notifier.delivered.lock().unwrap();
        assert_eq!(delivered[0].message(), "Mail from Ada about the invoice");
    }

    #[tokio::test]
    async fn a_turn_in_the_owner_chat_is_refused_and_sends_nothing() {
        let notifier = Arc::new(FakeNotifier {
            in_owner_chat: true,
            ..Default::default()
        });
        let env = env_with(notifier.clone()).await;
        let outcome = call(&env, json!({ "text": "hello" })).await;
        assert_eq!(outcome.status, crate::Status::Error);
        let error = outcome.error.expect("refused");
        assert_eq!(error.code, code::ALREADY_HERE);
        assert!(
            error.message.contains("just answer here"),
            "{}",
            error.message
        );
        assert!(notifier.delivered.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn the_tenth_message_of_an_hour_passes_and_the_eleventh_is_refused() {
        let notifier = Arc::new(FakeNotifier::default());
        notifier
            .prior
            .store(TOOL_MAX_PER_WINDOW - 1, Ordering::SeqCst);
        let env = env_with(notifier.clone()).await;
        let tenth = call(&env, json!({ "text": "b" })).await;
        assert_eq!(tenth.status, crate::Status::Ok);
        assert_eq!(tenth.result["remaining_this_hour"], 0);
        let eleventh = call(&env, json!({ "text": "b" })).await;
        assert_eq!(eleventh.status, crate::Status::Error);
        assert_eq!(eleventh.error.unwrap().code, code::RATE_LIMITED);
        assert_eq!(notifier.delivered.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn bad_arguments_never_reach_the_notifier() {
        let notifier = Arc::new(FakeNotifier::default());
        let env = env_with(notifier.clone()).await;
        let long = "x".repeat(MESSAGE_MAX_CHARS + 1);
        let long_title = "t".repeat(TITLE_MAX_CHARS + 1);
        for args in [
            json!({ "text": " " }),
            json!({ "text": long }),
            json!({ "text": "b", "title": long_title }),
        ] {
            let outcome = call(&env, args).await;
            assert_eq!(outcome.error.expect("refused").code, code::INVALID_ARGS);
        }
        assert!(notifier.delivered.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_muted_bot_reports_that_no_push_was_sent() {
        let notifier = Arc::new(FakeNotifier {
            muted: true,
            ..Default::default()
        });
        let env = env_with(notifier).await;
        let outcome = call(&env, json!({ "text": "b" })).await;
        assert_eq!(outcome.result["notified"], false);
        assert_eq!(outcome.result["muted"], true);
        assert_eq!(outcome.result["messaged"], true);
    }

    #[tokio::test]
    async fn a_host_without_a_notifier_fails_plainly() {
        let env = FakeEnv::new().await;
        let outcome = call(&env, json!({ "text": "b" })).await;
        assert_eq!(outcome.error.expect("failed").code, code::FAILED);
    }
}

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{Value, json};

use super::*;
use crate::testing::{FakeEnv, WORKSPACE};
use crate::{Approval, ToolOutcome, run, spec};

#[derive(Default)]
struct FakeMail {
    sent: Mutex<Vec<OutgoingMail>>,
    replies: Mutex<Vec<ReplyMail>>,
    fail: Mutex<Option<MailFailure>>,
}

#[async_trait]
impl MailBackend for FakeMail {
    async fn send(&self, mail: OutgoingMail) -> Result<SentMail, MailFailure> {
        if let Some(failure) = self.fail.lock().unwrap().clone() {
            return Err(failure);
        }
        let sent = SentMail {
            message_id: "m1".into(),
            thread: "t1".into(),
            from: "bot@flow-like.me".into(),
            to: mail.to.clone(),
            subject: mail.subject.clone(),
        };
        self.sent.lock().unwrap().push(mail);
        Ok(sent)
    }

    async fn reply(&self, mail: ReplyMail) -> Result<SentMail, MailFailure> {
        let sent = SentMail {
            message_id: "m2".into(),
            thread: mail.thread.clone(),
            from: "bot@flow-like.me".into(),
            to: vec!["a@b.com".into()],
            subject: "Re: x".into(),
        };
        self.replies.lock().unwrap().push(mail);
        Ok(sent)
    }
}

async fn env_with(mail: Arc<FakeMail>) -> FakeEnv {
    let env = FakeEnv::new().await;
    *env.mail.lock().unwrap() = Some(mail);
    env
}

async fn call(env: &FakeEnv, tool: &str, args: Value) -> ToolOutcome {
    run(tool, args, env).await
}

#[tokio::test]
async fn send_renders_markdown_and_escapes_raw_html() {
    let mail = Arc::new(FakeMail::default());
    let env = env_with(mail.clone()).await;
    let outcome = call(
        &env,
        EMAIL_SEND,
        json!({"to": ["a@b.com"], "subject": "Hi", "body": "**bold** <script>x</script>"}),
    )
    .await;
    assert!(outcome.is_ok(), "{:?}", outcome.error);
    let sent = mail.sent.lock().unwrap();
    assert!(sent[0].html.contains("<strong>bold</strong>"));
    assert!(!sent[0].html.contains("<script>"));
    assert!(sent[0].text.contains("**bold**"));
}

#[tokio::test]
async fn send_attaches_workspace_files_and_refuses_too_many() {
    let mail = Arc::new(FakeMail::default());
    let env = env_with(mail.clone()).await;
    env.put(WORKSPACE, "bot/files/a.txt", b"hello", "text/plain");
    let ok = call(
        &env,
        EMAIL_SEND,
        json!({"to": ["a@b.com"], "subject": "S", "body": "x", "attachments": ["bot/files/a.txt"]}),
    )
    .await;
    assert!(ok.is_ok(), "{:?}", ok.error);
    assert_eq!(
        mail.sent.lock().unwrap()[0].attachments[0].filename,
        "a.txt"
    );
    let many: Vec<String> = (0..11).map(|_| "bot/files/a.txt".to_string()).collect();
    let refused = call(
        &env,
        EMAIL_SEND,
        json!({"to": ["a@b.com"], "subject": "S", "body": "x", "attachments": many}),
    )
    .await;
    assert_eq!(refused.error.unwrap().code, "invalid_args");
}

#[tokio::test]
async fn send_validates_recipients_and_body() {
    let mail = Arc::new(FakeMail::default());
    let env = env_with(mail.clone()).await;
    for args in [
        json!({"to": [], "subject": "S", "body": "x"}),
        json!({"to": ["not-an-address"], "subject": "S", "body": "x"}),
        json!({"to": ["a@b.com"], "subject": " ", "body": "x"}),
        json!({"to": ["a@b.com"], "subject": "S", "body": "  "}),
    ] {
        let outcome = call(&env, EMAIL_SEND, args).await;
        assert_eq!(outcome.error.unwrap().code, "invalid_args");
    }
    assert!(mail.sent.lock().unwrap().is_empty());
}

#[tokio::test]
async fn host_failures_map_to_distinct_codes() {
    let mail = Arc::new(FakeMail::default());
    let env = env_with(mail.clone()).await;
    for (failure_code, expected) in [
        (failure::LIMIT, "mail_limit"),
        (failure::NO_ADDRESS, "mail_no_address"),
        (failure::REFUSED, "scope_refused"),
        (failure::UNAVAILABLE, "failed"),
    ] {
        *mail.fail.lock().unwrap() = Some(MailFailure::new(failure_code, "nope"));
        let outcome = call(
            &env,
            EMAIL_SEND,
            json!({"to": ["a@b.com"], "subject": "S", "body": "x"}),
        )
        .await;
        assert_eq!(outcome.error.unwrap().code, expected);
    }
}

#[tokio::test]
async fn a_host_without_mail_says_so() {
    let env = FakeEnv::new().await;
    let outcome = call(&env, EMAIL_REPLY, json!({"thread": "t", "body": "x"})).await;
    assert_eq!(outcome.error.unwrap().code, "not_yet");
}

#[tokio::test]
async fn reply_passes_thread_and_html() {
    let mail = Arc::new(FakeMail::default());
    let env = env_with(mail.clone()).await;
    let outcome = call(&env, EMAIL_REPLY, json!({"thread": " t9 ", "body": "_hi_"})).await;
    assert!(outcome.is_ok(), "{:?}", outcome.error);
    let replies = mail.replies.lock().unwrap();
    assert_eq!(replies[0].thread, "t9");
    assert!(replies[0].html.contains("<em>hi</em>"));
}

#[test]
fn specs_gate_on_the_email_capability_and_only_send_asks() {
    let send = spec(EMAIL_SEND).unwrap();
    let reply = spec(EMAIL_REPLY).unwrap();
    assert_eq!(send.capability, Some("email"));
    assert_eq!(reply.capability, Some("email"));
    assert_eq!(send.approval, Approval::Ask);
    assert_eq!(reply.approval, Approval::None);
    assert!(send.mutating && reply.mutating);
}

#[test]
fn every_recipient_domain_is_its_own_scope_sorted_and_deduplicated() {
    let scopes = grant_scopes(&json!({
        "to": ["a@Example.com", "b@example.com"], "cc": ["c@other.org"]
    }));
    assert_eq!(
        scopes,
        Some(vec![
            "email:example.com".to_string(),
            "email:other.org".to_string()
        ])
    );
}

#[test]
fn a_send_with_an_unreadable_recipient_is_never_remembered() {
    assert_eq!(grant_scopes(&json!({"to": ["a@x.com", "nonsense"]})), None);
    assert_eq!(grant_scopes(&json!({"to": []})), None);
    assert_eq!(grant_scopes(&json!({})), None);
}

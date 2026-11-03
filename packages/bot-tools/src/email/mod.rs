mod backend;

use pulldown_cmark::{Event, Options, Parser, html};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

use crate::env::ToolEnv;
use crate::error::{ToolError, ToolResult, code};
use crate::files::{FileRef, resolve};
use crate::outcome::ToolOutput;
use crate::spec::{Approval, ToolSpec, schema_of};

pub use backend::{MailBackend, MailFailure, MailFile, OutgoingMail, ReplyMail, SentMail, failure};

pub const EMAIL_SEND: &str = "email_send";
pub const EMAIL_REPLY: &str = "email_reply";

const MAX_ATTACHMENTS: usize = 10;
const MAX_ATTACHMENT_BYTES: u64 = 3 * 1024 * 1024;
const MAX_RECIPIENTS: usize = 20;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct EmailSendArgs {
    /// Recipient addresses (plain `name@host`). A domain the owner has not approved yet asks them first.
    pub to: Vec<String>,
    /// Carbon-copy addresses.
    #[serde(default)]
    pub cc: Vec<String>,
    pub subject: String,
    /// The message in markdown; it is sent as text and HTML.
    pub body: String,
    /// Files to attach (at most 10, 3 MiB each): workspace paths, attachments, handles.
    #[serde(default)]
    pub attachments: Vec<FileRef>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct EmailReplyArgs {
    /// The id of the mail thread (from the mail trigger's `payload.thread`).
    pub thread: String,
    /// The reply in markdown; it is sent as text and HTML to the sender of the thread's latest mail.
    pub body: String,
    /// Files to attach (at most 10, 3 MiB each).
    #[serde(default)]
    pub attachments: Vec<FileRef>,
}

pub fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: EMAIL_SEND,
            description: "Send an email from the bot's own address to new recipients. The owner approves each new recipient domain once. Mail is recorded in a mail thread; replies to it come back to the bot.",
            capability: Some("email"),
            module: "email",
            approval: Approval::Ask,
            approval_args: &[],
            mutating: true,
            input_schema: schema_of::<EmailSendArgs>(),
        },
        ToolSpec {
            name: EMAIL_REPLY,
            description: "Reply in a mail thread, to the sender of its latest mail, from the bot's own address. Use it to answer an email trigger.",
            capability: Some("email"),
            module: "email",
            approval: Approval::None,
            approval_args: &[],
            mutating: true,
            input_schema: schema_of::<EmailReplyArgs>(),
        },
    ]
}

pub fn recipient_domain(address: &str) -> Option<String> {
    let (local, host) = address.trim().rsplit_once('@')?;
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    (!local.is_empty() && !host.is_empty() && !host.contains(char::is_whitespace)).then_some(host)
}

/// One `email:<domain>` scope per recipient domain of a send, sorted; the owner approves each
/// new domain once. A send with no readable recipient is never remembered.
pub fn grant_scopes(args: &serde_json::Value) -> Option<Vec<String>> {
    let mut domains: Vec<String> = ["to", "cc"]
        .iter()
        .filter_map(|key| args.get(key)?.as_array())
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .map(|address| recipient_domain(address).map(|domain| format!("email:{domain}")))
        .collect::<Option<_>>()?;
    domains.sort();
    domains.dedup();
    (!domains.is_empty()).then_some(domains)
}

pub fn markdown_to_html(markdown: &str) -> String {
    let parser = Parser::new_ext(
        markdown,
        Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES,
    )
    .map(|event| match event {
        Event::Html(raw) | Event::InlineHtml(raw) => Event::Text(raw),
        other => other,
    });
    let mut out = String::new();
    html::push_html(&mut out, parser);
    out
}

fn failure_error(failure: MailFailure) -> ToolError {
    let code = match failure.code {
        failure::LIMIT => "mail_limit",
        failure::NO_ADDRESS => "mail_no_address",
        failure::NO_THREAD => code::NOT_FOUND,
        failure::REFUSED => code::SCOPE_REFUSED,
        _ => code::FAILED,
    };
    ToolError::new(code, failure.message)
}

fn backend(env: &dyn ToolEnv) -> ToolResult<std::sync::Arc<dyn MailBackend>> {
    env.mail().ok_or_else(|| {
        ToolError::new(
            code::NOT_YET,
            "Email is only available to bots on an account that run on the server",
        )
    })
}

async fn files(env: &dyn ToolEnv, refs: &[FileRef]) -> ToolResult<Vec<MailFile>> {
    if refs.len() > MAX_ATTACHMENTS {
        return Err(ToolError::invalid(format!(
            "Attach at most {MAX_ATTACHMENTS} files"
        )));
    }
    let mut out = Vec::new();
    for reference in refs {
        let file = resolve(env, reference, MAX_ATTACHMENT_BYTES).await?;
        out.push(MailFile {
            filename: file.name,
            content_type: file.mime,
            bytes: file.bytes,
        });
    }
    Ok(out)
}

fn clean_list(label: &str, list: Vec<String>) -> ToolResult<Vec<String>> {
    let clean: Vec<String> = list
        .into_iter()
        .map(|a| a.trim().to_owned())
        .filter(|a| !a.is_empty())
        .collect();
    if clean
        .iter()
        .any(|a| !a.contains('@') || a.contains(char::is_whitespace))
    {
        return Err(ToolError::invalid(format!(
            "'{label}' must hold plain email addresses"
        )));
    }
    Ok(clean)
}

fn require_body(body: &str) -> ToolResult<()> {
    if body.trim().is_empty() {
        return Err(ToolError::invalid("The mail body is empty"));
    }
    Ok(())
}

fn sent_output(sent: SentMail) -> ToolOutput {
    ToolOutput::new(json!({
        "sent": true,
        "message_id": sent.message_id,
        "thread": sent.thread,
        "from": sent.from,
        "to": sent.to,
        "subject": sent.subject,
    }))
}

pub async fn email_send(env: &dyn ToolEnv, args: EmailSendArgs) -> ToolResult<ToolOutput> {
    let to = clean_list("to", args.to)?;
    let cc = clean_list("cc", args.cc)?;
    if to.is_empty() || to.len() + cc.len() > MAX_RECIPIENTS {
        return Err(ToolError::invalid(format!(
            "Give at least one recipient and at most {MAX_RECIPIENTS} in total"
        )));
    }
    if args.subject.trim().is_empty() {
        return Err(ToolError::invalid("The subject is empty"));
    }
    require_body(&args.body)?;
    let backend = backend(env)?;
    let attachments = files(env, &args.attachments).await?;
    let mail = OutgoingMail {
        to,
        cc,
        subject: args.subject,
        html: markdown_to_html(&args.body),
        text: args.body,
        attachments,
    };
    backend
        .send(mail)
        .await
        .map(sent_output)
        .map_err(failure_error)
}

pub async fn email_reply(env: &dyn ToolEnv, args: EmailReplyArgs) -> ToolResult<ToolOutput> {
    require_body(&args.body)?;
    if args.thread.trim().is_empty() {
        return Err(ToolError::invalid("Name the mail thread to reply in"));
    }
    let backend = backend(env)?;
    let attachments = files(env, &args.attachments).await?;
    let mail = ReplyMail {
        thread: args.thread.trim().to_owned(),
        html: markdown_to_html(&args.body),
        text: args.body,
        attachments,
    };
    backend
        .reply(mail)
        .await
        .map(sent_output)
        .map_err(failure_error)
}

#[cfg(test)]
mod tests;

//! A bot's outbound mail and the owner's view of its mail threads. Sending uses the platform
//! mail client with the bot's own address as From, within the per-bot and per-account daily
//! recipient limits; every sent mail is recorded in the owner's conversation with the bot. The
//! owner approves before the tool gets here: a send once per new recipient domain (grant scope
//! `email:<domain>`), replies once or for good.

use crate::{
    entity::{bot, bot_mail_message, bot_mail_thread, inbox_entry},
    error::ApiError,
    mail::{AutoSubmitted, AutomationEmailMessage},
    mail_ingress::{
        bot_address,
        limits::{self, Counter, Window},
        now,
    },
    routes::{
        app::mail::{
            clean_display_name, decode_attachments, ensure_external, reserve_interval,
            valid_address, validate_body,
        },
        inbox::{
            mail_threads::{
                Direction, link_message_in, mail_conversation_in, record_outbound_in,
                start_thread_in, touch_thread_in,
            },
            now_millis, wake,
        },
    },
    state::AppState,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use flow_like_bot_tools::email::{
    MailFailure, MailFile, OutgoingMail, ReplyMail, SentMail, failure,
};
use flow_like_catalog_core::MailAttachment;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use serde_json::{Value, json};

const MAX_RECIPIENTS: usize = 20;

fn refused(error: ApiError) -> MailFailure {
    MailFailure::new(failure::REFUSED, error.to_string())
}

fn unavailable(error: impl std::fmt::Display) -> MailFailure {
    MailFailure::new(failure::UNAVAILABLE, error.to_string())
}

struct Composed {
    to: Vec<String>,
    cc: Vec<String>,
    subject: String,
    text: String,
    html: String,
    attachments: Vec<MailFile>,
    in_reply_to: Option<String>,
    references: Vec<String>,
}

fn dedupe(to: Vec<String>, cc: Vec<String>) -> (Vec<String>, Vec<String>) {
    let mut seen = std::collections::HashSet::new();
    let mut keep = |list: Vec<String>| {
        list.into_iter()
            .filter(|a| seen.insert(a.to_ascii_lowercase()))
            .collect::<Vec<_>>()
    };
    let to = keep(to);
    (to, keep(cc))
}

fn validate(composed: &Composed, domain: Option<&str>) -> Result<(), ApiError> {
    let all: Vec<&String> = composed.to.iter().chain(&composed.cc).collect();
    if composed.to.is_empty() || all.len() > MAX_RECIPIENTS {
        return Err(ApiError::bad_request(
            "Provide a To recipient and at most 20 total recipients",
        ));
    }
    if all.iter().any(|a| !valid_address(a)) {
        return Err(ApiError::bad_request(
            "Recipients must be plain email addresses",
        ));
    }
    let subject = &composed.subject;
    if subject.trim().is_empty() || subject.len() > 998 || subject.chars().any(char::is_control) {
        return Err(ApiError::bad_request(
            "Subject must contain 1-998 bytes without control characters",
        ));
    }
    validate_body(Some(&composed.text), Some(&composed.html))?;
    ensure_external(domain, all)
}

fn limit_failure(error: crate::routes::app::mail::MailError) -> MailFailure {
    let wait = error
        .retry_after
        .map(|secs| format!(" Try again in {secs} s."))
        .unwrap_or_default();
    MailFailure::new(failure::LIMIT, format!("{}{wait}", error.error))
}

async fn consume_daily(
    state: &AppState,
    bot: &bot::Model,
    recipients: u32,
) -> Result<(Window, String, String), MailFailure> {
    let config = &state.mail_automation;
    let window = Window::at(now());
    let bot_key = bot.id.clone();
    let owner_key = bot.owner_id.clone();
    let counters = [
        Counter {
            scope: "bot",
            subject: &bot_key,
            limit: config.daily_app_recipient_limit,
        },
        Counter {
            scope: "principal",
            subject: &owner_key,
            limit: config.daily_principal_recipient_limit,
        },
    ];
    match limits::consume(state, &window, &counters, recipients)
        .await
        .map_err(unavailable)?
    {
        None => Ok((window, bot_key, owner_key)),
        Some(index) => {
            let who = if index == 0 {
                "This bot"
            } else {
                "This account"
            };
            Err(MailFailure::new(
                failure::LIMIT,
                format!(
                    "{who} reached its limit of {} mail recipients per day. Try again in {} s.",
                    counters[index].limit,
                    window.retry_after_secs(now())
                ),
            ))
        }
    }
}

async fn refund_daily(
    state: &AppState,
    window: &Window,
    bot_key: &str,
    owner_key: &str,
    recipients: u32,
) {
    let config = &state.mail_automation;
    let counters = [
        Counter {
            scope: "bot",
            subject: bot_key,
            limit: config.daily_app_recipient_limit,
        },
        Counter {
            scope: "principal",
            subject: owner_key,
            limit: config.daily_principal_recipient_limit,
        },
    ];
    limits::refund(state, window, &counters, recipients).await;
}

async fn live_bot(state: &AppState, bot_id: &str) -> Result<bot::Model, MailFailure> {
    bot::Entity::find_by_id(bot_id)
        .filter(bot::Column::DeletedAt.is_null())
        .one(&state.db)
        .await
        .map_err(unavailable)?
        .filter(bot_address::has_email)
        .ok_or_else(|| {
            MailFailure::new(failure::REFUSED, "This bot does not have the email ability")
        })
}

fn attachments_of(files: Vec<MailFile>) -> Vec<MailAttachment> {
    files
        .into_iter()
        .map(|file| MailAttachment {
            filename: file.filename,
            content_type: file.content_type,
            content: STANDARD.encode(&file.bytes),
        })
        .collect()
}

async fn deliver(
    state: &AppState,
    bot: &bot::Model,
    from: &str,
    thread_id: &str,
    mut composed: Composed,
) -> Result<SentMail, MailFailure> {
    let domain = state.mail_automation.domain().map(str::to_owned);
    let (to, cc) = dedupe(
        std::mem::take(&mut composed.to),
        std::mem::take(&mut composed.cc),
    );
    composed.to = to;
    composed.cc = cc;
    validate(&composed, domain.as_deref()).map_err(refused)?;
    let attachments = decode_attachments(attachments_of(std::mem::take(&mut composed.attachments)))
        .map_err(refused)?;
    let client = state
        .automation_mail_client
        .clone()
        .ok_or_else(|| MailFailure::new(failure::UNAVAILABLE, "No mail provider is configured"))?;
    if !state.mail_automation.sending_enabled {
        return Err(MailFailure::new(
            failure::UNAVAILABLE,
            "Sending mail is disabled on this server",
        ));
    }
    reserve_interval(state, &format!("bot:{}", bot.id))
        .await
        .map_err(limit_failure)?;
    let recipients = (composed.to.len() + composed.cc.len()) as u32;
    let (window, bot_key, owner_key) = consume_daily(state, bot, recipients).await?;
    let (local, host) = from.split_once('@').unwrap_or((from, ""));
    let message = AutomationEmailMessage {
        from_email: Some(from.to_owned()),
        from_name: bot.name.as_deref().and_then(clean_display_name),
        to: composed.to.clone(),
        cc: composed.cc.clone(),
        bcc: Vec::new(),
        reply_to: Some(format!("{local}+{thread_id}@{host}")),
        subject: composed.subject.clone(),
        body_html: Some(composed.html.clone()),
        body_text: Some(composed.text.clone()),
        in_reply_to: composed.in_reply_to.clone(),
        references: composed.references.clone(),
        auto_submitted: Some(if composed.in_reply_to.is_some() {
            AutoSubmitted::Replied
        } else {
            AutoSubmitted::Generated
        }),
        attachments,
    };
    if let Err(error) = client.send_automation(message).await {
        refund_daily(state, &window, &bot_key, &owner_key, recipients).await;
        tracing::error!(bot_id = %bot.id, %error, "Bot mail submission failed");
        return Err(MailFailure::new(
            failure::UNAVAILABLE,
            "The mail provider could not confirm acceptance",
        ));
    }
    let sent = SentMail {
        message_id: format!("mailout-{}", flow_like_types::create_id()),
        thread: thread_id.to_owned(),
        from: from.to_owned(),
        to: composed.to.iter().chain(&composed.cc).cloned().collect(),
        subject: composed.subject.clone(),
    };
    if let Err(error) = record_sent(state, bot, &composed, &sent).await {
        tracing::warn!(bot_id = %bot.id, thread = %thread_id, %error, "A sent bot mail could not be recorded in its thread");
    }
    Ok(sent)
}

async fn record_sent(
    state: &AppState,
    bot: &bot::Model,
    composed: &Composed,
    sent: &SentMail,
) -> Result<(), ApiError> {
    let received = now_millis();
    let mail = json!({
        "direction": "out",
        "from": sent.from,
        "to": composed.to,
        "cc": composed.cc,
        "subject": composed.subject,
        "attachments": composed.attachments.iter().map(|f| json!({
            "name": f.filename, "content_type": f.content_type, "size": f.bytes.len(),
        })).collect::<Vec<_>>(),
        "thread": sent.thread,
    });
    let participants: Vec<String> = composed.to.iter().chain(&composed.cc).cloned().collect();
    let existing = bot_mail_thread::Entity::find_by_id(sent.thread.as_str())
        .one(&state.db)
        .await?
        .is_some();
    let wake_list = state
        .transaction(|txn| {
            let (bot, sent, composed_subject, participants, mail) = (
                bot.clone(),
                sent.clone(),
                composed.subject.clone(),
                participants.clone(),
                mail.clone(),
            );
            let text = composed.text.clone();
            Box::pin(async move {
                if !existing {
                    start_thread_in(
                        txn,
                        &bot.owner_id,
                        &bot.id,
                        &composed_subject,
                        &participants,
                        Some(sent.thread.clone()),
                        received,
                    )
                    .await?;
                }
                link_message_in(
                    txn,
                    &sent.thread,
                    &bot.id,
                    &sent.message_id,
                    Direction::Out,
                    &sent.message_id,
                    received,
                )
                .await?;
                let conversation = mail_conversation_in(
                    txn,
                    &sent.thread,
                    &bot.owner_id,
                    &bot.id,
                    &composed_subject,
                    received,
                )
                .await?;
                let record = record_outbound_in(
                    txn,
                    &conversation,
                    &bot.id,
                    &sent.message_id,
                    &text,
                    mail,
                    received,
                )
                .await?;
                touch_thread_in(txn, &sent.thread, &participants, &text, received).await?;
                Ok::<_, ApiError>(record.wake)
            })
        })
        .await?;
    wake(state, wake_list).await;
    Ok(())
}

pub(crate) async fn send(
    state: &AppState,
    bot_id: &str,
    mail: OutgoingMail,
) -> Result<SentMail, MailFailure> {
    let bot = live_bot(state, bot_id).await?;
    let from = bot_address::address_of(&state.db, &bot.id)
        .await
        .map_err(unavailable)?
        .ok_or_else(|| {
            MailFailure::new(
                failure::NO_ADDRESS,
                "This bot has no email address yet. The owner claims one in the bot's email settings",
            )
        })?;
    let composed = Composed {
        to: mail.to,
        cc: mail.cc,
        subject: mail.subject,
        text: mail.text,
        html: mail.html,
        attachments: mail.attachments,
        in_reply_to: None,
        references: Vec::new(),
    };
    let thread_id = flow_like_types::create_id();
    deliver(state, &bot, &from, &thread_id, composed).await
}

struct Inbound {
    reply_to: String,
    subject: String,
    message_id: String,
    references: Vec<String>,
}

fn first_email(list: &Value) -> Option<String> {
    list.as_array()?
        .iter()
        .find_map(|a| a.get("email")?.as_str().map(str::to_owned))
}

async fn latest_inbound(
    state: &AppState,
    bot: &bot::Model,
    thread: &str,
) -> Result<Option<Inbound>, MailFailure> {
    let row = bot_mail_message::Entity::find()
        .filter(bot_mail_message::Column::ThreadId.eq(thread))
        .filter(bot_mail_message::Column::BotId.eq(bot.id.as_str()))
        .filter(bot_mail_message::Column::Direction.eq("IN"))
        .order_by_desc(bot_mail_message::Column::CreatedAt)
        .one(&state.db)
        .await
        .map_err(unavailable)?;
    let Some(row) = row else {
        return Ok(None);
    };
    let entry = inbox_entry::Entity::find()
        .filter(inbox_entry::Column::InboxId.eq(bot.owner_id.as_str()))
        .filter(inbox_entry::Column::MessageId.eq(row.entry_message_id.as_str()))
        .one(&state.db)
        .await
        .map_err(unavailable)?;
    let Some(entry) = entry else {
        return Ok(None);
    };
    let payload = entry
        .envelope
        .pointer("/body/payload")
        .cloned()
        .unwrap_or(Value::Null);
    let reply_to = first_email(&payload["reply_to"]).or_else(|| first_email(&payload["from"]));
    let references = payload["references"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(Value::as_str)
                .map(|id| format!("<{id}>"))
                .collect()
        })
        .unwrap_or_default();
    Ok(reply_to.map(|reply_to| Inbound {
        reply_to,
        subject: payload["subject"].as_str().unwrap_or_default().to_owned(),
        message_id: payload["message_id"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        references,
    }))
}

pub(crate) async fn reply(
    state: &AppState,
    bot_id: &str,
    mail: ReplyMail,
) -> Result<SentMail, MailFailure> {
    let bot = live_bot(state, bot_id).await?;
    let from = bot_address::address_of(&state.db, &bot.id)
        .await
        .map_err(unavailable)?
        .ok_or_else(|| MailFailure::new(failure::NO_ADDRESS, "This bot has no email address"))?;
    let thread = bot_mail_thread::Entity::find_by_id(mail.thread.as_str())
        .filter(bot_mail_thread::Column::BotId.eq(bot.id.as_str()))
        .one(&state.db)
        .await
        .map_err(unavailable)?
        .ok_or_else(|| {
            MailFailure::new(
                failure::NO_THREAD,
                format!("No mail thread '{}'", mail.thread),
            )
        })?;
    let inbound = latest_inbound(state, &bot, &thread.id)
        .await?
        .ok_or_else(|| {
            MailFailure::new(
                failure::NO_THREAD,
                "This thread has no received mail to reply to; use email_send for a new mail",
            )
        })?;
    let subject = if inbound.subject.to_ascii_lowercase().starts_with("re:") {
        inbound.subject.clone()
    } else {
        format!("Re: {}", inbound.subject)
    };
    let mut references = inbound.references.clone();
    let parent = format!("<{}>", inbound.message_id);
    if !inbound.message_id.is_empty() {
        references.push(parent.clone());
    }
    let start = references.len().saturating_sub(20);
    let composed = Composed {
        to: vec![inbound.reply_to],
        cc: Vec::new(),
        subject,
        text: mail.text,
        html: mail.html,
        attachments: mail.attachments,
        in_reply_to: (!inbound.message_id.is_empty()).then_some(parent),
        references: references[start..].to_vec(),
    };
    deliver(state, &bot, &from, &thread.id, composed).await
}

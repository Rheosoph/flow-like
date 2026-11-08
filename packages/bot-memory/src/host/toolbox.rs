//! The memory tools of a headless turn: one dispatcher over the engine's stores and the inbox
//! reader. Answers are JSON text for the model; failures are answers too, never panics or
//! errors that end the turn.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::Utc;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::session::MemorySession;
use crate::engine::MemoryBusy;
use crate::facts::{RejectReason, UpdateAction};
use crate::model::{HistoryItem, ItemKind, Role};
use crate::tools::{
    self, HistoryReadArgs, HistoryReader, HistorySearchArgs, MemoryForgetArgs, MemorySaveArgs,
    MemorySearchArgs, MessageGetArgs, SummaryExpandArgs, ToolOutput, WriteGate, WriteOutcome,
};

pub const TOOL_NAMES: [&str; 7] = [
    "memory_search",
    "history_search",
    "history_read",
    "message_get",
    "summary_expand",
    "memory_save",
    "memory_forget",
];

pub struct MemoryToolbox {
    session: Arc<MemorySession>,
    reader: Arc<dyn HistoryReader>,
    trigger: HistoryItem,
    foreign_calls: AtomicBool,
    wrote: Arc<AtomicBool>,
}

fn reply(value: Value) -> String {
    value.to_string()
}

fn failure(code: &str, message: impl std::fmt::Display) -> String {
    reply(json!({ "status": "error", "code": code, "message": message.to_string() }))
}

fn output(output: ToolOutput) -> String {
    reply(json!({
        "status": "ok",
        "text": output.text,
        "next": output.next,
        "truncated": output.truncated,
    }))
}

fn reason(reason: RejectReason) -> &'static str {
    match reason {
        RejectReason::Unsafe => {
            "That text holds only links, images or instructions, which are not kept as memories."
        }
        RejectReason::NoEvidence => "There is no message to point to for that memory.",
        RejectReason::InvalidMetadata => "The location, time or TTL is invalid; nothing was saved.",
        RejectReason::UntrustedPersonal => {
            "Only what the user said themselves can be saved as a profile fact or preference."
        }
    }
}

impl MemoryToolbox {
    /// `trigger` is the message the turn answers: the evidence of what the bot saves.
    pub fn new(
        session: Arc<MemorySession>,
        reader: Arc<dyn HistoryReader>,
        trigger: HistoryItem,
    ) -> Self {
        Self {
            session,
            reader,
            trigger,
            foreign_calls: AtomicBool::new(false),
            wrote: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Raised when `memory_save` or `memory_forget` changed the memory, so a failed attempt is
    /// not repeated.
    pub fn with_write_flag(mut self, wrote: Arc<AtomicBool>) -> Self {
        self.wrote = wrote;
        self
    }

    pub fn handles(name: &str) -> bool {
        TOOL_NAMES.contains(&name)
    }

    /// Another tool ran in this turn: web or tool content is in the context, so writes wait
    /// for the user to confirm in a later turn.
    pub fn note_foreign_call(&self) {
        self.foreign_calls.store(true, Ordering::SeqCst);
    }

    pub fn wrote(&self) -> bool {
        self.wrote.load(Ordering::SeqCst)
    }

    fn gate(&self) -> WriteGate {
        WriteGate {
            untrusted_in_context: self.foreign_calls.load(Ordering::SeqCst)
                || self.trigger.role != Role::User,
            confirmed: false,
        }
    }

    pub async fn call(&self, name: &str, arguments: Value) -> String {
        match self.dispatch(name, arguments).await {
            Ok(answer) => answer,
            Err(error) => {
                if let Some(busy) = error.downcast_ref::<MemoryBusy>() {
                    return failure("memory_busy", busy);
                }
                failure("memory_tool_failed", format!("{error:#}"))
            }
        }
    }

    async fn dispatch(&self, name: &str, arguments: Value) -> flow_like_types::Result<String> {
        let engine = self.session.engine();
        let tz = engine.timezone();
        let now = Utc::now();
        match name {
            "memory_search" => Ok(output(
                tools::memory_search(
                    engine.stores(),
                    &args::<MemorySearchArgs>(arguments)?,
                    now,
                    tz,
                )
                .await?,
            )),
            "history_search" => Ok(output(
                tools::history_search(
                    &engine.stores().history,
                    &args::<HistorySearchArgs>(arguments)?,
                    now,
                    tz,
                )
                .await?,
            )),
            "history_read" => Ok(output(
                tools::history_read(
                    self.reader.as_ref(),
                    &args::<HistoryReadArgs>(arguments)?,
                    tz,
                )
                .await?,
            )),
            "message_get" => Ok(output(
                tools::message_get(
                    self.reader.as_ref(),
                    &args::<MessageGetArgs>(arguments)?,
                    tz,
                )
                .await?,
            )),
            "summary_expand" => Ok(output(
                tools::summary_expand(
                    &engine.stores().segments,
                    &args::<SummaryExpandArgs>(arguments)?,
                    tz,
                )
                .await?,
            )),
            "memory_save" => self.save(args::<MemorySaveArgs>(arguments)?, now).await,
            "memory_forget" => self.forget(args::<MemoryForgetArgs>(arguments)?, now).await,
            other => Ok(failure(
                "memory_tool_unknown",
                format!("'{other}' is not a memory tool"),
            )),
        }
    }

    async fn save(
        &self,
        arguments: MemorySaveArgs,
        now: chrono::DateTime<Utc>,
    ) -> flow_like_types::Result<String> {
        let evidence = match arguments.evidence_seq {
            Some(seq) => {
                if seq == 0 || seq > self.trigger.seq {
                    return Ok(failure(
                        "memory_evidence_invalid",
                        "Memory evidence must be an existing message at or before the current message in this conversation.",
                    ));
                }
                let Some(item) = self.reader.message(seq).await?.filter(|item| {
                    !item.deleted
                        && item.kind == ItemKind::Message
                        && item.speaker.is_some()
                        && matches!(item.role, Role::User | Role::Assistant | Role::External)
                }) else {
                    return Ok(failure(
                        "memory_evidence_invalid",
                        "That position does not contain a live participant message in this conversation.",
                    ));
                };
                if self.trigger.role != Role::User && item.role == Role::User {
                    return Ok(failure(
                        "memory_evidence_owner_only",
                        "Only the owner may save a memory using the owner's words as its evidence. Save the participant's own statement instead.",
                    ));
                }
                item
            }
            None => self.trigger.clone(),
        };
        let mut gate = self.gate();
        // A room participant may contribute an attributed statement. Outside tool content
        // still waits for confirmation, and forgetting remains restricted to the owner.
        if evidence.role == Role::External
            && evidence.speaker.is_some()
            && (self.trigger.role == Role::User
                || (self.trigger.role == Role::External && self.trigger.speaker.is_some()))
            && !self.foreign_calls.load(Ordering::SeqCst)
        {
            gate.untrusted_in_context = false;
        }
        let outcome =
            tools::memory_save(self.session.engine(), &arguments, &evidence, gate, now).await?;
        Ok(match outcome {
            WriteOutcome::Done((fact, action)) => {
                if !matches!(action, UpdateAction::Noop) {
                    self.wrote.store(true, Ordering::SeqCst);
                }
                reply(json!({
                    "status": "ok",
                    "text": format!("Remembered ({}): {} [{}]", action_label(&action), fact.text, fact.id),
                    "id": fact.id,
                    "geojson": fact.geojson,
                    "time": fact.time,
                    "expires_at": fact.expires_at,
                }))
            }
            WriteOutcome::NeedsConfirmation { preview } => reply(json!({
                "status": "needs_confirmation",
                "preview": preview,
                "message": "Web or tool content is part of this turn, so nothing was saved. Tell the user what you would remember and ask them to confirm; save it when they do.",
            })),
            WriteOutcome::Rejected { reason: why } => reply(json!({
                "status": "rejected",
                "message": reason(why),
            })),
        })
    }

    async fn forget(
        &self,
        arguments: MemoryForgetArgs,
        now: chrono::DateTime<Utc>,
    ) -> flow_like_types::Result<String> {
        let outcome =
            tools::memory_forget(self.session.engine(), &arguments, self.gate(), now).await?;
        Ok(match outcome {
            WriteOutcome::Done(fact) => {
                self.wrote.store(true, Ordering::SeqCst);
                reply(json!({ "status": "ok", "text": format!("Forgot: {}", fact.text) }))
            }
            WriteOutcome::NeedsConfirmation { preview } => reply(json!({
                "status": "needs_confirmation",
                "preview": preview,
                "message": "Web or tool content is part of this turn, so nothing was forgotten. Ask the user to confirm first.",
            })),
            WriteOutcome::Rejected { reason: why } => reply(json!({
                "status": "rejected",
                "message": reason(why),
            })),
        })
    }
}

fn args<T: DeserializeOwned>(arguments: Value) -> flow_like_types::Result<T> {
    serde_json::from_value(arguments)
        .map_err(|error| flow_like_types::anyhow!("The tool arguments are invalid: {error}"))
}

fn action_label(action: &UpdateAction) -> &'static str {
    match action {
        UpdateAction::Add => "added",
        UpdateAction::Merge { .. } => "merged into an existing memory",
        UpdateAction::Supersede { .. } => "replaces an older memory",
        UpdateAction::AddSuperseded { .. } => "added as history",
        UpdateAction::Noop => "already known",
    }
}

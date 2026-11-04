//! The approval gate of a headless bot turn: a call that asks the owner runs under a standing
//! grant (or the owner's one-time answer) and otherwise parks the turn behind an approval card.
//! Tools consult it where an open chat would show a prompt; once it parks, it stops the run.

use flow_like::flow::copilot::bot_talk::{CrewMember, HeadlessPermission};
use flow_like::flow::copilot::tool_grants::{
    GrantCheck, GrantStore, HeadlessAnswer, approval_request, check_call,
    documents_expensive_granted, expensive_documents_request, headless_answer, take_once,
};
use flow_like_types::bot_message::ApprovalRequest;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex, PoisonError};

pub const CODE_WAITING: &str = "bot_tool_waiting_for_approval";
pub const CODE_REFUSED: &str = "bot_tool_refused_by_owner";
pub const ASK_BOT_TOOL: &str = "ask_bot";
const QUESTION_PREVIEW_CHARS: usize = 300;

pub fn ask_bot_scope(callee_id: &str) -> String {
    format!("bot:{callee_id}")
}

pub struct ApprovalGate {
    grants: Arc<dyn GrantStore>,
    turn_id: String,
    parked: Mutex<Option<ApprovalRequest>>,
    stop: Box<dyn Fn() + Send + Sync>,
}

impl std::fmt::Debug for ApprovalGate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApprovalGate")
            .field("turn_id", &self.turn_id)
            .finish()
    }
}

fn result(tool: &str, code: &str, message: &str) -> Value {
    json!({ "status": "error", "tool": tool, "code": code, "retryable": false, "message": message })
}

impl ApprovalGate {
    pub fn new(
        grants: Arc<dyn GrantStore>,
        turn_id: &str,
        stop: impl Fn() + Send + Sync + 'static,
    ) -> Arc<Self> {
        Arc::new(Self {
            grants,
            turn_id: turn_id.to_string(),
            parked: Mutex::new(None),
            stop: Box::new(stop),
        })
    }

    pub fn grants(&self) -> &Arc<dyn GrantStore> {
        &self.grants
    }

    pub fn turn_id(&self) -> &str {
        &self.turn_id
    }

    /// The card of the call that parked the turn.
    pub fn parked(&self) -> Option<ApprovalRequest> {
        self.parked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn park(&self, tool: &str, card: ApprovalRequest) -> Value {
        self.parked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_or_insert(card);
        (self.stop)();
        result(
            tool,
            CODE_WAITING,
            "Your owner was asked to approve this call. This turn pauses and continues after their answer. Call no further tools and write nothing more now.",
        )
    }

    /// `Ok` when the call may run; `Err` is the tool result for the model.
    pub async fn clear(
        &self,
        tool: &str,
        args: &Value,
        context_app: Option<&str>,
        description: &str,
    ) -> Result<(), Value> {
        if self.parked().is_some() {
            return Err(result(tool, CODE_WAITING, "This turn is waiting for an approval."));
        }
        match headless_answer(
            self.grants.as_ref(),
            &self.turn_id,
            tool,
            args,
            context_app,
            description,
        )
        .await
        {
            HeadlessAnswer::Run => Ok(()),
            HeadlessAnswer::Refuse(message) => Err(result(tool, CODE_REFUSED, &message)),
            HeadlessAnswer::Park(card) => Err(self.park(tool, card)),
        }
    }

    /// A first ask of `callee` by a headless bot: the owner's earlier answer decides, else the
    /// turn parks behind a "<caller> wants to ask <callee>" card. The answer is per callee, so
    /// a rephrased question after "Allow once" still finds it.
    pub async fn ask_bot(
        &self,
        caller: &CrewMember,
        callee: &CrewMember,
        question: &str,
    ) -> HeadlessPermission {
        if self.parked().is_some() {
            return HeadlessPermission::Parked;
        }
        let args = json!({ "bot": callee.id });
        let scope = ask_bot_scope(&callee.id);
        if self.grants.has(ASK_BOT_TOOL, &scope).await.unwrap_or(false) {
            return HeadlessPermission::Allowed;
        }
        match check_call(self.grants.as_ref(), ASK_BOT_TOOL, &args, None).await {
            Ok(check) if check.runs() => HeadlessPermission::Allowed,
            Ok(GrantCheck::Denied) => HeadlessPermission::Denied,
            Ok(_) => {
                let preview: String = question.chars().take(QUESTION_PREVIEW_CHARS).collect();
                let summary = format!("{} wants to ask {}: {preview}", caller.name, callee.name);
                let mut card =
                    approval_request(&self.turn_id, ASK_BOT_TOOL, &args, None, &summary);
                card.scope = scope;
                self.park(ASK_BOT_TOOL, card);
                HeadlessPermission::Parked
            }
            Err(_) => HeadlessPermission::Unavailable,
        }
    }

    /// The owner's "Always allow" on an ask card: `store_grant` makes it a standing permission
    /// between the two bots and the card's grant is spent. `false` when no such answer exists.
    pub async fn settle_ask_card(
        &self,
        callee: &CrewMember,
        store_grant: impl std::future::Future<Output = flow_like_types::Result<()>>,
    ) -> flow_like_types::Result<bool> {
        let scope = ask_bot_scope(&callee.id);
        if !self.grants.has(ASK_BOT_TOOL, &scope).await.unwrap_or(false) {
            return Ok(false);
        }
        store_grant.await?;
        self.grants.delete(ASK_BOT_TOOL, &scope).await?;
        Ok(true)
    }

    /// Whether a call above the document cost limits may run: the owner granted it, or answered
    /// this very call once.
    pub async fn expensive_documents_allowed(&self, tool: &str, args: &Value) -> bool {
        documents_expensive_granted(self.grants.as_ref())
            .await
            .unwrap_or(false)
            || take_once(self.grants.as_ref(), tool, args)
                .await
                .unwrap_or(false)
    }

    /// Parks the turn for a document call that refused for its cost; the tool result otherwise.
    pub fn park_for_cost(&self, tool: &str, args: &Value, outcome: Value) -> Value {
        if outcome["code"] != flow_like_bot_tools::error::code::NEEDS_APPROVAL {
            return outcome;
        }
        let message = outcome["message"].as_str().unwrap_or("This document call is above the cost limits.");
        self.park(tool, expensive_documents_request(&self.turn_id, tool, args, message))
    }
}

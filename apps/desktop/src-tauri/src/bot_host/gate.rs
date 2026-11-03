//! The approval gate of a headless bot turn: a call that asks the owner runs under a standing
//! grant (or the owner's one-time answer) and otherwise parks the turn behind an approval card.
//! Tools consult it where an open chat would show a prompt; once it parks, it stops the run.

use flow_like::flow::copilot::tool_grants::{
    GrantStore, HeadlessAnswer, documents_expensive_granted, expensive_documents_request,
    headless_answer, take_once,
};
use flow_like_types::bot_message::ApprovalRequest;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex, PoisonError};

pub const CODE_WAITING: &str = "bot_tool_waiting_for_approval";
pub const CODE_REFUSED: &str = "bot_tool_refused_by_owner";

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

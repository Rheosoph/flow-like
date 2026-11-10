//! The approval gate of a headless bot turn: a call that asks the owner runs under a standing
//! grant, a one-time answer or an inferred owner instruction. Otherwise it parks behind a card.
//! Tools consult it where an open chat would show a prompt. Once it parks, every later call that
//! asks waits too, and the run pauses after its current tool batch has settled.

use flow_like::flow::copilot::bot_talk::{CrewMember, HeadlessPermission};
use flow_like::flow::copilot::tool_authorization::AuthorizationInference;
use flow_like::flow::copilot::tool_grants::{
    GrantCheck, GrantStore, HeadlessAnswer, approval_request, check_call,
    documents_expensive_granted, expensive_documents_request, headless_answer_with_inference,
    take_once,
};
use flow_like_types::bot_message::ApprovalRequest;
use flow_like_types::tool_grant::args_hash;
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    sync::{Arc, Mutex, PoisonError},
};

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
    parked: Mutex<Option<ParkedCall>>,
    inference: Mutex<Option<Arc<dyn AuthorizationInference>>>,
    refused: Mutex<HashSet<String>>,
    approval_lock: tokio::sync::Mutex<()>,
}

struct ParkedCall {
    card: ApprovalRequest,
    recovery_call_id: Option<String>,
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

/// The run pauses only on this gate shape (`status: "waiting"`), the same one the server gate uses.
fn waiting(tool: &str, message: &str) -> Value {
    json!({ "status": "waiting", "tool": tool, "code": CODE_WAITING, "retryable": false, "message": message })
}

impl ApprovalGate {
    pub fn new(grants: Arc<dyn GrantStore>, turn_id: &str) -> Arc<Self> {
        Arc::new(Self {
            grants,
            turn_id: turn_id.to_string(),
            parked: Mutex::new(None),
            inference: Mutex::new(None),
            refused: Mutex::new(HashSet::new()),
            approval_lock: tokio::sync::Mutex::new(()),
        })
    }

    pub fn grants(&self) -> &Arc<dyn GrantStore> {
        &self.grants
    }

    pub fn turn_id(&self) -> &str {
        &self.turn_id
    }

    /// The host may replace its inference context after receiving authenticated owner input.
    pub fn set_inference(&self, inference: Option<Arc<dyn AuthorizationInference>>) {
        *self
            .inference
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = inference;
    }

    /// The card of the call that parked the turn.
    pub fn parked(&self) -> Option<ApprovalRequest> {
        self.parked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map(|parked| parked.card.clone())
    }

    /// The exact runtime call that this gate refused before execution.
    pub fn unexecuted_call(&self) -> Option<(String, String)> {
        self.parked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .and_then(|parked| {
                parked
                    .recovery_call_id
                    .clone()
                    .map(|id| (id, parked.card.tool.clone()))
            })
    }

    fn park(&self, tool: &str, card: ApprovalRequest) -> Value {
        self.parked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_or_insert_with(|| ParkedCall {
                card,
                recovery_call_id: flow_like::flow::copilot::run_recovery::current_recovery_call_id(
                ),
            });
        waiting(
            tool,
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
        // A consumed denial must be recorded before another call can check its token.
        let _approval = self.approval_lock.lock().await;
        if self.parked().is_some() {
            return Err(waiting(tool, "This turn is waiting for an approval."));
        }
        if self
            .refused
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(&args_hash(tool, args))
        {
            return Err(result(
                tool,
                CODE_REFUSED,
                "The owner refused this call during this turn. Do not try it again.",
            ));
        }
        match headless_answer_with_inference(
            self.grants.as_ref(),
            &self.turn_id,
            tool,
            args,
            context_app,
            description,
            Some(self),
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
        let _approval = self.approval_lock.lock().await;
        if self.parked().is_some() {
            return HeadlessPermission::Parked;
        }
        let args = json!({ "bot": callee.id });
        if self
            .refused
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(&args_hash(ASK_BOT_TOOL, &args))
        {
            return HeadlessPermission::Denied;
        }
        let scope = ask_bot_scope(&callee.id);
        match check_call(self.grants.as_ref(), ASK_BOT_TOOL, &args, None).await {
            Ok(check) if check.runs() => HeadlessPermission::Allowed,
            Ok(GrantCheck::Denied) => {
                self.record_refusal(ASK_BOT_TOOL, &args, None);
                HeadlessPermission::Denied
            }
            Ok(_) => {
                if self.grants.has(ASK_BOT_TOOL, &scope).await.unwrap_or(false) {
                    return HeadlessPermission::Allowed;
                }
                let preview: String = question.chars().take(QUESTION_PREVIEW_CHARS).collect();
                let summary = format!("{} wants to ask {}: {preview}", caller.name, callee.name);
                // Inspect the entire question; the explicit answer's stable scope still names
                // the callee, so a one-time answer survives a rephrased question.
                let inferred_args = json!({"bot":callee.id,"question":question});
                if tokio::time::timeout(
                    flow_like::flow::copilot::tool_authorization::DECISION_TIMEOUT,
                    self.infer(ASK_BOT_TOOL, &inferred_args, None, &summary),
                )
                .await
                .unwrap_or(false)
                {
                    return HeadlessPermission::Allowed;
                }
                let mut card = approval_request(&self.turn_id, ASK_BOT_TOOL, &args, None, &summary);
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
        let message = outcome["message"]
            .as_str()
            .unwrap_or("This document call is above the cost limits.");
        self.park(
            tool,
            expensive_documents_request(&self.turn_id, tool, args, message),
        )
    }
}

#[async_trait::async_trait]
impl AuthorizationInference for ApprovalGate {
    async fn infer(
        &self,
        tool: &str,
        args: &Value,
        context_app: Option<&str>,
        description: &str,
    ) -> bool {
        let inference = self
            .inference
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        match inference {
            Some(inference) => inference.infer(tool, args, context_app, description).await,
            None => false,
        }
    }

    fn record_refusal(&self, tool: &str, args: &Value, context_app: Option<&str>) {
        self.refused
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(args_hash(tool, args));
        let inference = self
            .inference
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(inference) = inference {
            inference.record_refusal(tool, args, context_app);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like::flow::copilot::tool_grants::GrantKey;
    use flow_like_types::tool_grant::DENY_PREFIX;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Grants(Mutex<Vec<GrantKey>>);

    #[async_trait::async_trait]
    impl GrantStore for Grants {
        async fn list(&self) -> flow_like_types::Result<Vec<GrantKey>> {
            Ok(self.0.lock().unwrap().clone())
        }
        async fn put(&self, tool: &str, scope: &str) -> flow_like_types::Result<()> {
            self.0.lock().unwrap().push(GrantKey::new(tool, scope));
            Ok(())
        }
        async fn delete(&self, tool: &str, scope: &str) -> flow_like_types::Result<bool> {
            let mut grants = self.0.lock().unwrap();
            let before = grants.len();
            grants.retain(|key| key.tool != tool || key.scope != scope);
            Ok(grants.len() != before)
        }
    }

    #[derive(Default)]
    struct Allow(AtomicUsize);

    #[async_trait::async_trait]
    impl AuthorizationInference for Allow {
        async fn infer(&self, _: &str, _: &Value, _: Option<&str>, _: &str) -> bool {
            self.0.fetch_add(1, Ordering::SeqCst);
            true
        }
    }

    #[tokio::test]
    async fn a_consumed_denial_cannot_be_reinferred_on_a_repeat_call() {
        let args = json!({"thread":"t", "body":"Reply"});
        let grants = Arc::new(Grants(Mutex::new(vec![GrantKey::new(
            "email_reply",
            format!("{DENY_PREFIX}{}", args_hash("email_reply", &args)),
        )])));
        let gate = ApprovalGate::new(grants.clone(), "turn");
        let inference = Arc::new(Allow::default());
        gate.set_inference(Some(inference.clone()));
        let (first, second) = tokio::join!(
            gate.clear("email_reply", &args, None, "Reply"),
            gate.clear("email_reply", &args, None, "Reply"),
        );
        assert_eq!(first.unwrap_err()["code"], CODE_REFUSED);
        assert_eq!(second.unwrap_err()["code"], CODE_REFUSED);
        assert_eq!(inference.0.load(Ordering::SeqCst), 0);
        assert!(grants.list().await.unwrap().is_empty());
        assert!(gate.parked().is_none());
    }

    #[tokio::test]
    async fn inference_runs_a_call_without_creating_a_card_or_grant() {
        let grants = Arc::new(Grants(Mutex::new(Vec::new())));
        let gate = ApprovalGate::new(grants.clone(), "turn");
        let inference = Arc::new(Allow::default());
        gate.set_inference(Some(inference.clone()));
        gate.clear(
            "email_reply",
            &json!({"thread":"t", "body":"Reply"}),
            None,
            "Reply",
        )
        .await
        .unwrap();
        assert_eq!(inference.0.load(Ordering::SeqCst), 1);
        assert!(grants.list().await.unwrap().is_empty());
        assert!(gate.parked().is_none());
    }

    #[tokio::test]
    async fn inferred_bot_consultation_still_respects_an_owner_refusal() {
        let caller = CrewMember {
            id: "a".into(),
            name: "A".into(),
            ..Default::default()
        };
        let callee = CrewMember {
            id: "b".into(),
            name: "B".into(),
            ..Default::default()
        };
        let args = json!({"bot":"b"});
        let grants = Arc::new(Grants(Mutex::new(Vec::new())));
        let gate = ApprovalGate::new(grants.clone(), "turn");
        let inference = Arc::new(Allow::default());
        gate.set_inference(Some(inference.clone()));
        assert!(matches!(
            gate.ask_bot(&caller, &callee, "Check this report").await,
            HeadlessPermission::Allowed
        ));
        grants.put(ASK_BOT_TOOL, &ask_bot_scope("b")).await.unwrap();
        grants
            .put(
                ASK_BOT_TOOL,
                &format!("{DENY_PREFIX}{}", args_hash(ASK_BOT_TOOL, &args)),
            )
            .await
            .unwrap();
        assert!(matches!(
            gate.ask_bot(&caller, &callee, "Check this report").await,
            HeadlessPermission::Denied
        ));
        assert!(matches!(
            gate.ask_bot(&caller, &callee, "Rephrased report question")
                .await,
            HeadlessPermission::Denied
        ));
        assert_eq!(inference.0.load(Ordering::SeqCst), 1);
        assert!(gate.parked().is_none());
    }
}

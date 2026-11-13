//! The approval gate of a headless bot turn: a call that asks the owner runs under a standing
//! grant, a one-time answer or an inferred owner instruction. Otherwise it joins the attempt's
//! [`ParkedBatch`], the one card the turn parks behind. Tools consult it where an open chat would
//! show a prompt. A park stops nothing else: later calls are checked on their own, and the run
//! pauses after its round has settled.

use flow_like::flow::copilot::bot_talk::{CrewMember, HeadlessPermission};
use flow_like::flow::copilot::platform::Clearance;
use flow_like::flow::copilot::tool_authorization::AuthorizationInference;
use flow_like::flow::copilot::tool_grants::{
    GrantCheck, GrantStore, HeadlessAnswer, ParkedBatch, approval_request, check_call,
    documents_expensive_granted, expensive_documents_request, take_once,
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

/// The tools a card asks about, each once, for a failure text.
pub fn card_tools(card: &ApprovalRequest) -> String {
    let mut tools: Vec<String> = Vec::new();
    for item in card.batch() {
        if !tools.contains(&item.tool) {
            tools.push(item.tool);
        }
    }
    tools.join(", ")
}

pub struct ApprovalGate {
    grants: Arc<dyn GrantStore>,
    batch: ParkedBatch,
    inference: Mutex<Option<Arc<dyn AuthorizationInference>>>,
    refused: Mutex<HashSet<String>>,
    approval_lock: tokio::sync::Mutex<()>,
}

impl std::fmt::Debug for ApprovalGate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApprovalGate")
            .field("batch", &self.batch)
            .finish()
    }
}

fn result(tool: &str, code: &str, message: &str) -> Value {
    json!({ "status": "error", "tool": tool, "code": code, "retryable": false, "message": message })
}

/// The run pauses only on this gate shape (`status: "waiting"`), the same one the server gate uses.
fn waiting(tool: &str) -> Value {
    json!({
        "status": "waiting", "tool": tool, "code": CODE_WAITING, "retryable": false,
        "message": "Your owner was asked to approve this call. This turn pauses and continues after their answer. Call no further tools and write nothing more now.",
    })
}

impl ApprovalGate {
    pub fn new(grants: Arc<dyn GrantStore>, turn_id: &str) -> Arc<Self> {
        Arc::new(Self {
            grants,
            batch: ParkedBatch::new(turn_id),
            inference: Mutex::new(None),
            refused: Mutex::new(HashSet::new()),
            approval_lock: tokio::sync::Mutex::new(()),
        })
    }

    pub fn grants(&self) -> &Arc<dyn GrantStore> {
        &self.grants
    }

    pub fn turn_id(&self) -> &str {
        self.batch.turn_id()
    }

    /// The host may replace its inference context after receiving authenticated owner input.
    pub fn set_inference(&self, inference: Option<Arc<dyn AuthorizationInference>>) {
        *self
            .inference
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = inference;
    }

    /// The card the turn parks behind: every call of this attempt that waits for the owner.
    pub fn parked(&self) -> Option<ApprovalRequest> {
        self.batch.card()
    }

    /// `(call id, tool)` of every call this gate held back before it ran.
    pub fn held_calls(&self) -> Vec<(String, String)> {
        self.batch.held_calls()
    }

    fn park(&self, tool: &str, card: ApprovalRequest) -> Value {
        self.batch.park(card, false);
        waiting(tool)
    }

    /// The tool result of a call the owner refused earlier in this turn.
    fn refused_before(&self, tool: &str, args: &Value) -> Option<Value> {
        self.refused
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(&args_hash(tool, args))
            .then(|| {
                result(
                    tool,
                    CODE_REFUSED,
                    "The owner refused this call during this turn. Do not try it again.",
                )
            })
    }

    /// `Ok` when the call may run; `Err` is the tool result for the model. A call that asks joins
    /// the turn's card.
    pub async fn clear(
        &self,
        tool: &str,
        args: &Value,
        context_app: Option<&str>,
        description: &str,
    ) -> Result<(), Value> {
        // A consumed denial must be recorded before another call can check its token.
        let _approval = self.approval_lock.lock().await;
        if let Some(refused) = self.refused_before(tool, args) {
            return Err(refused);
        }
        match self
            .batch
            .answer(
                self.grants.as_ref(),
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
            HeadlessAnswer::Park(_) => Err(waiting(tool)),
        }
    }

    /// The gate of a call its round held back after a park, asked without running it: no
    /// one-time answer is spent, a refusal is the call's result and a call that asks joins the
    /// card.
    pub async fn clearance(
        &self,
        tool: &str,
        args: &Value,
        context_app: Option<&str>,
        description: &str,
    ) -> Clearance {
        let _approval = self.approval_lock.lock().await;
        if let Some(refused) = self.refused_before(tool, args) {
            return Clearance::Refused(refused.to_string());
        }
        match self
            .batch
            .clearance(
                self.grants.as_ref(),
                tool,
                args,
                context_app,
                description,
                Some(self),
            )
            .await
        {
            HeadlessAnswer::Run => Clearance::Clear,
            HeadlessAnswer::Refuse(message) => {
                Clearance::Refused(result(tool, CODE_REFUSED, &message).to_string())
            }
            HeadlessAnswer::Park(_) => Clearance::Asks,
        }
    }

    /// A first ask of `callee` by a headless bot: the owner's earlier answer decides, else a
    /// "<caller> wants to ask <callee>" item joins the turn's card. The answer is per callee, so
    /// a rephrased question after "Allow once" still finds it.
    pub async fn ask_bot(
        &self,
        caller: &CrewMember,
        callee: &CrewMember,
        question: &str,
    ) -> HeadlessPermission {
        let _approval = self.approval_lock.lock().await;
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
                let mut card =
                    approval_request(self.turn_id(), ASK_BOT_TOOL, &args, None, &summary);
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

    /// Puts a document call that refused for its cost on the turn's card; the tool result
    /// otherwise.
    pub fn park_for_cost(&self, tool: &str, args: &Value, outcome: Value) -> Value {
        if outcome["code"] != flow_like_bot_tools::error::code::NEEDS_APPROVAL {
            return outcome;
        }
        let message = outcome["message"]
            .as_str()
            .unwrap_or("This document call is above the cost limits.");
        self.park(
            tool,
            expensive_documents_request(self.turn_id(), tool, args, message),
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
    use flow_like::flow::copilot::run_recovery::{CallScope, scope_call};
    use flow_like::flow::copilot::tool_grants::{GrantKey, grant_scopes};
    use flow_like_types::bot_message::APPROVAL_BATCH_TOOL;
    use flow_like_types::tool_grant::{DENY_PREFIX, ONCE_PREFIX, SCOPE_DOCUMENTS_EXPENSIVE};
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn call(id: &str, round_web_content: bool) -> CallScope {
        CallScope {
            id: id.into(),
            round_web_content,
            held_replay: false,
        }
    }

    fn token(prefix: &str, tool: &str, args: &Value) -> GrantKey {
        GrantKey::new(tool, format!("{prefix}{}", args_hash(tool, args)))
    }

    fn held(calls: &[(&str, &str)]) -> Vec<(String, String)> {
        calls
            .iter()
            .map(|(id, tool)| ((*id).into(), (*tool).into()))
            .collect()
    }

    fn member(id: &str) -> CrewMember {
        CrewMember {
            id: id.into(),
            name: id.to_uppercase(),
            ..Default::default()
        }
    }

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

    #[tokio::test]
    async fn a_later_asking_call_joins_the_batch() {
        let first = json!({"thread":"t", "body":"Reply"});
        let second = json!({"thread":"u", "body":"Other reply"});
        let drop = json!({"index":"notes"});
        let grants = Arc::new(Grants(Mutex::new(
            grant_scopes("knowledge_drop", &drop, None)
                .unwrap()
                .into_iter()
                .map(|scope| GrantKey::new("knowledge_drop", scope))
                .collect(),
        )));
        let gate = ApprovalGate::new(grants, "turn");
        let parked = scope_call(
            call("c1", false),
            gate.clear("email_reply", &first, None, "Reply"),
        )
        .await;
        assert_eq!(parked.unwrap_err()["status"], "waiting");
        scope_call(
            call("c2", false),
            gate.clear("knowledge_drop", &drop, None, "Drop"),
        )
        .await
        .expect("a granted call runs after a park");
        let joined = scope_call(
            call("c3", false),
            gate.clear("email_reply", &second, None, "Other"),
        )
        .await;
        assert_eq!(joined.unwrap_err()["code"], CODE_WAITING);
        let card = gate.parked().unwrap();
        assert_eq!(card.tool, APPROVAL_BATCH_TOOL);
        assert_eq!(
            card.items
                .iter()
                .map(|item| item.args_hash.clone())
                .collect::<Vec<_>>(),
            [
                args_hash("email_reply", &first),
                args_hash("email_reply", &second)
            ]
        );
        assert_eq!(
            gate.held_calls(),
            held(&[("c1", "email_reply"), ("c3", "email_reply")])
        );
    }

    #[tokio::test]
    async fn clearance_spends_no_token() {
        let once = json!({"thread":"o", "body":"Once"});
        let denied = json!({"thread":"d", "body":"Denied"});
        let asking = json!({"thread":"a", "body":"Ask"});
        let grants = Arc::new(Grants(Mutex::new(vec![
            token(ONCE_PREFIX, "email_reply", &once),
            token(DENY_PREFIX, "email_reply", &denied),
        ])));
        let gate = ApprovalGate::new(grants.clone(), "turn");
        let clear = scope_call(
            call("c1", false),
            gate.clearance("email_reply", &once, None, "Once"),
        )
        .await;
        assert_eq!(clear, Clearance::Clear);
        let Clearance::Refused(refused) = scope_call(
            call("c2", false),
            gate.clearance("email_reply", &denied, None, "Denied"),
        )
        .await
        else {
            panic!("a refused call is refused at clearance");
        };
        assert_eq!(
            serde_json::from_str::<Value>(&refused).unwrap()["code"],
            CODE_REFUSED
        );
        assert_eq!(
            grants.list().await.unwrap(),
            [token(ONCE_PREFIX, "email_reply", &once)],
            "clearance spends the refusal but not the one-time answer"
        );
        let asks = scope_call(
            call("c3", false),
            gate.clearance("email_reply", &asking, None, "Ask"),
        )
        .await;
        assert_eq!(asks, Clearance::Asks);
        let card = gate.parked().unwrap();
        assert_eq!(card.args_hash, args_hash("email_reply", &asking));
        assert_eq!(gate.held_calls(), held(&[("c3", "email_reply")]));
        gate.clear("email_reply", &once, None, "Once")
            .await
            .expect("the one-time answer runs the call");
        assert!(grants.list().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn ask_bot_and_cost_join_the_batch() {
        let reply = json!({"thread":"t", "body":"Reply"});
        let document = json!({"path":"report.pdf"});
        let gate = ApprovalGate::new(Arc::new(Grants(Mutex::new(Vec::new()))), "turn");
        scope_call(
            call("c1", false),
            gate.clear("email_reply", &reply, None, "Reply"),
        )
        .await
        .unwrap_err();
        let asked = scope_call(
            call("c2", false),
            gate.ask_bot(&member("a"), &member("b"), "Check this report"),
        )
        .await;
        assert!(matches!(asked, HeadlessPermission::Parked));
        let costly = scope_call(call("c3", false), async {
            gate.park_for_cost(
                "file_summarize",
                &document,
                json!({
                    "code": flow_like_bot_tools::error::code::NEEDS_APPROVAL,
                    "message": "above the cost limits",
                }),
            )
        })
        .await;
        assert_eq!(costly["status"], "waiting");
        let card = gate.parked().unwrap();
        assert_eq!(
            card.items
                .iter()
                .map(|item| (item.tool.as_str(), item.scope.as_str()))
                .collect::<Vec<_>>(),
            [
                ("email_reply", "*"),
                (ASK_BOT_TOOL, "bot:b"),
                ("file_summarize", SCOPE_DOCUMENTS_EXPENSIVE),
            ]
        );
        assert_eq!(
            gate.held_calls(),
            held(&[
                ("c1", "email_reply"),
                ("c2", ASK_BOT_TOOL),
                ("c3", "file_summarize")
            ])
        );
    }

    #[tokio::test]
    async fn a_granted_web_request_parks_with_a_flagged_item_after_web_content() {
        let fetch = json!({"url":"https://example.com/news"});
        let grants = Arc::new(Grants(Mutex::new(
            grant_scopes("web_request", &fetch, None)
                .unwrap()
                .into_iter()
                .map(|scope| GrantKey::new("web_request", scope))
                .collect(),
        )));
        let gate = ApprovalGate::new(grants, "turn");
        scope_call(
            call("c1", false),
            gate.clear("web_request", &fetch, None, "Fetch"),
        )
        .await
        .expect("the grant covers the call before web content");
        assert!(gate.parked().is_none());
        let marked = scope_call(
            call("c2", true),
            gate.clear("web_request", &fetch, None, "Fetch"),
        )
        .await;
        assert_eq!(marked.unwrap_err()["status"], "waiting");
        let asks = scope_call(
            call("c3", true),
            gate.clearance("web_request", &fetch, None, "Fetch"),
        )
        .await;
        assert_eq!(asks, Clearance::Asks);
        let card = gate.parked().unwrap();
        assert_eq!(card.scope, "", "no standing grant is offered");
        assert_eq!(card.items.len(), 1);
        assert!(card.items[0].after_web_content);
        assert_eq!(card.items[0].calls, 2);
        assert_eq!(
            gate.held_calls(),
            held(&[("c2", "web_request"), ("c3", "web_request")])
        );
    }
}

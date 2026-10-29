use flow_like::{
    flow::{
        execution::{ExecutorApiAuth, context::ExecutionContext},
        node::{Node, NodeLogic, NodeScores},
        variable::VariableType,
    },
    hub::{HubRetry, hub_api_url, hub_response_error, send_hub_request},
};
use flow_like_types::{Value, anyhow, async_trait, json::json, reqwest};
use serde::de::DeserializeOwned;
use std::time::Duration;

use crate::remote_util::{log_shadow_refusal, refuse_in_shadow};

pub mod context;
pub mod lookup;

fn transient(status: reqwest::StatusCode) -> bool {
    matches!(
        status,
        reqwest::StatusCode::CONFLICT | reqwest::StatusCode::TOO_MANY_REQUESTS
    ) || status.is_server_error()
}

const TEAMS_RETRY: HubRetry = HubRetry {
    retryable: transient,
    retries: 3,
    max_wait: Duration::from_secs(60),
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum TeamsReply {
    Message,
    Card,
    Update,
}

fn teams_node(name: &str, label: &str, description: &str, alias: &str, scores: NodeScores) -> Node {
    let mut node = Node::new(name, label, description, "Events/Chat/Teams");
    node.set_flowscript_name("teams", alias);
    node.add_icon("/flow/icons/teams.svg");
    node.set_scores(scores);
    node
}

const ANY_SESSION: &str = "Local or global session from the Teams Chat Event";

fn add_session_pin(node: &mut Node, description: &str) {
    node.add_input_pin("session", "Session", description, VariableType::Struct)
        .set_open_schema();
}

fn node(reply: TeamsReply) -> Node {
    let (name, label, description, alias) = match reply {
        TeamsReply::Message => (
            "events_teams_send_message",
            "Send Teams Message",
            "Send a text reply to the Teams conversation of a Chat Event session",
            "sendMessage",
        ),
        TeamsReply::Card => (
            "events_teams_send_card",
            "Send Teams Card",
            "Send an Adaptive Card to the Teams conversation of a Chat Event session",
            "sendCard",
        ),
        TeamsReply::Update => (
            "events_teams_update_message",
            "Update Teams Message",
            "Replace the text of a message a Teams send node posted in this conversation",
            "updateMessage",
        ),
    };
    let mut node = teams_node(
        name,
        label,
        description,
        alias,
        NodeScores::new()
            .set_privacy(5)
            .set_security(6)
            .set_performance(6)
            .set_governance(6)
            .set_reliability(6)
            .set_cost(8)
            .build(),
    );
    node.add_input_pin(
        "exec_in",
        "In",
        "Send the Teams response",
        VariableType::Execution,
    );
    add_session_pin(&mut node, ANY_SESSION);
    node.add_input_pin(
        "text",
        "Text",
        "Message text with Teams Markdown formatting",
        VariableType::String,
    )
    .set_default_value(Some(json!("")));
    if reply == TeamsReply::Card {
        node.add_input_pin(
            "card",
            "Adaptive Card",
            "Adaptive Card JSON. Use Interaction nodes for forms and approvals that resume this run.",
            VariableType::Struct,
        )
        .set_open_schema();
    }
    if reply == TeamsReply::Update {
        node.add_input_pin(
            "input_message_id",
            "Message ID",
            "Message returned by a Teams send node in this conversation",
            VariableType::String,
        );
    }
    node.add_output_pin(
        "exec_out",
        "Out",
        "Teams accepted the response",
        VariableType::Execution,
    );
    node.add_output_pin(
        "message_id",
        "Message ID",
        "The Teams message identifier",
        VariableType::String,
    );
    node
}

fn session_id(session: &Value) -> flow_like_types::Result<&str> {
    session
        .pointer("/teams/session_id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("Connect the local or global session from a Teams Chat Event"))
}

struct TeamsCall {
    auth: ExecutorApiAuth,
    app_id: String,
    session_id: String,
    secure: bool,
}

impl TeamsCall {
    async fn bind(context: &mut ExecutionContext, action: &str) -> flow_like_types::Result<Self> {
        context.deactivate_exec_pin("exec_out").await?;
        if let Err(refusal) = refuse_in_shadow(context, action) {
            return Err(log_shadow_refusal(context, refusal));
        }
        let app = context
            .execution_cache
            .as_ref()
            .ok_or_else(|| anyhow!("Teams nodes require a server event execution"))?;
        let app_id = app.app_id.clone();
        let auth = context
            .executor_api_auth
            .clone()
            .ok_or_else(|| anyhow!("Teams nodes require an authenticated server execution"))?;
        let session: Value = context.evaluate_pin("session").await?;
        let session_id = session_id(&session)?.to_owned();
        if session_id != context.run_id() {
            return Err(anyhow!("This Teams session belongs to another execution"));
        }
        Ok(Self {
            auth,
            app_id,
            session_id,
            secure: context.profile.secure,
        })
    }

    async fn post<T: DeserializeOwned>(
        &self,
        route: &str,
        operation: &str,
        body: &Value,
        retry: HubRetry,
    ) -> flow_like_types::Result<T> {
        let url = hub_api_url(
            self.auth.api_url(),
            self.secure,
            &["execution", "apps", &self.app_id, "teams", route],
        )?;
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(135))
            .build()?;
        let (client_ref, url, token) = (&client, &url, self.auth.token());
        let response = send_hub_request(&client, retry, move || async move {
            Ok::<_, flow_like_types::Error>(
                client_ref
                    .post(url.clone())
                    .bearer_auth(token)
                    .json(body)
                    .build()?,
            )
        })
        .await?;
        if !response.status().is_success() {
            return Err(hub_response_error(operation, response).await);
        }
        response
            .json()
            .await
            .map_err(|error| anyhow!("{operation} returned an unexpected body: {error}"))
    }
}

async fn reply_body(
    context: &mut ExecutionContext,
    reply: TeamsReply,
    session_id: &str,
) -> flow_like_types::Result<Value> {
    let text: String = context.evaluate_pin("text").await?;
    let mut body = json!({
        "session_id": session_id,
        "request_id": format!("{}:{}", context.id, context.trace.id),
        "text": text,
    });
    if reply == TeamsReply::Card {
        body["card"] = context.evaluate_pin::<Value>("card").await?;
    }
    if reply == TeamsReply::Update {
        let message_id = context.evaluate_pin::<String>("input_message_id").await?;
        if message_id.trim().is_empty() {
            return Err(anyhow!(
                "Connect the Message ID returned by a Teams send node"
            ));
        }
        body["message_id"] = json!(message_id);
    }
    Ok(body)
}

async fn send(context: &mut ExecutionContext, reply: TeamsReply) -> flow_like_types::Result<()> {
    let call = TeamsCall::bind(context, "send Teams messages").await?;
    let body = reply_body(context, reply, &call.session_id).await?;
    let result: Value = call
        .post("send", "Teams response", &body, TEAMS_RETRY)
        .await?;
    let message = result["message_id"]
        .as_str()
        .filter(|message| !message.is_empty())
        .ok_or_else(|| anyhow!("Teams did not confirm a message ID"))?;
    context.set_pin_value("message_id", json!(message)).await?;
    context.activate_exec_pin("exec_out").await?;
    Ok(())
}

#[crate::register_node]
#[derive(Default)]
pub struct TeamsSendMessage;
#[async_trait]
impl NodeLogic for TeamsSendMessage {
    fn get_node(&self) -> Node {
        node(TeamsReply::Message)
    }
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        send(context, TeamsReply::Message).await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct TeamsSendCard;
#[async_trait]
impl NodeLogic for TeamsSendCard {
    fn get_node(&self) -> Node {
        node(TeamsReply::Card)
    }
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        send(context, TeamsReply::Card).await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct TeamsUpdateMessage;
#[async_trait]
impl NodeLogic for TeamsUpdateMessage {
    fn get_node(&self) -> Node {
        node(TeamsReply::Update)
    }
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        send(context, TeamsReply::Update).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like::{
        flow::{
            board::ExecutionStage,
            execution::{
                LogLevel, context::ExecutionContextCache, internal_node::InternalNode,
                internal_pin::InternalPin,
            },
            pin::PinType,
        },
        profile::Profile,
        state::{FlowLikeConfig, FlowLikeState},
        utils::http::HTTPClient,
    };
    use flow_like_types::sync::{Mutex, RwLock};
    use std::sync::{Arc, Weak};

    #[test]
    fn requires_a_server_session_reference() {
        assert_eq!(
            session_id(&json!({"teams":{"session_id":"run-1"}})).unwrap(),
            "run-1"
        );
        assert!(session_id(&json!({"bot_token":"untrusted"})).is_err());
        assert!(session_id(&json!({"teams":{"session_id":""}})).is_err());
    }

    #[test]
    fn update_reads_a_distinct_input_and_every_node_is_described_and_scored() {
        let nodes = [
            TeamsSendMessage.get_node(),
            TeamsSendCard.get_node(),
            TeamsUpdateMessage.get_node(),
            context::TeamsContextNode.get_node(),
            lookup::TeamsGetMessages.get_node(),
            lookup::TeamsGetMembers.get_node(),
        ];
        let descriptions: std::collections::HashSet<_> =
            nodes.iter().map(|node| node.description.as_str()).collect();
        assert_eq!(descriptions.len(), nodes.len());
        for node in &nodes {
            assert!(node.scores.is_some(), "{} has no scores", node.name);
            assert_eq!(node.category, "Events/Chat/Teams");
            for pin_type in [PinType::Input, PinType::Output] {
                let mut names: Vec<_> = node
                    .pins
                    .values()
                    .filter(|pin| pin.pin_type == pin_type)
                    .map(|pin| pin.name.as_str())
                    .collect();
                let count = names.len();
                names.sort_unstable();
                names.dedup();
                assert_eq!(names.len(), count, "{} repeats a pin name", node.name);
            }
            let inputs: std::collections::HashSet<_> = node
                .pins
                .values()
                .filter(|pin| pin.pin_type == PinType::Input)
                .map(|pin| pin.name.as_str())
                .collect();
            for pin in node.pins.values() {
                assert!(
                    pin.pin_type == PinType::Input || !inputs.contains(pin.name.as_str()),
                    "{} uses {} as input and output",
                    node.name,
                    pin.name
                );
            }
        }
        let update = &nodes[2];
        assert_eq!(
            update.get_pin_by_name("input_message_id").unwrap().pin_type,
            PinType::Input
        );
        let message_ids: Vec<_> = update
            .pins
            .values()
            .filter(|pin| pin.name == "message_id")
            .collect();
        assert_eq!(message_ids.len(), 1);
        assert_eq!(message_ids[0].pin_type, PinType::Output);
    }

    #[test]
    fn retries_only_in_flight_throttled_and_server_failures() {
        for status in [409, 429, 500, 502, 503, 504] {
            assert!(transient(reqwest::StatusCode::from_u16(status).unwrap()));
        }
        for status in [400, 401, 403, 404, 413, 422] {
            assert!(!transient(reqwest::StatusCode::from_u16(status).unwrap()));
        }
    }

    async fn replay_context() -> ExecutionContext {
        let logic: Arc<dyn NodeLogic> = Arc::new(TeamsSendMessage);
        let node = logic.get_node();
        let pins = node
            .pins
            .values()
            .map(|pin| (pin.id.clone(), Arc::new(InternalPin::new(pin, false))))
            .collect();
        let node = Arc::new(InternalNode::new(node, pins, logic, Default::default()));
        let state = Arc::new(FlowLikeState::new(
            FlowLikeConfig::new(),
            HTTPClient::new_without_refetch(),
        ));
        let mut context = ExecutionContext::new(
            Arc::new(Default::default()),
            &Weak::new(),
            &state,
            &node,
            &Arc::new(Mutex::new(Default::default())),
            &Arc::new(RwLock::new(Default::default())),
            LogLevel::Debug,
            ExecutionStage::Dev,
            Arc::new(Profile::default()),
            None,
            Arc::new(RwLock::new(Vec::new())),
            None,
            Some("executor-jwt".into()),
            Arc::new(Default::default()),
            None,
        )
        .await;
        context.execution_cache = Some(ExecutionContextCache {
            stores: state.config.read().await.stores.clone(),
            app_id: "app-1".into(),
            model_usage_app_id: Some("app-1".into()),
            board_dir: flow_like_storage::Path::from("apps/app-1"),
            board_id: "board".into(),
            node_id: "node".into(),
            sub: "alice".into(),
            shadow: true,
        });
        context
    }

    #[tokio::test]
    async fn replays_refuse_every_teams_call_with_the_graded_refusal_and_log_it() {
        for action in [
            "send Teams messages",
            "read Teams messages",
            "read Teams members",
        ] {
            let mut context = replay_context().await;
            let refusal =
                format!("SHADOW_SIDE_EFFECT_REFUSED: Shadow and regression runs cannot {action}");
            let error = TeamsCall::bind(&mut context, action).await.err().unwrap();
            assert_eq!(error.to_string(), refusal);
            let errors: Vec<_> = context
                .trace
                .logs
                .iter()
                .filter(|log| log.log_level == LogLevel::Error)
                .map(|log| log.message.as_str())
                .collect();
            assert_eq!(errors, [refusal.as_str()]);
        }
    }
}

use super::{
    CATEGORY, FLOWSCRIPT_NAMESPACE,
    agent::{CopilotStudioAgent, add_agent_input_pin},
    engine::{EngineRequest, run_engine_request},
    provider::{CopilotStudioProvider, add_provider_input_pin},
    scores,
    turn::{BotActivity, StreamEmitter, Turn, TurnCollector, add_turn_output_pins},
};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::ValueType,
    variable::VariableType,
};
use flow_like_types::{Result, Value, anyhow, async_trait, json::json};

pub(crate) const DEFAULT_LOCALE: &str = "en-US";

pub(crate) fn add_locale_pin(node: &mut Node) {
    node.add_input_pin(
        "locale",
        "Locale",
        "Language of the conversation, e.g. en-US or de-DE",
        VariableType::String,
    )
    .set_default_value(Some(json!(DEFAULT_LOCALE)));
}

pub(crate) async fn evaluate_locale(context: &mut ExecutionContext) -> String {
    let locale: String = context.evaluate_pin("locale").await.unwrap_or_default();
    match locale.trim() {
        "" => DEFAULT_LOCALE.to_string(),
        locale => locale.to_string(),
    }
}

async fn connection(
    context: &mut ExecutionContext,
) -> Result<(CopilotStudioProvider, CopilotStudioAgent)> {
    let provider: CopilotStudioProvider = context.evaluate_pin("provider").await?;
    let agent: CopilotStudioAgent = context.evaluate_pin("agent").await?;
    Ok((provider, agent.validated()?))
}

fn missing_conversation_id(agent: &CopilotStudioAgent) -> flow_like_types::Error {
    anyhow!(
        "Copilot Studio started a conversation with agent '{}' but returned no conversation id",
        agent.schema_name
    )
}

/// Starts a conversation without the greeting; its activities are not part of the turn.
async fn start_silently(
    context: &mut ExecutionContext,
    provider: &CopilotStudioProvider,
    agent: &CopilotStudioAgent,
    locale: &str,
    emitter: &mut StreamEmitter,
) -> Result<String> {
    let mut start = TurnCollector::new(None);
    let request = EngineRequest::Start {
        run_greeting: false,
        locale: locale.to_string(),
    };
    run_engine_request(context, provider, agent, request, &mut start, emitter).await?;
    start
        .conversation_id()
        .map(str::to_string)
        .ok_or_else(|| missing_conversation_id(agent))
}

#[crate::register_node]
#[derive(Default)]
pub struct CopilotStudioChatNode {}

impl CopilotStudioChatNode {
    async fn execute(context: &mut ExecutionContext, turn: &mut Turn) -> Result<()> {
        let (provider, agent) = connection(context).await?;
        let prompt: String = context.evaluate_pin("prompt").await?;
        if prompt.trim().is_empty() {
            return Err(anyhow!(
                "Prompt is empty; Copilot Studio needs a message to answer"
            ));
        }
        let locale = evaluate_locale(context).await;
        let conversation_id: String = context
            .evaluate_pin("conversation_id")
            .await
            .unwrap_or_default();
        let conversation_id = match conversation_id.trim() {
            "" => start_silently(context, &provider, &agent, &locale, &mut turn.emitter).await?,
            id => id.to_string(),
        };
        turn.collector.set_conversation_id(&conversation_id);

        let request = EngineRequest::Execute {
            conversation_id,
            activity: json!({
                "type": "message",
                "text": prompt,
                "textFormat": "plain",
                "locale": locale,
            }),
        };
        run_engine_request(
            context,
            &provider,
            &agent,
            request,
            &mut turn.collector,
            &mut turn.emitter,
        )
        .await
    }
}

#[async_trait]
impl NodeLogic for CopilotStudioChatNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_copilot_studio_chat",
            "Copilot Studio Chat",
            "Send a message to a published Copilot Studio agent and stream its reply. Leave the conversation ID empty to start a new conversation; pass the returned ID to continue it. Billed to the agent's tenant in Copilot Credits.",
            CATEGORY,
        );
        node.set_flowscript_name(FLOWSCRIPT_NAMESPACE, "chat");
        node.add_icon("/flow/icons/copilot.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        add_provider_input_pin(&mut node);
        add_agent_input_pin(&mut node);
        node.add_input_pin(
            "prompt",
            "Prompt",
            "The user's message",
            VariableType::String,
        );
        node.add_input_pin(
            "conversation_id",
            "Conversation ID",
            "Conversation to continue; leave empty to start a new one",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        add_locale_pin(&mut node);
        add_turn_output_pins(&mut node);

        node.set_long_running(true);
        node.set_scores(scores(5, 5, 3));
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let mut turn = Turn::begin(context, None).await?;
        let outcome = Self::execute(context, &mut turn).await;
        turn.complete(context, outcome).await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct CopilotStudioStartConversationNode {}

impl CopilotStudioStartConversationNode {
    async fn execute(context: &mut ExecutionContext, turn: &mut Turn) -> Result<()> {
        let (provider, agent) = connection(context).await?;
        let locale = evaluate_locale(context).await;
        let run_greeting: bool = context.evaluate_pin("run_greeting").await.unwrap_or(true);
        let request = EngineRequest::Start {
            run_greeting,
            locale,
        };
        run_engine_request(
            context,
            &provider,
            &agent,
            request,
            &mut turn.collector,
            &mut turn.emitter,
        )
        .await?;
        if turn.collector.conversation_id().is_none() {
            return Err(missing_conversation_id(&agent));
        }
        Ok(())
    }
}

#[async_trait]
impl NodeLogic for CopilotStudioStartConversationNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_copilot_studio_start_conversation",
            "Start Copilot Studio Conversation",
            "Open a conversation with a published Copilot Studio agent and return its ID, plus the agent's greeting when enabled (e.g. to show a welcome message when a chat opens).",
            CATEGORY,
        );
        node.set_flowscript_name(FLOWSCRIPT_NAMESPACE, "startConversation");
        node.add_icon("/flow/icons/copilot.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        add_provider_input_pin(&mut node);
        add_agent_input_pin(&mut node);
        add_locale_pin(&mut node);
        node.add_input_pin(
            "run_greeting",
            "Run Greeting",
            "Run the agent's Conversation Start topic and return its greeting",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));
        add_turn_output_pins(&mut node);

        node.set_long_running(true);
        node.set_scores(scores(6, 6, 5));
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let mut turn = Turn::begin(context, None).await?;
        let outcome = Self::execute(context, &mut turn).await;
        turn.complete(context, outcome).await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct CopilotStudioSendActivityNode {}

impl CopilotStudioSendActivityNode {
    async fn execute(context: &mut ExecutionContext, turn: &mut Turn) -> Result<()> {
        let (provider, agent) = connection(context).await?;
        let conversation_id: String = context.evaluate_pin("conversation_id").await?;
        let conversation_id = conversation_id.trim().to_string();
        if conversation_id.is_empty() {
            return Err(anyhow!(
                "Conversation ID is empty; start one with Start Copilot Studio Conversation or Copilot Studio Chat"
            ));
        }
        let activity: Value = context.evaluate_pin("activity").await?;
        turn.collector.set_conversation_id(&conversation_id);
        let request = EngineRequest::Execute {
            conversation_id,
            activity: normalize_activity(activity)?,
        };
        run_engine_request(
            context,
            &provider,
            &agent,
            request,
            &mut turn.collector,
            &mut turn.emitter,
        )
        .await
    }
}

#[async_trait]
impl NodeLogic for CopilotStudioSendActivityNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_copilot_studio_send_activity",
            "Send Copilot Studio Activity",
            "Send a raw Bot Framework activity into a Copilot Studio conversation, e.g. an Adaptive Card submit ({\"type\":\"message\",\"value\":{…}}) or an event ({\"type\":\"event\",\"name\":…}). The type defaults to message.",
            CATEGORY,
        );
        node.set_flowscript_name(FLOWSCRIPT_NAMESPACE, "sendActivity");
        node.add_icon("/flow/icons/copilot.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        add_provider_input_pin(&mut node);
        add_agent_input_pin(&mut node);
        node.add_input_pin(
            "conversation_id",
            "Conversation ID",
            "Conversation to send the activity into",
            VariableType::String,
        );
        node.add_input_pin(
            "activity",
            "Activity",
            "Bot Framework activity object; conversation.id is set for you",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Normal)
        .set_schema::<BotActivity>()
        .set_default_value(Some(json!({ "type": "message", "value": {} })));
        add_turn_output_pins(&mut node);

        node.set_long_running(true);
        node.set_scores(scores(5, 5, 3));
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let mut turn = Turn::begin(context, None).await?;
        let outcome = Self::execute(context, &mut turn).await;
        turn.complete(context, outcome).await
    }
}

fn normalize_activity(activity: Value) -> Result<Value> {
    let Value::Object(mut map) = activity else {
        return Err(anyhow!(
            "Activity must be a JSON object such as {{\"type\":\"message\",\"text\":\"…\"}}"
        ));
    };
    let has_type = map
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(|kind| !kind.trim().is_empty());
    if !has_type {
        map.insert("type".to_string(), json!("message"));
    }
    Ok(Value::Object(map))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activities_default_to_messages_and_must_be_objects() {
        let submit = normalize_activity(json!({ "value": { "choice": "a" } })).unwrap();
        assert_eq!(submit["type"], "message");
        let event = normalize_activity(json!({ "type": "event", "name": "x" })).unwrap();
        assert_eq!(event["type"], "event");
        assert!(normalize_activity(json!("hello")).is_err());
    }
}

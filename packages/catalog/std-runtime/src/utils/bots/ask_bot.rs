use flow_like::{
    flow::{
        execution::{ExecutionEnvironment, context::ExecutionContext},
        node::{Node, NodeLogic, NodeScores},
        variable::VariableType,
    },
    hub::{hub_api_url, hub_response_error},
};
use flow_like_types::{
    Value, anyhow, async_trait, authorization::ResourceAudience, json::json, reqwest,
};
use std::time::Duration;

const MAX_WAIT_SECONDS: i64 = 100;
const REQUEST_SLACK: Duration = Duration::from_secs(30);
const ASK_REFUSED: &str = "SHADOW_SIDE_EFFECT_REFUSED: Shadow and regression runs cannot ask a bot";

/// Wakes one of the run user's bots with a message and optionally waits for its answer.
#[crate::register_node]
#[derive(Default)]
pub struct AskBotNode {}

impl AskBotNode {
    pub fn new() -> Self {
        Self {}
    }
}

fn clamp_wait(seconds: i64) -> i64 {
    seconds.clamp(0, MAX_WAIT_SECONDS)
}

fn request_id(run_id: &str, node_id: &str, trace_id: &str) -> String {
    blake3::hash(json!([run_id, node_id, trace_id]).to_string().as_bytes())
        .to_hex()
        .to_string()
}

fn validate(bot_id: &str, message: &str) -> flow_like_types::Result<()> {
    if bot_id.trim().is_empty() {
        return Err(anyhow!("Ask Bot needs the id of one of your bots"));
    }
    if message.trim().is_empty() {
        return Err(anyhow!("Ask Bot needs a message"));
    }
    Ok(())
}

fn ask_body(bot_id: Option<&str>, message: &str, wait: i64, request_id: &str) -> Value {
    let mut body = json!({
        "message": message.trim(),
        "wait_seconds": clamp_wait(wait),
        "request_id": request_id,
    });
    if let Some(bot_id) = bot_id {
        body["bot_id"] = json!(bot_id.trim());
    }
    body
}

fn ask_url(
    context: &ExecutionContext,
    app_id: &str,
    bot_id: &str,
) -> flow_like_types::Result<(reqwest::Url, Option<String>)> {
    let secure = context.profile.secure;
    if let Some(auth) = context.executor_api_auth.as_ref() {
        let url = hub_api_url(
            auth.api_url(),
            secure,
            &["execution", "apps", app_id, "bots", "ask"],
        )?;
        return Ok((url, Some(auth.token().to_string())));
    }
    if context.execution_environment() == ExecutionEnvironment::Server {
        return Err(anyhow!(
            "Ask Bot on a server needs an authenticated executor session"
        ));
    }
    let url = hub_api_url(
        &context.profile.hub,
        secure,
        &["bots", bot_id.trim(), "ask"],
    )?;
    Ok((url, None))
}

#[async_trait]
impl NodeLogic for AskBotNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "bots_ask_bot",
            "Ask Bot",
            "Sends a message to one of your bots, as if a flow event had woken it, and optionally waits for its answer. The exchange lands in the bot's chat with you. Works for bots on a server or on a desktop that is online; a bot that only lives in a local store can't be reached.",
            "Bots",
        );
        node.set_version(1);
        node.add_icon("/flow/icons/bot.svg");
        node.set_scores(
            NodeScores::new()
                .set_privacy(5)
                .set_security(6)
                .set_performance(4)
                .set_governance(6)
                .set_reliability(5)
                .set_cost(4)
                .build(),
        );

        node.add_input_pin("exec_in", "In", "Ask the bot", VariableType::Execution);
        node.add_input_pin(
            "bot_id",
            "Bot",
            "The id of one of your bots (the person this run acts as)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "message",
            "Message",
            "What the bot is asked, at most 8,000 characters",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "wait_seconds",
            "Wait (seconds)",
            "How long to wait for the bot's answer, at most 100. 0 continues as soon as the turn is queued.",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_output_pin(
            "exec_out",
            "Out",
            "The bot was asked",
            VariableType::Execution,
        );
        node.add_output_pin(
            "turn_id",
            "Turn ID",
            "The turn the message queued",
            VariableType::String,
        );
        node.add_output_pin(
            "status",
            "Status",
            "queued, running, waiting (for your approval), done, failed or cancelled",
            VariableType::String,
        );
        node.add_output_pin(
            "reply",
            "Reply",
            "The bot's answer; empty when you didn't wait or it hasn't answered yet",
            VariableType::String,
        );
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let bot_id = context.evaluate_pin::<String>("bot_id").await?;
        let message = context.evaluate_pin::<String>("message").await?;
        let wait = context.evaluate_pin::<i64>("wait_seconds").await?;
        validate(&bot_id, &message)?;
        let app_id = {
            let cache = context.execution_cache.as_ref();
            if cache.is_some_and(|cache| cache.shadow) {
                return Err(anyhow!(ASK_REFUSED));
            }
            cache.map(|cache| cache.app_id.clone()).unwrap_or_default()
        };
        let (url, executor_token) = ask_url(context, &app_id, &bot_id)?;
        let id = request_id(context.run_id(), &context.id, &context.trace.id);
        let body = ask_body(
            executor_token.is_some().then_some(bot_id.as_str()),
            &message,
            wait,
            &id,
        );
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(clamp_wait(wait) as u64) + REQUEST_SLACK)
            .build()?;
        let mut request = client.post(url).json(&body);
        if let Some(token) = &executor_token {
            request = request.bearer_auth(token);
        }
        let request = request.build()?;
        let request = if executor_token.is_some() {
            request
        } else {
            context
                .authorize_request(request, ResourceAudience::ProjectApi)
                .await?
        };
        let response = client.execute(request).await?;
        if !response.status().is_success() {
            return Err(hub_response_error("Ask Bot", response).await);
        }
        let turn: Value = response.json().await?;
        let text = |pointer: &str| {
            turn.pointer(pointer)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        context
            .set_pin_value("turn_id", json!(text("/turn_id")))
            .await?;
        context
            .set_pin_value("status", json!(text("/status")))
            .await?;
        context
            .set_pin_value("reply", json!(text("/reply/text")))
            .await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wait_is_clamped_to_the_server_limit() {
        assert_eq!(clamp_wait(-5), 0);
        assert_eq!(clamp_wait(30), 30);
        assert_eq!(clamp_wait(100_000), MAX_WAIT_SECONDS);
    }

    #[test]
    fn a_retried_node_asks_with_the_same_request_id_and_a_new_run_with_another() {
        let first = request_id("run-1", "node-1", "trace-1");
        assert_eq!(first, request_id("run-1", "node-1", "trace-1"));
        assert_ne!(first, request_id("run-2", "node-1", "trace-1"));
        assert_ne!(first, request_id("run-1", "node-2", "trace-1"));
    }

    #[test]
    fn an_empty_bot_or_message_is_refused_before_any_call() {
        assert!(validate("", "hi").is_err());
        assert!(validate("bot_1", "  ").is_err());
        assert!(validate("bot_1", "hi").is_ok());
    }

    #[test]
    fn the_executor_body_names_the_bot_and_the_user_body_does_not() {
        let executor = ask_body(Some(" bot_1 "), " hello ", 500, "req");
        assert_eq!(executor["bot_id"], "bot_1");
        assert_eq!(executor["message"], "hello");
        assert_eq!(executor["wait_seconds"], MAX_WAIT_SECONDS);
        assert!(ask_body(None, "hello", 0, "req").get("bot_id").is_none());
    }

    #[test]
    fn the_node_describes_itself_with_execution_pins() {
        let node = AskBotNode::new().get_node();
        assert_eq!(node.name, "bots_ask_bot");
        for pin in [
            "exec_in",
            "bot_id",
            "message",
            "wait_seconds",
            "exec_out",
            "reply",
        ] {
            assert!(node.pins.values().any(|p| p.name == pin), "{pin}");
        }
    }
}

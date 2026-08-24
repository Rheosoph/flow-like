use flow_like::{
    bit::Bit,
    flow::{
        execution::{LogLevel, context::ExecutionContext},
        node::{Node, NodeLogic, NodeScores},
        pin::{PinOptions, ValueType},
        variable::VariableType,
    },
};
#[cfg(feature = "execute")]
use flow_like_types::anyhow;
use flow_like_types::{Value, async_trait, json::json};
#[cfg(feature = "execute")]
use rig::completion::{Completion, Message, ToolDefinition};
#[cfg(feature = "execute")]
use rig::message::{AssistantContent, ToolCall, ToolChoice, ToolFunction};
#[cfg(feature = "execute")]
use rig::tool::Tool;
use std::collections::HashSet;
#[cfg(feature = "execute")]
use std::fmt;

#[crate::register_node]
#[derive(Default)]
pub struct AiKeywordExtractionNode {}

impl AiKeywordExtractionNode {
    pub fn new() -> Self {
        AiKeywordExtractionNode {}
    }
}

#[cfg(feature = "execute")]
#[derive(Debug)]
struct KeywordSubmitTool {
    max_keywords: usize,
}

#[cfg(feature = "execute")]
#[derive(Debug)]
struct SubmitError(String);

#[cfg(feature = "execute")]
impl fmt::Display for SubmitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Keyword extraction failed: {}", self.0)
    }
}

#[cfg(feature = "execute")]
impl std::error::Error for SubmitError {}

#[cfg(feature = "execute")]
impl Tool for KeywordSubmitTool {
    const NAME: &'static str = "submit_keywords";
    type Error = SubmitError;
    type Args = Value;
    type Output = Value;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: format!(
                "Submit extracted keywords from the text. Return up to {} unique, relevant keywords or key phrases.",
                self.max_keywords
            ),
            parameters: json!({
                "type": "object",
                "properties": {
                    "keywords": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Array of extracted keywords or key phrases",
                        "maxItems": self.max_keywords
                    }
                },
                "required": ["keywords"],
                "additionalProperties": false
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> std::result::Result<Self::Output, Self::Error> {
        let keywords = args
            .get("keywords")
            .and_then(|v| v.as_array())
            .ok_or_else(|| SubmitError("Missing 'keywords' array in response".to_string()))?;

        if keywords.iter().all(|k| k.is_string()) {
            Ok(args)
        } else {
            Err(SubmitError("All keywords must be strings".to_string()))
        }
    }

    fn name(&self) -> String {
        Self::NAME.to_string()
    }
}

#[async_trait]
impl NodeLogic for AiKeywordExtractionNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_processing_ai_keyword_extraction",
            "AI Keywords",
            "Extracts keywords from text using an LLM. The AI understands context and semantics, providing high-quality keyword extraction for complex or domain-specific content.",
            "AI/Processing",
        );
        node.set_flowscript_name("ai.processing", "extractKeywords");
        node.add_icon("/flow/icons/sparkles.svg");
        node.set_version(3);

        node.set_scores(
            NodeScores::new()
                .set_privacy(4)
                .set_security(4)
                .set_performance(5)
                .set_governance(5)
                .set_reliability(7)
                .set_cost(3)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger to start keyword extraction",
            VariableType::Execution,
        );

        node.add_input_pin(
            "model",
            "Model",
            "LLM to use for keyword extraction",
            VariableType::Struct,
        )
        .set_schema::<Bit>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "text",
            "Text",
            "The text to extract keywords from",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "max_keywords",
            "Max Keywords",
            "Maximum number of keywords to extract",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(10)));

        node.add_input_pin(
            "context",
            "Context",
            "Optional context or instructions for keyword extraction (e.g., 'focus on technical terms' or 'extract product names')",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Executes after keyword extraction completes",
            VariableType::Execution,
        );

        node.add_output_pin(
            "keywords",
            "Keywords",
            "Extracted keywords as a string set",
            VariableType::String,
        )
        .set_value_type(ValueType::HashSet);

        node.set_long_running(true);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let model_bit = context.evaluate_pin::<Bit>("model").await?;
        let text: String = context.evaluate_pin("text").await?;
        let max_keywords: i64 = context.evaluate_pin("max_keywords").await?;
        let custom_context: String = context.evaluate_pin("context").await?;

        let max_keywords = max_keywords.clamp(1, 100) as usize;

        let mut system_prompt = format!(
            "You are a keyword extraction expert. Extract the most relevant and important keywords or key phrases from the provided text. \
            Focus on: \
            - Nouns and noun phrases that represent key concepts \
            - Technical terms and domain-specific vocabulary \
            - Named entities (people, places, organizations) \
            - Important action verbs when relevant \
            Return up to {} unique keywords.",
            max_keywords
        );

        if !custom_context.trim().is_empty() {
            system_prompt.push_str(&format!("\n\nAdditional instructions: {}", custom_context));
        }

        let llm_input = format!("Extract keywords from the following text:\n\n{}", text);

        let agent_builder = model_bit
            .agent(context, &None)
            .await?
            .preamble(&system_prompt)
            .tool(KeywordSubmitTool { max_keywords })
            .tool_choice(ToolChoice::Required);

        let agent = agent_builder.build();

        context.log_message("Invoking LLM for keyword extraction", LogLevel::Debug);

        let response = agent
            .completion(llm_input, Vec::<Message>::new())
            .await
            .map_err(|e| anyhow!("Model completion failed: {}", e))?
            .send()
            .await
            .map_err(|e| anyhow!("Failed to send completion request: {}", e))?;

        let mut keywords_value: Option<Value> = None;
        for content in response.choice {
            if let AssistantContent::ToolCall(ToolCall {
                function: ToolFunction {
                    name, arguments, ..
                },
                ..
            }) = content
                && name == "submit_keywords"
            {
                keywords_value = Some(arguments);
                break;
            }
        }

        let args = keywords_value.ok_or_else(|| {
            anyhow!("Model did not return keyword extraction results. Ensure the model supports function calling.")
        })?;

        let keywords_array = args
            .get("keywords")
            .and_then(|v| v.as_array())
            .ok_or_else(|| anyhow!("Invalid keyword extraction response format"))?;

        let result: HashSet<String> = keywords_array
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();

        context.log_message(
            &format!("Extracted {} keywords using AI", result.len()),
            LogLevel::Debug,
        );

        context.set_pin_value("keywords", json!(result)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Processing requires the 'execute' feature"
        ))
    }
}

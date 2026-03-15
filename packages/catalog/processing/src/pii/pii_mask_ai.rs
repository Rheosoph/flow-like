//! AI-based PII (Personally Identifiable Information) masking node
//!
//! This node uses an LLM to intelligently detect and mask PII in text.
//! It can understand context and detect PII types that may be missed by regex patterns.

#[cfg(feature = "execute")]
use flow_like::flow::execution::LogLevel;
use flow_like::{
    bit::Bit,
    flow::{
        execution::context::ExecutionContext,
        node::{Node, NodeLogic, NodeScores},
        pin::{PinOptions, ValueType},
        variable::VariableType,
    },
};
#[cfg(feature = "execute")]
use flow_like_types::Value;
#[cfg(feature = "execute")]
use flow_like_types::anyhow;
use flow_like_types::{async_trait, json::json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Details about a detected PII instance
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PiiDetection {
    /// Type of PII detected (e.g., email, phone, name, address, ssn, credit_card)
    #[serde(rename = "type")]
    pub pii_type: String,
    /// The original PII value that was masked
    pub original: String,
    /// Brief context about why this was identified as PII
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
}
#[cfg(feature = "execute")]
use rig::completion::{Completion, ToolDefinition};
#[cfg(feature = "execute")]
use rig::message::{AssistantContent, ToolCall, ToolChoice, ToolFunction};
#[cfg(feature = "execute")]
use rig::tool::Tool;
#[cfg(feature = "execute")]
use std::fmt;

#[cfg(feature = "execute")]
#[derive(Debug)]
struct PiiMaskTool {
    mask_text: String,
}

#[cfg(feature = "execute")]
#[derive(Debug)]
struct PiiMaskError(String);

#[cfg(feature = "execute")]
impl fmt::Display for PiiMaskError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PII masking failed: {}", self.0)
    }
}

#[cfg(feature = "execute")]
impl std::error::Error for PiiMaskError {}

#[cfg(feature = "execute")]
impl Tool for PiiMaskTool {
    const NAME: &'static str = "submit_masked_text";
    type Error = PiiMaskError;
    type Args = Value;
    type Output = Value;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: format!(
                "Submit the text with all PII masked. Replace each PII instance with '{}' or similar masking.",
                self.mask_text
            ),
            parameters: json!({
                "type": "object",
                "properties": {
                    "masked_text": {
                        "type": "string",
                        "description": "The input text with all PII replaced by mask placeholders"
                    },
                    "detections": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "type": {
                                    "type": "string",
                                    "description": "Type of PII detected (e.g., email, phone, name, address, ssn, credit_card, date_of_birth, etc.)"
                                },
                                "original": {
                                    "type": "string",
                                    "description": "The original PII value that was masked"
                                },
                                "context": {
                                    "type": "string",
                                    "description": "Brief context about why this was identified as PII"
                                }
                            },
                            "required": ["type", "original"]
                        },
                        "description": "Array of PII detections with type and original value"
                    }
                },
                "required": ["masked_text", "detections"],
                "additionalProperties": false
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> std::result::Result<Self::Output, Self::Error> {
        let masked_text = args
            .get("masked_text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| PiiMaskError("Missing 'masked_text' in response".to_string()))?;

        let detections = args
            .get("detections")
            .and_then(|v| v.as_array())
            .ok_or_else(|| PiiMaskError("Missing 'detections' array in response".to_string()))?;

        // Validate detections format
        for detection in detections {
            if !detection.get("type").is_some_and(|v| v.is_string()) {
                return Err(PiiMaskError(
                    "Each detection must have a 'type' string".to_string(),
                ));
            }
            if !detection.get("original").is_some_and(|v| v.is_string()) {
                return Err(PiiMaskError(
                    "Each detection must have an 'original' string".to_string(),
                ));
            }
        }

        // Verify masked_text doesn't contain the original values
        for detection in detections {
            if let Some(original) = detection.get("original").and_then(|v| v.as_str())
                && !original.is_empty()
                && masked_text.contains(original)
            {
                return Err(PiiMaskError(format!(
                    "Masked text still contains original PII: {}",
                    original
                )));
            }
        }

        Ok(args)
    }

    fn name(&self) -> String {
        Self::NAME.to_string()
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct PiiMaskAiNode {}

impl PiiMaskAiNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for PiiMaskAiNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "processing_pii_mask_ai",
            "PII Mask (AI)",
            "Masks Personally Identifiable Information using an LLM. Can detect contextual PII like names, addresses, and sensitive information that regex patterns might miss.",
            "AI/Processing",
        );
        node.add_icon("/flow/icons/shield-ai.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(8)
                .set_security(7)
                .set_performance(5)
                .set_governance(7)
                .set_reliability(7)
                .set_cost(3)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger",
            VariableType::Execution,
        );

        node.add_input_pin(
            "model",
            "Model",
            "LLM to use for PII detection",
            VariableType::Struct,
        )
        .set_schema::<Bit>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "text",
            "Text",
            "The text to scan for PII",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "mask_text",
            "Mask Text",
            "Text to replace PII with (default: [REDACTED])",
            VariableType::String,
        )
        .set_default_value(Some(json!("[REDACTED]")));

        node.add_input_pin(
            "additional_context",
            "Context",
            "Additional instructions for PII detection (e.g., 'focus on medical records' or 'mask company names')",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "sensitivity",
            "Sensitivity",
            "Detection sensitivity level",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "low".to_string(),
                    "medium".to_string(),
                    "high".to_string(),
                ])
                .build(),
        )
        .set_default_value(Some(json!("medium")));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after masking",
            VariableType::Execution,
        );

        node.add_output_pin(
            "masked_text",
            "Masked Text",
            "Text with PII masked",
            VariableType::String,
        );

        node.add_output_pin(
            "detection_count",
            "Detection Count",
            "Number of PII instances detected and masked",
            VariableType::Integer,
        );

        node.add_output_pin(
            "detections",
            "Detections",
            "Array with detection details (type, original value, context)",
            VariableType::Struct,
        )
        .set_schema::<PiiDetection>()
        .set_value_type(ValueType::Array);

        node.set_long_running(true);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let model_bit = context.evaluate_pin::<Bit>("model").await?;
        let text: String = context.evaluate_pin("text").await?;
        let mask_text: String = context.evaluate_pin("mask_text").await?;
        let additional_context: String = context.evaluate_pin("additional_context").await?;
        let sensitivity: String = context.evaluate_pin("sensitivity").await?;

        if text.trim().is_empty() {
            context.set_pin_value("masked_text", json!("")).await?;
            context
                .set_pin_value("detection_count", json!(0i64))
                .await?;
            context.set_pin_value("detections", json!([])).await?;
            context.activate_exec_pin("exec_out").await?;
            return Ok(());
        }

        let sensitivity_instruction = match sensitivity.as_str() {
            "low" => {
                "Only mask clearly identifiable PII such as full names, complete email addresses, phone numbers, SSNs, and credit card numbers."
            }
            "high" => {
                "Aggressively mask any potentially identifying information including partial names, nicknames, locations, dates, ages, job titles, organizations, and any data that could be combined to identify someone."
            }
            _ => {
                "Mask standard PII including names, email addresses, phone numbers, physical addresses, SSNs, credit card numbers, dates of birth, passport numbers, and any other government ID numbers."
            }
        };

        let mut system_prompt = format!(
            r#"You are a PII (Personally Identifiable Information) detection and masking expert. Your task is to:
1. Identify all PII in the given text
2. Replace each PII instance with the mask text: "{}"
3. Return the masked text and a list of what was detected

{sensitivity_instruction}

Common PII types to look for:
- Names (first, last, full names)
- Email addresses
- Phone numbers
- Physical addresses
- Social Security Numbers (SSN)
- Credit card numbers
- Dates of birth
- Passport/ID numbers
- IP addresses
- Bank account numbers
- Medical record numbers
- Biometric data references

IMPORTANT:
- Maintain the original text structure and formatting
- Replace each PII with the exact mask text provided
- Do not add extra spaces or formatting around the mask
- Be thorough but avoid false positives on common words"#,
            mask_text
        );

        if !additional_context.trim().is_empty() {
            system_prompt.push_str(&format!(
                "\n\nAdditional instructions: {}",
                additional_context
            ));
        }

        let user_prompt = format!(
            "Please scan the following text for PII and return the masked version:\n\n{}",
            text
        );

        let agent_builder = model_bit
            .agent(context, &None)
            .await?
            .preamble(&system_prompt)
            .tool(PiiMaskTool {
                mask_text: mask_text.clone(),
            })
            .tool_choice(ToolChoice::Required);

        let agent = agent_builder.build();

        context.log_message("Invoking LLM for PII detection", LogLevel::Debug);

        let response = agent
            .completion(user_prompt, vec![])
            .await
            .map_err(|e| anyhow!("Model completion failed: {}", e))?
            .send()
            .await
            .map_err(|e| anyhow!("Failed to send completion request: {}", e))?;

        let mut result_value: Option<Value> = None;
        for content in response.choice {
            if let AssistantContent::ToolCall(ToolCall {
                function: ToolFunction {
                    name, arguments, ..
                },
                ..
            }) = content
                && name == "submit_masked_text"
            {
                result_value = Some(arguments);
                break;
            }
        }

        let args = result_value.ok_or_else(|| {
            anyhow!("Model did not return PII masking results. Ensure the model supports function calling.")
        })?;

        let masked_text = args
            .get("masked_text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Invalid response: missing masked_text"))?;

        let detections = args
            .get("detections")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        context.log_message(
            &format!("Masked {} PII instances using AI", detections.len()),
            LogLevel::Debug,
        );

        context
            .set_pin_value("masked_text", json!(masked_text))
            .await?;
        context
            .set_pin_value("detection_count", json!(detections.len() as i64))
            .await?;
        context
            .set_pin_value("detections", json!(detections))
            .await?;
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

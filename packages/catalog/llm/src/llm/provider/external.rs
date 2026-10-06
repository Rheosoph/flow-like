use std::collections::HashMap;

use flow_like::{
    bit::{Bit, BitModelClassification, BitTypes, LLMParameters, Metadata},
    flow::{
        execution::context::ExecutionContext,
        node::{Node, NodeLogic},
        pin::PinOptions,
        variable::VariableType,
    },
    state::FlowLikeState,
};
use flow_like_model_provider::{
    llm::external::{self, ExternalProvider},
    provider::ModelProvider,
};
use flow_like_types::{
    Value, async_trait,
    json::{self, json},
};

#[crate::register_node]
#[derive(Default)]
pub struct BuildClaudeCodeNode;

#[crate::register_node]
#[derive(Default)]
pub struct BuildCodexNode;

#[crate::register_node]
#[derive(Default)]
pub struct BuildGithubCopilotNode;

#[crate::register_node]
#[derive(Default)]
pub struct BuildMicrosoftCopilotNode;

fn fields(kind: ExternalProvider) -> &'static [(&'static str, &'static str, &'static str, bool)] {
    match kind {
        ExternalProvider::ClaudeCode => &[(
            "executable",
            "Executable",
            "Optional absolute path to the installed Claude CLI on this desktop",
            false,
        )],
        ExternalProvider::Codex => &[
            (
                "access_token",
                "Access Token",
                "ChatGPT access token; leave empty on desktop to use the Codex CLI login",
                true,
            ),
            (
                "account_id",
                "Account ID",
                "Optional ChatGPT account identifier",
                false,
            ),
        ],
        ExternalProvider::GithubCopilot => &[
            (
                "access_token",
                "GitHub Access Token",
                "GitHub token authorized for Copilot; required unless a Copilot API token is supplied",
                true,
            ),
            (
                "api_key",
                "Copilot API Token",
                "Optional already exchanged Copilot token",
                true,
            ),
        ],
        ExternalProvider::MicrosoftCopilot => &[
            (
                "access_token",
                "Access Token",
                "Delegated Microsoft Graph token authorized for Microsoft 365 Copilot Chat",
                true,
            ),
            (
                "timezone",
                "Timezone",
                "Timezone used by Copilot, for example UTC",
                false,
            ),
        ],
    }
}

fn definition(kind: ExternalProvider, name: &str, method: &str) -> Node {
    let mut node = Node::new(
        name,
        &format!("{} Model", kind.label()),
        "Builds a model Bit for the existing model and agent nodes and checks provider availability",
        "AI/Generative/Provider",
    );
    node.set_flowscript_name("ai.provider", method);
    node.add_icon("/flow/icons/find_model.svg");
    node.set_version(2);
    node.set_long_running(true);
    node.add_input_pin(
        "exec_in",
        "Input",
        "Build the provider Bit",
        VariableType::Execution,
    );
    node.add_input_pin(
        "model_id",
        "Model ID",
        "Model identifier offered by this provider",
        VariableType::String,
    )
    .set_default_value(Some(json!(if kind == ExternalProvider::MicrosoftCopilot {
        "microsoft-365-copilot"
    } else {
        ""
    })));
    node.add_input_pin(
        "context_length",
        "Context Length",
        "Context window in tokens for the selected model",
        VariableType::Integer,
    )
    .set_default_value(Some(json!(32000)));
    for &(key, label, description, sensitive) in fields(kind) {
        node.add_input_pin(key, label, description, VariableType::String)
            .set_default_value(Some(json!(if key == "timezone" { "UTC" } else { "" })))
            .set_options(PinOptions::new().set_sensitive(sensitive).build());
    }
    node.add_output_pin(
        "exec_out",
        "Done",
        "The Bit and availability status are ready",
        VariableType::Execution,
    );
    node.add_output_pin(
        "model",
        "Model",
        "Model Bit compatible with Invoke Model and Agent from Model",
        VariableType::Struct,
    )
    .set_schema::<Bit>();
    node.add_output_pin(
        "available",
        "Available",
        "The execution host can currently access this provider",
        VariableType::Boolean,
    );
    node.add_output_pin(
        "unavailable_reason",
        "Unavailable Reason",
        "Readiness error, empty when available",
        VariableType::String,
    );
    node
}

fn model_bit(
    kind: ExternalProvider,
    model_id: String,
    context_length: u32,
    params: HashMap<String, Value>,
) -> flow_like_types::Result<Bit> {
    if model_id.trim().is_empty() {
        return Err(flow_like_types::anyhow!("A model ID is required"));
    }
    if context_length == 0 {
        return Err(flow_like_types::anyhow!("Context length must be positive"));
    }
    if kind == ExternalProvider::MicrosoftCopilot && model_id != "microsoft-365-copilot" {
        return Err(flow_like_types::anyhow!(
            "Microsoft 365 Copilot selects its own underlying model; use microsoft-365-copilot"
        ));
    }
    let parameters = json::to_value(LLMParameters {
        context_length,
        model_classification: BitModelClassification::default(),
        provider: ModelProvider {
            provider_name: kind.provider_name().into(),
            model_id: Some(model_id.clone()),
            api_surface: None,
            version: None,
            params: Some(params),
        },
    })?;
    let id = flow_like_storage::blake3::hash(&json::to_vec(&parameters)?)
        .to_hex()
        .to_string();
    let mut bit = Bit::default();
    bit.id = id;
    bit.bit_type = BitTypes::Llm;
    bit.parameters = parameters;
    bit.meta.insert(
        "en".into(),
        Metadata {
            name: format!("{} / {model_id}", kind.label()),
            description: format!("{} model for Flow-Like", kind.label()),
            ..Metadata::default()
        },
    );
    Ok(bit)
}

async fn run_provider(
    kind: ExternalProvider,
    context: &mut ExecutionContext,
) -> flow_like_types::Result<()> {
    context.deactivate_exec_pin("exec_out").await?;
    let model_id: String = context.evaluate_pin("model_id").await?;
    let context_length: u32 = context.evaluate_pin("context_length").await?;
    let mut params = HashMap::new();
    for &(key, _, _, _) in fields(kind) {
        let value: String = context.evaluate_pin(key).await?;
        if !value.trim().is_empty() {
            params.insert(key.into(), json!(value.trim()));
        }
    }
    let bit = model_bit(kind, model_id.trim().into(), context_length, params)?;
    let capabilities = FlowLikeState::completion_model_capabilities(&context.app_state).await;
    let provider = bit
        .try_to_provider()
        .ok_or_else(|| flow_like_types::anyhow!("Invalid model Bit"))?;
    let status = external::check_available(&provider, capabilities.local_credentials).await;
    context
        .set_pin_value("available", json!(status.is_ok()))
        .await?;
    // Upstream errors can contain request data. Keep the node's status safe to log.
    let reason = if status.is_err() {
        format!(
            "{} is unavailable. Check credentials and runtime setup.",
            kind.label()
        )
    } else {
        String::new()
    };
    context
        .set_pin_value("unavailable_reason", json!(reason))
        .await?;
    context.set_pin_value("model", json!(bit)).await?;
    context.activate_exec_pin("exec_out").await?;
    Ok(())
}

macro_rules! implement_provider {
    ($node:ident, $kind:ident, $name:literal, $method:literal) => {
        impl $node {
            pub fn new() -> Self {
                Self
            }
        }
        #[async_trait]
        impl NodeLogic for $node {
            fn get_node(&self) -> Node {
                definition(ExternalProvider::$kind, $name, $method)
            }
            async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
                run_provider(ExternalProvider::$kind, context).await
            }
        }
    };
}

implement_provider!(
    BuildClaudeCodeNode,
    ClaudeCode,
    "ai_generative_build_claude_code",
    "claudeCode"
);
implement_provider!(BuildCodexNode, Codex, "ai_generative_build_codex", "codex");
implement_provider!(
    BuildGithubCopilotNode,
    GithubCopilot,
    "ai_generative_build_github_copilot",
    "githubCopilot"
);
implement_provider!(
    BuildMicrosoftCopilotNode,
    MicrosoftCopilot,
    "ai_generative_build_microsoft_copilot",
    "microsoftCopilot"
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_nodes_emit_the_existing_bit_schema() {
        for kind in [
            ExternalProvider::ClaudeCode,
            ExternalProvider::Codex,
            ExternalProvider::GithubCopilot,
            ExternalProvider::MicrosoftCopilot,
        ] {
            let model = if kind == ExternalProvider::MicrosoftCopilot {
                "microsoft-365-copilot"
            } else {
                "selected-model"
            };
            let bit = model_bit(kind, model.into(), 32000, HashMap::new()).unwrap();
            let provider = bit.try_to_provider().unwrap();
            assert_eq!(provider.provider_name, kind.provider_name());
            assert_eq!(provider.model_id.as_deref(), Some(model));
            assert_eq!(bit.bit_type, BitTypes::Llm);
        }
    }
}

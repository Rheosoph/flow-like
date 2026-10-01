use rig::client::CompletionClient;
use rig::completion::{
    CompletionError, CompletionModel, CompletionRequest, CompletionResponse, ToolDefinition,
};
use rig::message::ToolChoice;
use rig::providers::chatgpt;
use rig::providers::openai::responses_api::{ResponsesCompletionModel, ResponsesToolDefinition};
use rig::streaming::StreamingCompletionResponse;
use serde_json::{Map, Value};

/// A Rig OpenAI Responses client whose function tools are sent as written, with `strict: false`.
///
/// Rig 0.38.2 makes every request tool strict on the Responses API: it rewrites the schema for
/// strict mode (every property required, `additionalProperties: false`) and sets `strict: true`,
/// so OpenAI rejects schemas strict mode cannot express, such as a property without a `type`, and
/// optional or open-object parameters silently change meaning. Rig appends a model's own tools
/// after that conversion without touching them, so each request's tools move there instead.
#[derive(Clone)]
pub struct NonStrictToolsClient<C>(pub C);

#[derive(Clone)]
pub struct NonStrictToolsModel<M>(M);

/// A Rig Responses completion model's model-level tools, which Rig sends unchanged.
pub trait ResponsesModelTools {
    fn model_tools(&mut self) -> &mut Vec<ResponsesToolDefinition>;
}

impl<H> ResponsesModelTools for ResponsesCompletionModel<H> {
    fn model_tools(&mut self) -> &mut Vec<ResponsesToolDefinition> {
        &mut self.tools
    }
}

impl<H> ResponsesModelTools for chatgpt::ResponsesCompletionModel<H> {
    fn model_tools(&mut self) -> &mut Vec<ResponsesToolDefinition> {
        &mut self.tools
    }
}

/// `strict` travels in the flattened config because Rig skips a `false` strict field, and the
/// Responses API applies strict mode to a function tool that omits it.
pub fn non_strict_function(tool: ToolDefinition) -> ResponsesToolDefinition {
    ResponsesToolDefinition {
        kind: "function".to_string(),
        name: tool.name,
        parameters: tool.parameters,
        strict: false,
        description: tool.description,
        config: Map::from_iter([("strict".to_string(), Value::Bool(false))]),
    }
}

/// Rig's OpenAI tool choice rejects `Specific`, so the request keeps only the named tools and
/// requires a call to one of them.
fn narrow_to_specific_tools(request: &mut CompletionRequest) -> Result<(), CompletionError> {
    let Some(ToolChoice::Specific { function_names }) = &request.tool_choice else {
        return Ok(());
    };
    let available: Vec<&str> = request
        .tools
        .iter()
        .map(|tool| tool.name.as_str())
        .collect();
    if function_names.is_empty()
        || function_names
            .iter()
            .any(|name| !available.contains(&name.as_str()))
    {
        return Err(CompletionError::RequestError(
            format!(
                "tool_choice must name one or more of the request's tools {available:?}, got {function_names:?}"
            )
            .into(),
        ));
    }
    request
        .tools
        .retain(|tool| function_names.contains(&tool.name));
    request.tool_choice = Some(ToolChoice::Required);
    Ok(())
}

impl<M> NonStrictToolsModel<M>
where
    M: ResponsesModelTools + Clone,
{
    /// Drains the request's tools so Rig never sees, converts or duplicates them.
    fn with_request_tools(&self, request: &mut CompletionRequest) -> Result<M, CompletionError> {
        narrow_to_specific_tools(request)?;
        let mut model = self.0.clone();
        model.model_tools().extend(
            std::mem::take(&mut request.tools)
                .into_iter()
                .map(non_strict_function),
        );
        Ok(model)
    }
}

impl<C> CompletionClient for NonStrictToolsClient<C>
where
    C: CompletionClient,
    C::CompletionModel: ResponsesModelTools,
{
    type CompletionModel = NonStrictToolsModel<C::CompletionModel>;
}

impl<M> CompletionModel for NonStrictToolsModel<M>
where
    M: CompletionModel + ResponsesModelTools,
{
    type Response = M::Response;
    type StreamingResponse = M::StreamingResponse;
    type Client = NonStrictToolsClient<M::Client>;

    fn make(client: &Self::Client, model: impl Into<String>) -> Self {
        Self(M::make(&client.0, model))
    }

    async fn completion(
        &self,
        mut request: CompletionRequest,
    ) -> Result<CompletionResponse<Self::Response>, CompletionError> {
        let model = self.with_request_tools(&mut request)?;
        model.completion(request).await
    }

    async fn stream(
        &self,
        mut request: CompletionRequest,
    ) -> Result<StreamingCompletionResponse<Self::StreamingResponse>, CompletionError> {
        let model = self.with_request_tools(&mut request)?;
        model.stream(request).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::test_support::typeless_tool;
    use serde_json::json;

    #[test]
    fn a_non_strict_function_keeps_its_schema_and_says_strict_false() {
        let tool = typeless_tool();

        let wire = serde_json::to_value(non_strict_function(tool.clone())).unwrap();

        assert_eq!(
            wire,
            json!({
                "type": "function",
                "name": tool.name,
                "description": tool.description,
                "parameters": tool.parameters,
                "strict": false,
            })
        );
    }

    fn model() -> NonStrictToolsModel<ResponsesCompletionModel> {
        let client = rig::providers::openai::Client::new("test-key").unwrap();
        NonStrictToolsClient(client).completion_model("gpt-5.4")
    }

    fn request(tools: Vec<ToolDefinition>, tool_choice: Option<ToolChoice>) -> CompletionRequest {
        CompletionRequest {
            model: None,
            preamble: None,
            chat_history: rig::OneOrMany::one(rig::completion::Message::user("hi")),
            documents: Vec::new(),
            tools,
            temperature: None,
            max_tokens: None,
            tool_choice,
            additional_params: None,
            output_schema: None,
        }
    }

    fn other_tool() -> ToolDefinition {
        ToolDefinition {
            name: "open_app_page".to_string(),
            description: "Open an app page.".to_string(),
            parameters: json!({"type": "object", "properties": {}}),
        }
    }

    fn specific(names: &[&str]) -> Option<ToolChoice> {
        Some(ToolChoice::Specific {
            function_names: names.iter().map(ToString::to_string).collect(),
        })
    }

    #[test]
    fn request_tools_move_to_the_model_once() {
        let model = model();
        let mut request = request(vec![typeless_tool()], None);

        let sent = model.with_request_tools(&mut request).unwrap();

        assert!(request.tools.is_empty());
        assert_eq!(sent.tools, vec![non_strict_function(typeless_tool())]);
        assert!(model.0.tools.is_empty(), "the shared model must stay clean");
    }

    #[test]
    fn a_specific_tool_choice_sends_only_the_named_tools_and_requires_one() {
        let mut request = request(
            vec![other_tool(), typeless_tool()],
            specific(&["interact_app_page"]),
        );

        let sent = model().with_request_tools(&mut request).unwrap();

        assert_eq!(sent.tools, vec![non_strict_function(typeless_tool())]);
        assert_eq!(request.tool_choice, Some(ToolChoice::Required));
    }

    #[test]
    fn a_specific_tool_choice_without_a_matching_tool_fails_before_sending() {
        for names in [&["missing"][..], &[]] {
            let mut request = request(vec![typeless_tool()], specific(names));

            let error = model().with_request_tools(&mut request).err().unwrap();

            assert!(
                matches!(error, CompletionError::RequestError(_)),
                "{names:?}: {error}"
            );
        }
    }
}

//! Optional request settings for callers that build their own Rig requests. Each setting is
//! sent until a provider rejects it for the model; the rejected request is then retried
//! without it, and later requests leave it out.

use super::{
    CompletionClientDyn, CompletionModelDyn, CompletionModelHandle, DynamicResponse,
    DynamicStreamingResponse, ModelLogic, merge_additional_params, raw_stream_items,
};
use crate::history::{History, HistoryThinking};
use futures::StreamExt;
use http::StatusCode;
use rig::agent::AgentBuilder;
use rig::completion::{
    CompletionError, CompletionModel, CompletionRequest, CompletionRequestBuilder,
    CompletionResponse, Message,
};
use rig::http_client;
use rig::streaming::StreamingCompletionResponse;
use rig::wasm_compat::WasmBoxedFuture;
use serde_json::Value;
use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestSetting {
    Temperature,
    Thinking,
    MaxTokens,
}

impl RequestSetting {
    /// Drop order when one error names several: the output cap guards against runaway
    /// generation, so it goes last.
    const ALL: [Self; 3] = [Self::Temperature, Self::Thinking, Self::MaxTokens];

    pub fn label(self) -> &'static str {
        match self {
            Self::Temperature => "Temperature",
            Self::Thinking => "Thinking",
            Self::MaxTokens => "Max Tokens",
        }
    }

    fn named_in(self, message: &str) -> bool {
        let needles: &[&str] = match self {
            Self::Temperature => &["temperature"],
            Self::Thinking => &["reasoning", "think", "effort"],
            Self::MaxTokens => &[
                "max_tokens",
                "max_completion_tokens",
                "max_output_tokens",
                "maxoutputtokens",
                "max tokens",
                "output tokens",
            ],
        };
        needles.iter().any(|needle| message.contains(needle))
    }
}

#[derive(Debug, Clone)]
pub struct RejectedSetting {
    pub setting: RequestSetting,
    pub error: String,
}

#[derive(Default)]
struct Rejections {
    settings: Vec<RejectedSetting>,
    reported: usize,
}

/// The settings a provider rejected during a run, shared by every clone of the model.
#[derive(Clone, Default)]
pub struct RejectedSettings(Arc<Mutex<Rejections>>);

impl RejectedSettings {
    fn lock(&self) -> MutexGuard<'_, Rejections> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn contains(&self, setting: RequestSetting) -> bool {
        self.lock()
            .settings
            .iter()
            .any(|rejected| rejected.setting == setting)
    }

    fn record(&self, setting: RequestSetting, error: &CompletionError) {
        let mut rejections = self.lock();
        if rejections
            .settings
            .iter()
            .any(|rejected| rejected.setting == setting)
        {
            return;
        }
        tracing::warn!(
            setting = setting.label(),
            %error,
            "The provider rejected an optional setting; retrying without it"
        );
        rejections.settings.push(RejectedSetting {
            setting,
            error: error.to_string(),
        });
    }

    /// Rejections not returned by an earlier call.
    pub fn take_unreported(&self) -> Vec<RejectedSetting> {
        let mut rejections = self.lock();
        let unreported = rejections.settings[rejections.reported..].to_vec();
        rejections.reported = rejections.settings.len();
        unreported
    }
}

/// The request names a setting it carried, and the failure is not a rate limit or outage
/// whose message happens to mention one ("8,000 output tokens per minute").
fn rejected_setting(error: &CompletionError, sent: &[RequestSetting]) -> Option<RequestSetting> {
    let message = match error {
        CompletionError::ProviderError(message) => message,
        CompletionError::HttpError(http_client::Error::InvalidStatusCodeWithMessage(
            status,
            message,
        )) if status.is_client_error() && *status != StatusCode::TOO_MANY_REQUESTS => message,
        _ => return None,
    }
    .to_ascii_lowercase();

    let transient = ["rate limit", "rate_limit", "429", "quota", "overload"]
        .iter()
        .any(|needle| message.contains(needle));
    if transient {
        return None;
    }
    sent.iter()
        .copied()
        .find(|setting| setting.named_in(&message))
}

#[derive(Clone)]
pub struct LenientSettings<M> {
    inner: M,
    thinking: Option<Value>,
    droppable: &'static [RequestSetting],
    rejected: RejectedSettings,
}

impl<M> LenientSettings<M> {
    /// `thinking` is the provider body for the caller's thinking level, merged into every
    /// request while the provider accepts it.
    pub fn new(inner: M, thinking: Option<Value>) -> Self {
        Self {
            inner,
            thinking,
            droppable: &RequestSetting::ALL,
            rejected: RejectedSettings::default(),
        }
    }

    /// Drops only the thinking body; a rejected temperature or output cap still fails the
    /// request.
    pub fn thinking_only(inner: M, thinking: Option<Value>) -> Self {
        Self {
            droppable: &[RequestSetting::Thinking],
            ..Self::new(inner, thinking)
        }
    }

    pub fn rejected(&self) -> RejectedSettings {
        self.rejected.clone()
    }

    fn sendable(&self, request: &CompletionRequest) -> Vec<RequestSetting> {
        self.droppable
            .iter()
            .copied()
            .filter(|setting| match setting {
                RequestSetting::Temperature => request.temperature.is_some(),
                RequestSetting::Thinking => self.thinking.is_some(),
                RequestSetting::MaxTokens => request.max_tokens.is_some(),
            })
            .filter(|setting| !self.rejected.contains(*setting))
            .collect()
    }

    fn prepare(&self, mut request: CompletionRequest) -> CompletionRequest {
        if self.rejected.contains(RequestSetting::Temperature) {
            request.temperature = None;
        }
        if self.rejected.contains(RequestSetting::MaxTokens) {
            request.max_tokens = None;
        }
        if !self.rejected.contains(RequestSetting::Thinking) {
            request.additional_params =
                merge_additional_params(request.additional_params.take(), self.thinking.clone());
        }
        request
    }

    /// Every retry sends at least one setting fewer, so this ends after at most one attempt
    /// per setting. A setting another request already dropped is not recorded twice.
    async fn send<T, F, Fut>(
        &self,
        request: CompletionRequest,
        send: F,
    ) -> Result<T, CompletionError>
    where
        F: Fn(CompletionRequest, Vec<RequestSetting>) -> Fut,
        Fut: Future<Output = Result<T, CompletionError>>,
    {
        loop {
            let sent = self.sendable(&request);
            let error = match send(self.prepare(request.clone()), sent.clone()).await {
                Ok(response) => return Ok(response),
                Err(error) => error,
            };
            match rejected_setting(&error, &sent) {
                Some(setting) => self.rejected.record(setting, &error),
                None => return Err(error),
            }
        }
    }
}

impl<M> LenientSettings<M>
where
    M: CompletionModel,
    M::StreamingResponse: 'static,
{
    /// Most streaming clients send the request on the first poll, so a rejection arrives as the
    /// first item rather than from `stream`. It is surfaced as the call's error so `send` can
    /// retry; anything else, an unrelated first error included, is streamed as before.
    async fn open_stream(
        &self,
        request: CompletionRequest,
        sent: Vec<RequestSetting>,
    ) -> Result<StreamingCompletionResponse<M::StreamingResponse>, CompletionError> {
        let mut stream = self.inner.stream(request).await?;
        if sent.is_empty() {
            return Ok(stream);
        }
        let first = match stream.next().await {
            Some(Err(error)) if rejected_setting(&error, &sent).is_some() => return Err(error),
            first => first,
        };
        let items = futures::stream::iter(first).chain(stream).flat_map(|item| {
            futures::stream::iter(match item {
                Ok(item) => raw_stream_items(item, |response| response)
                    .into_iter()
                    .map(Ok)
                    .collect::<Vec<_>>(),
                Err(error) => vec![Err(error)],
            })
        });
        Ok(StreamingCompletionResponse::stream(Box::pin(items)))
    }
}

/// The body for `history` without its thinking level, and the complete body with it for
/// [`LenientSettings`] to merge over the first until the provider rejects it. The second is
/// `None` when the provider maps the level to nothing, so no request is retried in vain.
pub fn split_thinking_params(
    history: &mut History,
    params: impl Fn(&History) -> anyhow::Result<Option<Value>>,
) -> anyhow::Result<(Option<Value>, Option<Value>)> {
    let with_thinking = params(&*history)?;
    let Some(thinking) = history.thinking.take() else {
        return Ok((with_thinking, None));
    };
    let without_thinking = params(&*history);
    history.thinking = Some(thinking);
    let without_thinking = without_thinking?;
    let thinking = with_thinking.filter(|body| Some(body) != without_thinking.as_ref());
    Ok((without_thinking, thinking))
}

/// The complete agent body for `thinking` on `model_name`, for [`lenient_agent`]. A level the
/// provider cannot map is left out with a warning.
pub fn thinking_params(
    model: &dyn ModelLogic,
    model_name: &str,
    thinking: HistoryThinking,
) -> Option<Value> {
    let mut history = History::new(model_name.to_string(), Vec::new());
    history.thinking = Some(thinking);
    let split = split_thinking_params(&mut history, |history| {
        Ok(model.agent_settings(&Some(history.clone()))?.params)
    });
    match split {
        Ok((_, thinking)) => thinking,
        Err(error) => {
            tracing::warn!(
                model = model_name,
                ?thinking,
                %error,
                "The provider cannot map the thinking level; leaving it out"
            );
            None
        }
    }
}

/// `model` sending `thinking` merged over each request's own body until the provider rejects
/// it; the rejected request is retried without it, and later requests leave it out.
pub fn lenient_thinking<'a>(
    model: CompletionModelHandle<'a>,
    thinking: Option<Value>,
) -> CompletionModelHandle<'a> {
    if thinking.is_none() {
        return model;
    }
    CompletionModelHandle::new(Arc::new(LenientHandle(LenientSettings::thinking_only(
        model, thinking,
    ))))
}

/// `client.agent(model_name)` with [`lenient_thinking`]. `thinking` is merged over the agent's
/// own `additional_params`, so the caller can pass the complete body there and keep the body
/// without thinking as the agent's params.
pub fn lenient_agent<'a>(
    client: &(dyn CompletionClientDyn + '_),
    model_name: &str,
    thinking: Option<Value>,
) -> AgentBuilder<CompletionModelHandle<'a>> {
    let model = CompletionModelHandle::new(Arc::from(client.completion_model(model_name)));
    AgentBuilder::new(lenient_thinking(model, thinking))
}

/// Erases [`LenientSettings`] back into a handle without re-mapping each response the way the
/// blanket [`CompletionModelDyn`] impl would.
#[derive(Clone)]
struct LenientHandle<'a>(LenientSettings<CompletionModelHandle<'a>>);

impl CompletionModelDyn for LenientHandle<'_> {
    fn completion(
        &self,
        request: CompletionRequest,
    ) -> WasmBoxedFuture<'_, Result<CompletionResponse<DynamicResponse>, CompletionError>> {
        Box::pin(CompletionModel::completion(&self.0, request))
    }

    fn stream(
        &self,
        request: CompletionRequest,
    ) -> WasmBoxedFuture<
        '_,
        Result<StreamingCompletionResponse<DynamicStreamingResponse>, CompletionError>,
    > {
        Box::pin(CompletionModel::stream(&self.0, request))
    }

    fn completion_request(
        &self,
        prompt: Message,
    ) -> CompletionRequestBuilder<CompletionModelHandle<'_>> {
        CompletionRequestBuilder::new(CompletionModelHandle::new(Arc::new(self.clone())), prompt)
    }
}

impl<M> CompletionModel for LenientSettings<M>
where
    M: CompletionModel,
    M::StreamingResponse: 'static,
{
    type Response = M::Response;
    type StreamingResponse = M::StreamingResponse;
    type Client = M::Client;

    fn make(client: &Self::Client, model: impl Into<String>) -> Self {
        Self::new(M::make(client, model), None)
    }

    async fn completion(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse<Self::Response>, CompletionError> {
        self.send(request, |request, _| self.inner.completion(request))
            .await
    }

    async fn stream(
        &self,
        request: CompletionRequest,
    ) -> Result<StreamingCompletionResponse<Self::StreamingResponse>, CompletionError> {
        self.send(request, |request, sent| self.open_stream(request, sent))
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rig::OneOrMany;
    use rig::completion::{AssistantContent, Usage};
    use serde_json::json;

    type Rule = fn(&CompletionRequest) -> Option<&'static str>;

    #[derive(Clone)]
    struct Scripted {
        rule: Rule,
        sent: Arc<Mutex<Vec<CompletionRequest>>>,
    }

    impl Scripted {
        fn new(rule: Rule) -> Self {
            Self {
                rule,
                sent: Arc::default(),
            }
        }

        fn sent(&self) -> Vec<CompletionRequest> {
            self.sent.lock().unwrap().clone()
        }
    }

    impl CompletionModel for Scripted {
        type Response = ();
        type StreamingResponse = ();
        type Client = ();

        fn make(_: &Self::Client, _: impl Into<String>) -> Self {
            unreachable!()
        }

        async fn completion(
            &self,
            request: CompletionRequest,
        ) -> Result<CompletionResponse<()>, CompletionError> {
            let rejection = (self.rule)(&request);
            self.sent.lock().unwrap().push(request);
            match rejection {
                Some(message) => Err(CompletionError::ProviderError(message.to_string())),
                None => Ok(CompletionResponse {
                    choice: OneOrMany::one(AssistantContent::text("ok")),
                    usage: Usage::new(),
                    raw_response: (),
                    message_id: None,
                }),
            }
        }

        /// Like an SSE client, the request fails on the first poll, not when the stream opens.
        async fn stream(
            &self,
            request: CompletionRequest,
        ) -> Result<StreamingCompletionResponse<()>, CompletionError> {
            let rejection = (self.rule)(&request);
            self.sent.lock().unwrap().push(request);
            let first = match rejection {
                Some(message) => Err(CompletionError::ProviderError(message.to_string())),
                None => Ok(rig::streaming::RawStreamingChoice::Message(
                    "ok".to_string(),
                )),
            };
            Ok(StreamingCompletionResponse::stream(Box::pin(
                futures::stream::iter([first]),
            )))
        }
    }

    fn carries_thinking(request: &CompletionRequest) -> bool {
        request
            .additional_params
            .as_ref()
            .is_some_and(|params| params.get("reasoning_effort").is_some())
    }

    fn lenient(rule: Rule) -> (LenientSettings<Scripted>, Scripted) {
        let inner = Scripted::new(rule);
        let model = LenientSettings::new(inner.clone(), Some(json!({"reasoning_effort": "none"})));
        (model, inner)
    }

    async fn send(model: &LenientSettings<Scripted>) -> Result<(), CompletionError> {
        CompletionModel::completion_request(model, "Read this page.")
            .temperature(0.1)
            .max_tokens(4096)
            .send()
            .await
            .map(|_| ())
    }

    #[tokio::test]
    async fn a_rejected_thinking_level_is_dropped_for_the_rest_of_the_run() {
        let (model, inner) = lenient(|request| {
            carries_thinking(request).then_some(
                "Unsupported value: 'reasoning_effort' does not support 'none' with this model.",
            )
        });

        send(&model).await.unwrap();
        send(&model).await.unwrap();

        let sent = inner.sent();
        assert_eq!(sent.len(), 3, "one rejection, then two requests without it");
        assert!(!carries_thinking(&sent[1]) && !carries_thinking(&sent[2]));
        assert_eq!(sent[2].temperature, Some(0.1));
        assert_eq!(sent[2].max_tokens, Some(4096));
        let rejected = model.rejected().take_unreported();
        assert_eq!(rejected.len(), 1);
        assert_eq!(rejected[0].setting, RequestSetting::Thinking);
        assert!(model.rejected().take_unreported().is_empty());
    }

    #[tokio::test]
    async fn settings_named_together_are_dropped_one_at_a_time_temperature_first() {
        let (model, inner) = lenient(|request| {
            request
                .temperature
                .is_some()
                .then_some("`temperature` may only be set to 1 when thinking is enabled")
        });

        send(&model).await.unwrap();

        let sent = inner.sent();
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[1].temperature, None);
        assert!(carries_thinking(&sent[1]), "thinking must survive");
    }

    fn rejects_thinking(request: &CompletionRequest) -> Option<&'static str> {
        carries_thinking(request).then_some(
            "Unsupported value: 'reasoning_effort' does not support 'none' with this model.",
        )
    }

    #[tokio::test]
    async fn a_thinking_only_model_keeps_a_rejected_temperature_and_drops_only_thinking() {
        let inner = Scripted::new(|request| {
            request
                .temperature
                .is_some()
                .then_some("`temperature` is not supported with this model")
                .or_else(|| rejects_thinking(request))
        });
        let model = LenientSettings::thinking_only(
            inner.clone(),
            Some(json!({"reasoning_effort": "none"})),
        );

        assert!(send(&model).await.is_err());
        assert_eq!(inner.sent().len(), 1);

        let inner = Scripted::new(rejects_thinking);
        let model = LenientSettings::thinking_only(
            inner.clone(),
            Some(json!({"reasoning_effort": "none"})),
        );
        send(&model).await.unwrap();
        let sent = inner.sent();
        assert_eq!(sent.len(), 2);
        assert!(!carries_thinking(&sent[1]));
        assert_eq!(sent[1].temperature, Some(0.1));
        assert_eq!(sent[1].max_tokens, Some(4096));
    }

    async fn stream_text(model: &LenientSettings<Scripted>) -> Result<String, CompletionError> {
        let mut stream = CompletionModel::completion_request(model, "Read this page.")
            .stream()
            .await?;
        let mut text = String::new();
        while let Some(item) = stream.next().await {
            if let rig::streaming::StreamedAssistantContent::Text(chunk) = item? {
                text.push_str(&chunk.text);
            }
        }
        Ok(text)
    }

    #[tokio::test]
    async fn a_stream_whose_first_item_rejects_thinking_is_reopened_without_it() {
        let (model, inner) = lenient(rejects_thinking);

        assert_eq!(stream_text(&model).await.unwrap(), "ok");
        let sent = inner.sent();
        assert_eq!(sent.len(), 2);
        assert!(carries_thinking(&sent[0]) && !carries_thinking(&sent[1]));

        let (model, inner) = lenient(|_| Some("Image exceeds 5 MB maximum"));
        assert!(stream_text(&model).await.is_err());
        assert_eq!(inner.sent().len(), 1);
    }

    struct ScriptedClient(Scripted);

    impl CompletionClientDyn for ScriptedClient {
        fn completion_model<'a>(&self, _: &str) -> Box<dyn CompletionModelDyn + 'a> {
            Box::new(self.0.clone())
        }

        fn agent<'a>(&self, _: &str) -> AgentBuilder<CompletionModelHandle<'a>> {
            unreachable!()
        }
    }

    #[tokio::test]
    async fn a_lenient_agent_retries_without_a_rejected_thinking_level() {
        let inner = Scripted::new(rejects_thinking);
        let client = ScriptedClient(inner.clone());
        let agent = lenient_agent(&client, "o3", Some(json!({"reasoning_effort": "none"}))).build();

        rig::completion::Completion::completion(&agent, "Read this page.", Vec::<Message>::new())
            .await
            .unwrap()
            .send()
            .await
            .unwrap();

        let sent = inner.sent();
        assert_eq!(sent.len(), 2);
        assert!(carries_thinking(&sent[0]) && !carries_thinking(&sent[1]));
    }

    struct ThinkingModel;

    #[async_trait::async_trait]
    impl ModelLogic for ThinkingModel {
        async fn provider(&self) -> anyhow::Result<crate::llm::ModelConstructor> {
            unreachable!()
        }

        async fn default_model(&self) -> Option<String> {
            None
        }

        fn additional_params(&self, history: &Option<History>) -> Option<Value> {
            let history = history.as_ref()?;
            let effort = history
                .thinking
                .filter(|thinking| *thinking != HistoryThinking::Off)
                .map(HistoryThinking::openai_reasoning_effort);
            Some(match effort {
                Some(effort) => json!({"model": history.model, "reasoning_effort": effort}),
                None => json!({"model": history.model}),
            })
        }
    }

    struct ScriptedLogic(Scripted);

    #[async_trait::async_trait]
    impl ModelLogic for ScriptedLogic {
        async fn provider(&self) -> anyhow::Result<crate::llm::ModelConstructor> {
            Ok(crate::llm::ModelConstructor {
                inner: Box::new(ScriptedClient(self.0.clone())),
            })
        }

        async fn default_model(&self) -> Option<String> {
            None
        }

        fn additional_params(&self, history: &Option<History>) -> Option<Value> {
            ThinkingModel.additional_params(history)
        }
    }

    #[tokio::test]
    async fn invoke_retries_without_a_rejected_thinking_level_streamed_or_not() {
        let callback: crate::llm::LLMCallback = Arc::new(|_| Box::pin(async { Ok(()) }));
        for lambda in [None, Some(callback)] {
            let inner = Scripted::new(rejects_thinking);
            let mut history = History::new(
                "o3".to_string(),
                vec![crate::history::HistoryMessage::from_string(
                    crate::history::Role::User,
                    "Read this page.",
                )],
            );
            history.thinking = Some(HistoryThinking::Mid);

            ScriptedLogic(inner.clone())
                .invoke(&history, lambda)
                .await
                .unwrap();

            let sent = inner.sent();
            assert_eq!(sent.len(), 2);
            assert!(carries_thinking(&sent[0]) && !carries_thinking(&sent[1]));
            assert_eq!(
                sent[1].additional_params,
                Some(json!({"model": "o3"})),
                "the rest of the body stays"
            );
        }
    }

    #[test]
    fn thinking_params_are_the_complete_body_and_none_when_the_level_maps_to_nothing() {
        assert_eq!(
            thinking_params(&ThinkingModel, "o3", HistoryThinking::Mid),
            Some(json!({"model": "o3", "reasoning_effort": "medium"}))
        );
        assert_eq!(
            thinking_params(&ThinkingModel, "o3", HistoryThinking::Off),
            None
        );
    }

    #[tokio::test]
    async fn rate_limits_and_unrelated_errors_are_returned_untouched() {
        let rules: [Rule; 2] = [
            |_| Some("This request would exceed the rate limit of 8,000 output tokens per minute"),
            |_| Some("Image exceeds 5 MB maximum"),
        ];
        for rule in rules {
            let (model, inner) = lenient(rule);

            assert!(send(&model).await.is_err());
            assert_eq!(inner.sent().len(), 1);
            assert!(model.rejected().take_unreported().is_empty());
        }
    }
}

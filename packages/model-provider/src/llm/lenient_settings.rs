//! Optional request settings for callers that build their own Rig requests. Each setting is
//! sent until a provider rejects it for the model; the rejected request is then retried
//! without it, and later requests leave it out.

use super::merge_additional_params;
use http::StatusCode;
use rig::completion::{CompletionError, CompletionModel, CompletionRequest, CompletionResponse};
use rig::http_client;
use rig::streaming::StreamingCompletionResponse;
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
    rejected: RejectedSettings,
}

impl<M> LenientSettings<M> {
    /// `thinking` is the provider body for the caller's thinking level, merged into every
    /// request while the provider accepts it.
    pub fn new(inner: M, thinking: Option<Value>) -> Self {
        Self {
            inner,
            thinking,
            rejected: RejectedSettings::default(),
        }
    }

    pub fn rejected(&self) -> RejectedSettings {
        self.rejected.clone()
    }

    fn sendable(&self, request: &CompletionRequest) -> Vec<RequestSetting> {
        RequestSetting::ALL
            .into_iter()
            .filter(|setting| match setting {
                RequestSetting::Temperature => request.temperature.is_some(),
                RequestSetting::Thinking => self.thinking.is_some(),
                RequestSetting::MaxTokens => request.max_tokens.is_some(),
            })
            .filter(|setting| !self.rejected.contains(*setting))
            .collect()
    }

    fn prepare(
        &self,
        mut request: CompletionRequest,
        sent: &[RequestSetting],
    ) -> CompletionRequest {
        if !sent.contains(&RequestSetting::Temperature) {
            request.temperature = None;
        }
        if !sent.contains(&RequestSetting::MaxTokens) {
            request.max_tokens = None;
        }
        if sent.contains(&RequestSetting::Thinking) {
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
        F: Fn(CompletionRequest) -> Fut,
        Fut: Future<Output = Result<T, CompletionError>>,
    {
        loop {
            let sent = self.sendable(&request);
            let error = match send(self.prepare(request.clone(), &sent)).await {
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

impl<M: CompletionModel> CompletionModel for LenientSettings<M> {
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
        self.send(request, |request| self.inner.completion(request))
            .await
    }

    async fn stream(
        &self,
        request: CompletionRequest,
    ) -> Result<StreamingCompletionResponse<Self::StreamingResponse>, CompletionError> {
        self.send(request, |request| self.inner.stream(request))
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

        async fn stream(
            &self,
            _: CompletionRequest,
        ) -> Result<StreamingCompletionResponse<()>, CompletionError> {
            unreachable!()
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
        model
            .completion_request("Read this page.")
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

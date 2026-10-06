//! Bits served from an OpenAI-compatible endpoint instead of in-process: models hosted on a
//! Flow-Like device and, on a device itself, the Local and MLX Bits its model host serves.

use std::{collections::HashSet, fmt, sync::Arc};

use flow_like_model_provider::provider::ModelApiSurface;
use flow_like_types::{Result, Value, anyhow, async_trait, bail, json};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use super::llm::ModelUsageContext;
use crate::{
    bit::{Bit, BitTypes},
    state::FlowLikeState,
};

pub const DEVICE_PROVIDER_NAME: &str = "device";

pub fn is_device_provider(provider_name: &str) -> bool {
    provider_name
        .trim()
        .eq_ignore_ascii_case(DEVICE_PROVIDER_NAME)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceModelKind {
    Chat,
    Vision,
    Embedding,
}

/// The hosted model a `device` Bit names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceModelTarget {
    pub device_id: String,
    /// Hosted model id on the device's model gateway.
    pub model: String,
    pub kind: DeviceModelKind,
    /// The Bit's name, for prompts and errors.
    pub display_name: String,
}

#[derive(Deserialize)]
struct DeviceModelParams {
    #[serde(default)]
    device_id: String,
    #[serde(default)]
    model: String,
    #[serde(default)]
    kind: Option<DeviceModelKind>,
    #[serde(default)]
    api_surface: Option<ModelApiSurface>,
}

impl DeviceModelTarget {
    /// `None` for Bits of every other provider.
    pub fn from_bit(bit: &Bit) -> Option<Result<Self>> {
        let provider = bit.model_provider()?;
        if !is_device_provider(&provider.provider_name) {
            return None;
        }
        let params = Value::Object(provider.params.unwrap_or_default().into_iter().collect());
        Some(Self::parse(bit, params, provider.api_surface))
    }

    fn parse(bit: &Bit, params: Value, api_surface: Option<ModelApiSurface>) -> Result<Self> {
        let params: DeviceModelParams = json::from_value(params).map_err(|error| {
            anyhow!("Device Bit {} has invalid provider params: {error}", bit.id)
        })?;
        let device_id = params.device_id.trim();
        let model = params.model.trim();
        if device_id.is_empty() || model.is_empty() {
            bail!(
                "Device Bit {} must name both a device_id and a model",
                bit.id
            );
        }
        if api_surface
            .or(params.api_surface)
            .is_some_and(|surface| surface.is_responses())
        {
            bail!(
                "Device Bit {} declares the Responses API, but device models serve chat_completions",
                bit.id
            );
        }
        Ok(Self {
            device_id: device_id.to_string(),
            model: model.to_string(),
            kind: resolve_kind(bit, params.kind)?,
            display_name: display_name(bit),
        })
    }
}

fn resolve_kind(bit: &Bit, declared: Option<DeviceModelKind>) -> Result<DeviceModelKind> {
    let implied = match &bit.bit_type {
        BitTypes::Llm => DeviceModelKind::Chat,
        BitTypes::Vlm => DeviceModelKind::Vision,
        BitTypes::Embedding => DeviceModelKind::Embedding,
        other => bail!(
            "Device Bit {} is a {other:?} Bit; device models are LLM, VLM or Embedding Bits",
            bit.id
        ),
    };
    let kind = declared.unwrap_or(implied);
    if (kind == DeviceModelKind::Embedding) != (implied == DeviceModelKind::Embedding) {
        bail!(
            "Device Bit {} is a {:?} Bit but names a {kind:?} model",
            bit.id,
            bit.bit_type
        );
    }
    Ok(kind)
}

fn display_name(bit: &Bit) -> String {
    bit.meta
        .get("en")
        .or_else(|| bit.meta.values().next())
        .map(|meta| meta.name.trim())
        .filter(|name| !name.is_empty())
        .unwrap_or(&bit.id)
        .to_string()
}

/// An OpenAI-compatible endpoint serving one model.
#[derive(Clone, PartialEq, Eq)]
pub struct ModelEndpoint {
    /// API base such as `http://127.0.0.1:41234/v1`; requests go to `{base_url}/chat/completions`
    /// and `{base_url}/embeddings`.
    pub base_url: String,
    pub bearer: String,
    /// Model name sent with each request.
    pub model: String,
}

impl fmt::Debug for ModelEndpoint {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("ModelEndpoint")
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .finish_non_exhaustive()
    }
}

impl ModelEndpoint {
    /// A new port, model or bearer is a new client.
    pub(crate) fn cache_key(&self) -> String {
        let bearer = blake3::hash(self.bearer.as_bytes()).to_hex();
        format!(
            "endpoint:{}#{}#{}",
            self.base_url.trim_end_matches('/'),
            self.model,
            &bearer[..16]
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Interaction {
    /// The connector may ask the user, for example to unlock the device. `run_label` is the id of
    /// the asking run: the prompt names it, and a decline holds for the rest of that run.
    Allowed { run_label: Option<String> },
    /// The connector answers without asking the user.
    Forbidden,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelUnavailableReason {
    /// No device connector exists where the run executes, as on a cloud executor.
    ServerExecution,
    DeviceLockedDeclined,
    DeviceOffline,
    DeviceRemoved,
    ModelMissing,
    NotGranted,
}

impl ModelUnavailableReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ServerExecution => "server_execution",
            Self::DeviceLockedDeclined => "device_locked_declined",
            Self::DeviceOffline => "device_offline",
            Self::DeviceRemoved => "device_removed",
            Self::ModelMissing => "model_missing",
            Self::NotGranted => "not_granted",
        }
    }

    fn clause(self) -> &'static str {
        match self {
            Self::ServerExecution => {
                "which this host cannot reach; device models run from the desktop app"
            }
            Self::DeviceLockedDeclined => "which stayed locked",
            Self::DeviceOffline => "which is offline",
            Self::DeviceRemoved => "which was removed or is no longer shared with you",
            Self::ModelMissing => "which does not host this model",
            Self::NotGranted => "which has not granted you model access",
        }
    }
}

/// A device model that cannot serve this call. Model selection moves on to its next candidate;
/// a hard-coded Bit fails with `message`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelUnavailable {
    pub reason: ModelUnavailableReason,
    pub message: String,
}

impl ModelUnavailable {
    pub fn new(reason: ModelUnavailableReason, message: impl Into<String>) -> Self {
        Self {
            reason,
            message: message.into(),
        }
    }

    /// "Model 'Qwen3 8B' runs on device 'GPU box', which stayed locked."
    pub fn for_target(
        reason: ModelUnavailableReason,
        target: &DeviceModelTarget,
        device_name: Option<&str>,
    ) -> Self {
        let device = device_name
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or(&target.device_id);
        Self::new(
            reason,
            format!(
                "Model '{}' runs on device '{device}', {}.",
                target.display_name,
                reason.clause()
            ),
        )
    }
}

impl fmt::Display for ModelUnavailable {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ModelUnavailable {}

/// The [`ModelUnavailable`] behind `error`, also when context was added on the way up.
pub fn model_unavailable(error: &flow_like_types::Error) -> Option<&ModelUnavailable> {
    error
        .chain()
        .find_map(|cause| cause.downcast_ref::<ModelUnavailable>())
}

/// Lets model selection try the Bits that run outside this process and move on when one is
/// unavailable: on a device, the Bits its model host serves through the router, and models on
/// other devices through the connector. Within one selection a device that proved unavailable is
/// not asked again, so a decline or an offline device costs one prompt or timeout.
#[derive(Clone)]
pub struct DeviceModelProbe {
    connector: Option<Arc<dyn DeviceModelConnector>>,
    router: Option<Arc<dyn LocalModelRouter>>,
    interaction: Interaction,
    unavailable_devices: Arc<Mutex<HashSet<String>>>,
}

impl DeviceModelProbe {
    pub fn new(connector: Arc<dyn DeviceModelConnector>, interaction: Interaction) -> Self {
        Self {
            connector: Some(connector),
            router: None,
            interaction,
            unavailable_devices: Arc::default(),
        }
    }

    /// `None` for a host with neither a connector nor a router.
    pub fn for_host(
        connector: Option<Arc<dyn DeviceModelConnector>>,
        router: Option<Arc<dyn LocalModelRouter>>,
        interaction: Interaction,
    ) -> Option<Self> {
        (connector.is_some() || router.is_some()).then(|| Self {
            connector,
            router,
            interaction,
            unavailable_devices: Arc::default(),
        })
    }

    /// A copy for one selection that has not found any device unavailable yet.
    pub(crate) fn for_selection(&self) -> Self {
        Self {
            unavailable_devices: Arc::default(),
            ..self.clone()
        }
    }

    /// Whether this host's model host may serve `bit`, which then needs no runtime in this process.
    pub(crate) fn may_route(&self, bit: &Bit) -> bool {
        self.router.is_some() && routable(bit)
    }

    /// Whether `bit` can serve this selection from outside this process; `None` for a Bit that
    /// would run in-process here.
    pub(crate) async fn availability(&self, bit: &Bit) -> Option<bool> {
        if let Some(served) = self.routed(bit).await {
            return Some(served);
        }
        if bit.is_device_model() {
            return Some(self.reachable(bit).await);
        }
        None
    }

    async fn routed(&self, bit: &Bit) -> Option<bool> {
        let router = self.router.as_ref().filter(|_| routable(bit))?;
        let answer = router.route(bit).await?;
        if let Err(error) = &answer {
            tracing::debug!(bit_id = %bit.id, %error, "Skipping a model the model host cannot serve");
        }
        Some(answer.is_ok())
    }

    async fn reachable(&self, bit: &Bit) -> bool {
        let Some(connector) = &self.connector else {
            return false;
        };
        let target = match DeviceModelTarget::from_bit(bit) {
            Some(Ok(target)) => target,
            Some(Err(error)) => {
                tracing::debug!(bit_id = %bit.id, %error, "Skipping malformed device model");
                return false;
            }
            None => return false,
        };
        if self.unavailable_devices.lock().contains(&target.device_id) {
            return false;
        }
        let Err(unavailable) = connector.connect(&target, self.interaction.clone()).await else {
            return true;
        };
        tracing::debug!(
            bit_id = %bit.id,
            reason = unavailable.reason.as_str(),
            "Skipping unavailable device model"
        );
        if unavailable.reason != ModelUnavailableReason::ModelMissing {
            self.unavailable_devices.lock().insert(target.device_id);
        }
        false
    }
}

/// Where `bit` is served when it does not run in-process: this device's model host through the
/// router, or another device through the connector. `None` keeps the in-process path. The run in
/// `usage_context` labels any prompt the connector shows.
pub(crate) async fn resolve_endpoint(
    bit: &Bit,
    state: &FlowLikeState,
    usage_context: Option<&ModelUsageContext>,
) -> Result<Option<ModelEndpoint>> {
    let target = DeviceModelTarget::from_bit(bit).transpose()?;
    if target.is_none() && !served_by_model_host(bit) {
        return Ok(None);
    }
    if let Some(router) = &state.local_model_router
        && let Some(endpoint) = router.route(bit).await
    {
        return endpoint.map(Some);
    }
    let Some(target) = target else {
        return Ok(None);
    };
    let Some(connector) = &state.device_model_connector else {
        return Err(ModelUnavailable::for_target(
            ModelUnavailableReason::ServerExecution,
            &target,
            None,
        )
        .into());
    };
    let run_label = usage_context.and_then(|context| context.run_id.clone());
    let endpoint = connector
        .connect(&target, Interaction::Allowed { run_label })
        .await?;
    Ok(Some(endpoint))
}

fn served_by_model_host(bit: &Bit) -> bool {
    bit.is_mlx_model()
        || bit
            .model_provider()
            .is_some_and(|provider| provider.provider_name.trim().eq_ignore_ascii_case("local"))
}

/// The Bits a router may serve: Local, MLX and `device` Bits.
fn routable(bit: &Bit) -> bool {
    bit.is_device_model() || served_by_model_host(bit)
}

/// Reaches models hosted on other devices. A host without one cannot reach device models.
#[async_trait]
pub trait DeviceModelConnector: Send + Sync {
    /// May prompt the user when `interaction` allows it, so no caller holds a lock across it.
    async fn connect(
        &self,
        target: &DeviceModelTarget,
        interaction: Interaction,
    ) -> std::result::Result<ModelEndpoint, ModelUnavailable>;
}

/// On a device, serves its Local and MLX Bits, and its own `device` Bits, from the device's model
/// host instead of an in-process runtime. `None` means the host does not serve the Bit.
#[async_trait]
pub trait LocalModelRouter: Send + Sync {
    async fn route(&self, bit: &Bit) -> Option<Result<ModelEndpoint>>;
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use crate::bit::{BitModelClassification, LLMParameters, Metadata};
    use flow_like_model_provider::provider::{
        EmbeddingModelProvider, ModelProvider, Pooling, Prefix,
    };
    use std::{
        collections::HashMap,
        sync::atomic::{AtomicUsize, Ordering},
    };

    pub(crate) fn endpoint(base_url: &str) -> ModelEndpoint {
        ModelEndpoint {
            base_url: base_url.to_string(),
            bearer: "loopback-token".to_string(),
            model: "qwen3-8b".to_string(),
        }
    }

    fn device_provider(device_id: &str, model: &str) -> ModelProvider {
        ModelProvider {
            provider_name: DEVICE_PROVIDER_NAME.to_string(),
            model_id: None,
            version: None,
            api_surface: None,
            params: Some(HashMap::from([
                ("device_id".to_string(), Value::from(device_id)),
                ("model".to_string(), Value::from(model)),
            ])),
        }
    }

    pub(crate) fn device_llm_bit(id: &str, device_id: &str) -> Bit {
        let mut bit = Bit {
            id: id.to_string(),
            bit_type: BitTypes::Llm,
            parameters: json::to_value(LLMParameters {
                context_length: 32_000,
                model_classification: BitModelClassification::default(),
                provider: device_provider(device_id, "qwen3-8b"),
            })
            .unwrap(),
            ..Bit::default()
        };
        bit.meta.insert(
            "en".to_string(),
            Metadata {
                name: "Qwen3 8B".to_string(),
                ..Metadata::default()
            },
        );
        bit
    }

    pub(crate) fn embedding_bit(id: &str, provider: ModelProvider) -> Bit {
        Bit {
            id: id.to_string(),
            bit_type: BitTypes::Embedding,
            parameters: json::to_value(EmbeddingModelProvider {
                languages: vec!["en".to_string()],
                vector_length: 2,
                input_length: 512,
                prefix: Prefix {
                    query: "query: ".to_string(),
                    paragraph: String::new(),
                },
                pooling: Pooling::Mean,
                provider,
                remote: None,
            })
            .unwrap(),
            ..Bit::default()
        }
    }

    pub(crate) fn device_embedding_bit(id: &str, device_id: &str) -> Bit {
        embedding_bit(id, device_provider(device_id, "embed-small"))
    }

    pub(crate) struct FakeConnector {
        answer: std::result::Result<ModelEndpoint, ModelUnavailableReason>,
        calls: AtomicUsize,
        pub(crate) interactions: parking_lot::Mutex<Vec<Interaction>>,
    }

    impl FakeConnector {
        pub(crate) fn serving(endpoint: ModelEndpoint) -> Arc<Self> {
            Self::answering(Ok(endpoint))
        }

        pub(crate) fn failing(reason: ModelUnavailableReason) -> Arc<Self> {
            Self::answering(Err(reason))
        }

        fn answering(
            answer: std::result::Result<ModelEndpoint, ModelUnavailableReason>,
        ) -> Arc<Self> {
            Arc::new(Self {
                answer,
                calls: AtomicUsize::new(0),
                interactions: parking_lot::Mutex::new(Vec::new()),
            })
        }

        pub(crate) fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl DeviceModelConnector for FakeConnector {
        async fn connect(
            &self,
            target: &DeviceModelTarget,
            interaction: Interaction,
        ) -> std::result::Result<ModelEndpoint, ModelUnavailable> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.interactions.lock().push(interaction);
            self.answer
                .clone()
                .map_err(|reason| ModelUnavailable::for_target(reason, target, Some("GPU box")))
        }
    }

    /// Serves every Bit whose provider is `local` or `device`.
    pub(crate) struct FakeRouter(pub(crate) ModelEndpoint);

    #[async_trait]
    impl LocalModelRouter for FakeRouter {
        async fn route(&self, bit: &Bit) -> Option<Result<ModelEndpoint>> {
            let provider = bit.model_provider()?;
            let name = provider.provider_name.trim();
            (name.eq_ignore_ascii_case("local") || is_device_provider(name))
                .then(|| Ok(self.0.clone()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{testing::*, *};

    fn with_params(mut bit: Bit, params: Value) -> Bit {
        let mut parameters = bit.parameters.clone();
        parameters["provider"]["params"] = params;
        bit.parameters = parameters;
        bit
    }

    #[test]
    fn device_bits_parse_their_target() {
        let target = device_llm_bit("bit-1", "dev-1")
            .device_model_target()
            .expect("a device Bit")
            .expect("valid params");
        assert_eq!(
            target,
            DeviceModelTarget {
                device_id: "dev-1".to_string(),
                model: "qwen3-8b".to_string(),
                kind: DeviceModelKind::Chat,
                display_name: "Qwen3 8B".to_string(),
            }
        );

        let embedding = device_embedding_bit("embed-1", "dev-1")
            .device_model_target()
            .unwrap()
            .unwrap();
        assert_eq!(embedding.kind, DeviceModelKind::Embedding);
        assert_eq!(embedding.display_name, "embed-1");
        assert!(device_llm_bit("bit-1", "dev-1").is_device_model());
    }

    #[test]
    fn other_providers_are_not_device_models() {
        let bit = embedding_bit(
            "local-embedding",
            flow_like_model_provider::provider::ModelProvider {
                provider_name: "Local".to_string(),
                model_id: None,
                version: None,
                api_surface: None,
                params: None,
            },
        );
        assert!(bit.device_model_target().is_none());
        assert!(!bit.is_device_model());
    }

    #[test]
    fn malformed_device_bits_are_rejected_with_the_bit_id() {
        let cases = [
            json::json!({"device_id": "dev-1"}),
            json::json!({"device_id": " ", "model": "qwen3-8b"}),
            json::json!({"device_id": "dev-1", "model": "qwen3-8b", "kind": "embedding"}),
            json::json!({"device_id": "dev-1", "model": "qwen3-8b", "api_surface": "responses"}),
            json::json!({"device_id": "dev-1", "model": "qwen3-8b", "kind": "speech"}),
        ];
        for params in cases {
            let bit = with_params(device_llm_bit("bad-bit", "dev-1"), params.clone());
            let error = bit
                .device_model_target()
                .expect("still a device Bit")
                .expect_err(&format!("{params} must be rejected"));
            assert!(error.to_string().contains("bad-bit"), "{error}");
        }

        let vision = with_params(
            device_llm_bit("vision-bit", "dev-1"),
            json::json!({"device_id": "dev-1", "model": "qwen-vl", "kind": "vision"}),
        );
        assert_eq!(
            vision.device_model_target().unwrap().unwrap().kind,
            DeviceModelKind::Vision
        );
    }

    #[test]
    fn unavailability_survives_added_context_and_names_model_and_device() {
        let target = device_llm_bit("bit-1", "dev-1")
            .device_model_target()
            .unwrap()
            .unwrap();
        let error: flow_like_types::Error = ModelUnavailable::for_target(
            ModelUnavailableReason::DeviceLockedDeclined,
            &target,
            Some("GPU box"),
        )
        .into();
        let error = error.context("Invoke failed");

        let unavailable = model_unavailable(&error).expect("typed reason");
        assert_eq!(
            unavailable.reason,
            ModelUnavailableReason::DeviceLockedDeclined
        );
        assert_eq!(
            unavailable.message,
            "Model 'Qwen3 8B' runs on device 'GPU box', which stayed locked."
        );
        assert!(model_unavailable(&anyhow!("unrelated")).is_none());
        assert_eq!(
            json::to_value(ModelUnavailableReason::DeviceLockedDeclined).unwrap(),
            "device_locked_declined"
        );
    }

    #[test]
    fn endpoints_never_print_their_bearer_and_key_on_every_field() {
        let endpoint = endpoint("http://127.0.0.1:41234/v1/");
        assert!(!format!("{endpoint:?}").contains("loopback-token"));

        let key = endpoint.cache_key();
        assert!(key.starts_with("endpoint:http://127.0.0.1:41234/v1#qwen3-8b#"));
        assert!(!key.contains("loopback-token"));
        let rotated = ModelEndpoint {
            bearer: "rotated".to_string(),
            ..endpoint.clone()
        };
        assert_ne!(rotated.cache_key(), key);
        let other_model = ModelEndpoint {
            model: "other".to_string(),
            ..endpoint
        };
        assert_ne!(other_model.cache_key(), key);
    }

    #[test]
    fn device_provider_names_ignore_case_and_padding() {
        assert!(is_device_provider(" Device "));
        assert!(!is_device_provider("devices"));
    }
}

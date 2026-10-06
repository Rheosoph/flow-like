use std::{
    collections::{HashMap, HashSet},
    hash::{Hash, Hasher},
    sync::Arc,
};

use crate::{
    bit::{Bit, BitModelPreference, BitTypes},
    hub::{BitSearchQuery, Hub},
    models::device::DeviceModelProbe,
    state::CompletionModelCapabilities,
    utils::http::HTTPClient,
};
use flow_like_types::{Result, Value, anyhow, tokio::task};
use futures::future::join_all;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

mod home;
pub use home::validate_home_layout;

fn split_profile_bit_reference(reference: &str) -> Option<(&str, &str)> {
    let (hub, bit_id) = reference.rsplit_once(':')?;
    if hub.trim().is_empty() || bit_id.trim().is_empty() {
        return None;
    }

    Some((hub, bit_id))
}

/// What the host can run while a model is selected, and how candidates are probed.
struct ModelSelection {
    only_hosted: bool,
    capabilities: Option<CompletionModelCapabilities>,
    devices: Option<DeviceModelProbe>,
    unavailable_model: Option<String>,
}

impl ModelSelection {
    /// `bit` with its preference score when this host can run it.
    fn candidate(
        &self,
        bit: Bit,
        preference: &BitModelPreference,
        multimodal: bool,
    ) -> Option<(f32, Bit)> {
        if !self.admits(&bit) || (multimodal && !bit.is_multimodal()) {
            return None;
        }
        let score = bit.score(preference).ok()?;
        Some((score, bit))
    }

    /// Whether this host can run `bit` in-process or have its model host serve it.
    fn admits(&self, bit: &Bit) -> bool {
        let routed = self
            .devices
            .as_ref()
            .is_some_and(|devices| devices.may_route(bit));
        Profile::model_matches_host_filter(bit, self.only_hosted, self.capabilities, routed)
    }

    async fn is_available(&self, bit: &Bit) -> bool {
        Profile::model_available(bit, self.capabilities, self.devices.as_ref()).await
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Hash, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ConnectionMode {
    Default,
    Straight,
    Step,
    SmoothStep,
    SimpleBezier,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Hash, PartialEq, Eq)]
pub struct Settings {
    pub connection_mode: ConnectionMode,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            connection_mode: ConnectionMode::SimpleBezier,
        }
    }
}
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Hash, PartialEq, Eq)]
pub struct ProfileApp {
    pub app_id: String,
    pub favorite: bool,
    pub favorite_order: Option<i32>,
    pub pinned: bool,
    pub pinned_order: Option<i32>,
}

impl ProfileApp {
    pub fn new(app_id: String) -> Self {
        Self {
            app_id,
            favorite: false,
            favorite_order: None,
            pinned: false,
            pinned_order: None,
        }
    }
}

fn default_secure() -> bool {
    true
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Hash, PartialEq, Eq)]
pub struct ProfileShortcut {
    pub id: String,
    #[serde(rename = "profileId")]
    pub profile_id: String,
    pub label: String,
    pub path: String,
    #[serde(rename = "appId")]
    pub app_id: Option<String>,
    pub icon: Option<String>,
    pub order: i32,
    #[serde(rename = "createdAt")]
    pub created_at: String,
}

/// A user-owned private bit carried inline on the profile (custom provider
/// configs or private HuggingFace models). `Bit` cannot derive `Hash`/`Eq`
/// (untyped `parameters`), so equality and hashing go through the canonical
/// JSON serialization.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Default)]
#[serde(transparent)]
pub struct ProfileCustomBit(pub Bit);

impl ProfileCustomBit {
    fn canonical(&self) -> String {
        flow_like_types::json::to_string(&self.0).unwrap_or_default()
    }
}

impl PartialEq for ProfileCustomBit {
    fn eq(&self, other: &Self) -> bool {
        self.canonical() == other.canonical()
    }
}

impl Eq for ProfileCustomBit {}

impl Hash for ProfileCustomBit {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.canonical().hash(state);
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Hash, PartialEq, Eq)]
pub struct Profile {
    #[serde(default = "flow_like_types::create_id")]
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    pub thumbnail: Option<String>,
    #[serde(default)]
    pub interests: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub hub: String,
    #[serde(default = "default_secure")]
    pub secure: bool,
    #[serde(default)]
    pub hubs: Vec<String>,
    #[serde(default)]
    pub apps: Option<Vec<ProfileApp>>,
    #[serde(default)]
    pub shortcuts: Option<Vec<ProfileShortcut>>,
    #[serde(default)]
    pub home_layout: Option<Value>,
    #[serde(default)]
    pub home_default_id: Option<String>,
    #[serde(default)]
    pub theme: Option<Value>,
    pub bits: Vec<String>, // hub:id
    /// User-owned private bits, hydrated per trust boundary: with decrypted
    /// provider secrets only server-side per request/run and on the owner's
    /// desktop; never in the browser client or the server profile row.
    /// Schema-wise these are plain `Bit`s (the wrapper is serde-transparent);
    /// `schemars(with)` keeps the generated schema referencing `Bit` instead
    /// of minting a duplicate inline type, which would cascade renames through
    /// the quicktype-generated TS.
    #[serde(default)]
    #[schemars(with = "Vec<Bit>")]
    pub custom_bits: Vec<ProfileCustomBit>,
    #[serde(default)]
    pub settings: Settings,
    pub updated: String,
    pub created: String,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            id: flow_like_types::create_id(),
            name: "".to_string(),
            description: Some("".to_string()),
            thumbnail: Some("".to_string()),
            hub: "".to_string(),
            secure: true,
            hubs: vec![],
            bits: vec![],
            custom_bits: vec![],
            icon: Some("".to_string()),
            interests: vec![],
            tags: vec![],
            apps: Some(vec![]),
            shortcuts: Some(vec![]),
            home_layout: None,
            home_default_id: None,
            theme: None,
            settings: Settings {
                connection_mode: ConnectionMode::SimpleBezier,
            },
            updated: "".to_string(),
            created: "".to_string(),
        }
    }
}

impl Profile {
    fn is_self_hosted_provider_name(provider_name: &str) -> bool {
        matches!(
            provider_name.trim().to_ascii_lowercase().as_str(),
            "local"
                | "local:any-tts"
                | "llama.cpp"
                | "llamacpp"
                | "ollama"
                | "custom:ollama"
                | "lmstudio"
                | "custom:lmstudio"
                | "mlx"
                | crate::models::device::DEVICE_PROVIDER_NAME
        )
    }

    /// Check if a bit is a local model (requires local hosting capabilities)
    fn is_local_model(bit: &Bit) -> bool {
        if let Some(parameters) = bit.try_to_systemone() {
            return Self::is_self_hosted_provider_name(&parameters.provider.provider_name);
        }
        if bit.try_to_provider().is_some_and(|provider| {
            flow_like_model_provider::llm::external::ExternalProvider::from_provider_name(
                &provider.provider_name,
            )
            .is_some_and(|kind| kind.requires_local_credentials(&provider))
        }) {
            return true;
        }
        if bit.bit_type == crate::bit::BitTypes::Tts {
            return true;
        }

        if let Ok(llm_params) =
            flow_like_types::json::from_value::<crate::bit::LLMParameters>(bit.parameters.clone())
        {
            if Self::is_self_hosted_provider_name(&llm_params.provider.provider_name) {
                return true;
            }
        } else if let Ok(vlm_params) =
            flow_like_types::json::from_value::<crate::bit::VLMParameters>(bit.parameters.clone())
            && Self::is_self_hosted_provider_name(&vlm_params.provider.provider_name)
        {
            return true;
        }

        false
    }

    fn can_execute_completion_model(bit: &Bit, capabilities: CompletionModelCapabilities) -> bool {
        if bit.try_to_provider().is_some_and(|provider| {
            flow_like_model_provider::llm::external::ExternalProvider::from_provider_name(
                &provider.provider_name,
            )
            .is_some_and(|kind| kind.requires_local_credentials(&provider))
        }) {
            return capabilities.local_credentials;
        }
        if bit.is_mlx_model() {
            return capabilities.mlx;
        }
        if bit.is_device_model() {
            return capabilities.device_models;
        }

        let requires_local_server = bit
            .try_to_provider()
            .is_some_and(|provider| provider.provider_name.trim().eq_ignore_ascii_case("local"));
        !requires_local_server || capabilities.local_server
    }

    /// `routed`: the device's model host may serve `bit`, so it needs no runtime in this process.
    fn model_matches_host_filter(
        bit: &Bit,
        only_hosted: bool,
        capabilities: Option<CompletionModelCapabilities>,
        routed: bool,
    ) -> bool {
        if only_hosted && Self::is_local_model(bit) {
            return false;
        }
        routed
            || capabilities
                .is_none_or(|capabilities| Self::can_execute_completion_model(bit, capabilities))
    }

    async fn external_model_available(
        bit: &Bit,
        capabilities: Option<CompletionModelCapabilities>,
    ) -> bool {
        let Some(provider) = bit.try_to_provider() else {
            return true;
        };
        if flow_like_model_provider::llm::external::ExternalProvider::from_provider_name(
            &provider.provider_name,
        )
        .is_none()
        {
            return true;
        }
        let available = flow_like_model_provider::llm::external::check_available(
            &provider,
            capabilities.is_some_and(|caps| caps.local_credentials),
        )
        .await;
        if available.is_err() {
            // Do not log provider parameters or upstream bodies: they can contain credentials.
            tracing::debug!(bit_id = %bit.id, "Skipping unavailable external model");
        }
        available.is_ok()
    }

    /// Bits the device's model host serves are available when its router answers them, and device
    /// models when the probe reaches them, which may prompt the user to unlock the device. Without
    /// a probe device models are unreachable, and a Bit the router does not serve needs its runtime
    /// in this process.
    async fn model_available(
        bit: &Bit,
        capabilities: Option<CompletionModelCapabilities>,
        devices: Option<&DeviceModelProbe>,
    ) -> bool {
        if let Some(devices) = devices
            && let Some(available) = devices.availability(bit).await
        {
            return available;
        }
        if bit.is_device_model() {
            return false;
        }
        capabilities
            .is_none_or(|capabilities| Self::can_execute_completion_model(bit, capabilities))
            && Self::external_model_available(bit, capabilities).await
    }

    /// Personal providers and device models can be unavailable at run time; a saved preference
    /// for one falls back instead of failing.
    fn falls_back_when_unavailable(bit: &Bit) -> bool {
        bit.is_device_model()
            || bit.try_to_provider().is_some_and(|provider| {
                flow_like_model_provider::llm::external::ExternalProvider::from_provider_name(
                    &provider.provider_name,
                )
                .is_some()
            })
    }

    /// Gets the best model based on the preference
    /// `remote = true` skips this profile's own bits entirely and scores the whole
    /// hub catalog instead — recommendation/discovery only. Anything that runs on
    /// the user's behalf must pass `false`, or it can land on a model their plan
    /// does not include.
    /// When only_hosted=true, filters out local models that require hosting capabilities
    pub async fn get_best_model(
        &self,
        preference: &BitModelPreference,
        multimodal: bool,
        remote: bool,
        http_client: Arc<HTTPClient>,
    ) -> Result<Bit> {
        self.get_best_model_filtered(preference, multimodal, remote, false, http_client)
            .await
    }

    /// Resolve an explicitly selected model or choose the best profile model
    /// that this host can execute.
    ///
    /// `capabilities` applies to both paths. This matters for explicit model
    /// IDs, which otherwise bypass automatic filtering and can reach an
    /// unsupported local runtime. `devices` probes device candidates and, on a
    /// device, the Bits its model host serves; any unavailable one is skipped
    /// for the next candidate.
    pub async fn resolve_completion_model(
        &self,
        model_id: Option<&str>,
        preference: &BitModelPreference,
        multimodal: bool,
        capabilities: CompletionModelCapabilities,
        devices: Option<&DeviceModelProbe>,
        http_client: Arc<HTTPClient>,
    ) -> Result<Bit> {
        let mut selection = ModelSelection {
            only_hosted: false,
            capabilities: Some(capabilities),
            devices: devices.map(DeviceModelProbe::for_selection),
            unavailable_model: None,
        };
        if let Some(model_id) = model_id {
            let bit = self.find_bit(model_id, http_client.clone()).await?;
            if Self::falls_back_when_unavailable(&bit) {
                if selection.admits(&bit) && selection.is_available(&bit).await {
                    return Ok(bit);
                }
                // An unavailable personal provider must not strand a saved use-case preference.
                tracing::debug!(bit_id = %bit.id, "Falling back from unavailable model");
                selection.unavailable_model = Some(bit.id);
            } else if !selection.admits(&bit) {
                return Err(anyhow!(
                    "Model {model_id} requires a local completion runtime that this host cannot execute"
                ));
            } else {
                return Ok(bit);
            }
        }

        self.get_best_model_filtered_inner(preference, multimodal, false, &selection, http_client)
            .await
    }

    /// Select the first available SystemOne Bit in this profile's activated model order.
    pub async fn find_decision_model(
        &self,
        capabilities: CompletionModelCapabilities,
        devices: Option<&DeviceModelProbe>,
        http_client: Arc<HTTPClient>,
    ) -> Result<Bit> {
        let selection = ModelSelection {
            only_hosted: false,
            capabilities: Some(capabilities),
            devices: devices.map(DeviceModelProbe::for_selection),
            unavailable_model: None,
        };
        let mut seen = HashSet::new();
        for bit_ref in &self.bits {
            let bit = match self.get_profile_bit(bit_ref, http_client.clone()).await {
                Ok(bit) => bit,
                Err(error) => {
                    tracing::warn!(bit = %bit_ref, %error, "Skipping unresolved profile bit");
                    continue;
                }
            };
            let Some(parameters) = bit.try_to_systemone() else {
                continue;
            };
            if !seen.insert(bit.id.clone())
                || parameters.context_length == 0
                || !selection.admits(&bit)
            {
                continue;
            }
            let available = if crate::models::systemone::supports_provider(
                &parameters.provider.provider_name,
            ) {
                selection.is_available(&bit).await
            } else if let Some(devices) = &selection.devices {
                // Device endpoints are resolved before the factory's native provider dispatch.
                devices.availability(&bit).await == Some(true)
            } else {
                false
            };
            if available {
                return Ok(bit);
            }
        }
        Err(anyhow!(
            "No available SystemOne decision model found in this profile"
        ))
    }

    /// Create a copy of this profile with only hosted models (filters out local models)
    /// This is useful for cloud deployments where local models cannot be hosted
    pub fn filter_hosted_only(&self) -> Self {
        let mut filtered = self.clone();
        filtered.bits.retain(|_bit_ref| {
            // We can't check the actual bit without fetching it from the hub,
            // so we filter based on known patterns in the bit reference
            // Desktop app will use the full profile; cloud will use filtered
            true // Keep all for now - actual filtering happens in get_best_model_filtered
        });
        filtered
    }

    /// Gets the best model based on the preference with filtering options
    /// When only_hosted=true, filters out local models that require hosting capabilities
    pub async fn get_best_model_filtered(
        &self,
        preference: &BitModelPreference,
        multimodal: bool,
        remote: bool,
        only_hosted: bool,
        http_client: Arc<HTTPClient>,
    ) -> Result<Bit> {
        let selection = ModelSelection {
            only_hosted,
            capabilities: None,
            devices: None,
            unavailable_model: None,
        };
        self.get_best_model_filtered_inner(preference, multimodal, remote, &selection, http_client)
            .await
    }

    async fn get_best_model_filtered_inner(
        &self,
        preference: &BitModelPreference,
        multimodal: bool,
        remote: bool,
        selection: &ModelSelection,
        http_client: Arc<HTTPClient>,
    ) -> Result<Bit> {
        let multimodal = multimodal || preference.multimodal.unwrap_or(false);
        let mut candidates: Vec<(f32, Bit)> = self
            .activated_custom_bits()
            .into_iter()
            .filter_map(|bit| selection.candidate(bit.clone(), preference, multimodal))
            .collect();

        if !remote {
            for bit_ref in &self.bits {
                let bit = match self.get_profile_bit(bit_ref, http_client.clone()).await {
                    Ok(bit) => bit,
                    Err(err) => {
                        tracing::warn!(
                            bit = %bit_ref,
                            error = %err,
                            "Skipping unresolved profile bit"
                        );
                        continue;
                    }
                };
                candidates.extend(selection.candidate(bit, preference, multimodal));
            }

            return Self::select_available_model(candidates, selection).await;
        }

        let preference = preference.parse();
        let hub_models = self.hub_completion_models(http_client).await?;
        candidates.extend(
            hub_models
                .into_iter()
                .filter_map(|bit| selection.candidate(bit, &preference, multimodal)),
        );
        Self::select_available_model(candidates, selection).await
    }

    /// Every LLM and VLM the profile's hubs list; a hub that fails to answer is skipped.
    async fn hub_completion_models(&self, http_client: Arc<HTTPClient>) -> Result<Vec<Bit>> {
        let query = BitSearchQuery::builder()
            .with_bit_types(vec![BitTypes::Vlm, BitTypes::Llm])
            .build();
        let mut bits: HashMap<String, Bit> = HashMap::new();
        for hub in self.get_available_hubs(http_client).await? {
            if let Ok(models) = hub.search_bit(&query).await {
                bits.extend(models.into_iter().map(|bit| (bit.id.clone(), bit)));
            }
        }
        Ok(bits.into_values().collect())
    }

    async fn select_available_model(
        mut candidates: Vec<(f32, Bit)>,
        selection: &ModelSelection,
    ) -> Result<Bit> {
        // Stable sorting preserves profile order for equal scores. Probe only
        // candidates that can win, and never probe a hydrated Bit twice.
        candidates.sort_by(|a, b| b.0.total_cmp(&a.0));
        let mut seen: HashSet<String> = selection.unavailable_model.iter().cloned().collect();
        for (_, bit) in candidates {
            if seen.insert(bit.id.clone()) && selection.is_available(&bit).await {
                return Ok(bit);
            }
        }
        Err(anyhow!("No available model found in this profile"))
    }

    /// Looks up a user-owned custom bit carried on this profile by id. Resolves
    /// against everything hydrated into `custom_bits`, activated or not: picking
    /// a model explicitly is the activation.
    pub fn custom_bit(&self, bit_id: &str) -> Option<Bit> {
        self.custom_bits
            .iter()
            .map(|custom| &custom.0)
            .find(|bit| bit.id == bit_id)
            .cloned()
    }

    /// The custom bits this profile activated through its `bits` references.
    /// Hosts may hydrate `custom_bits` with the user's whole library so an
    /// explicitly selected model always resolves; discovery — best-model
    /// scoring and search — stays scoped to the profile's own line-up.
    fn activated_custom_bits(&self) -> Vec<&Bit> {
        let activated: HashSet<&str> = self
            .bits
            .iter()
            .map(|reference| {
                reference
                    .rsplit_once(':')
                    .map_or(reference.as_str(), |(_, id)| id)
            })
            .collect();

        self.custom_bits
            .iter()
            .map(|custom| &custom.0)
            .filter(|bit| activated.contains(bit.id.as_str()))
            .collect()
    }

    fn custom_bits_matching(&self, query: &BitSearchQuery) -> Vec<Bit> {
        self.activated_custom_bits()
            .into_iter()
            .filter(|bit| {
                query
                    .bit_types
                    .as_ref()
                    .is_none_or(|types| types.contains(&bit.bit_type))
            })
            .filter(|bit| {
                query.search.as_ref().is_none_or(|search| {
                    let search = search.to_lowercase();
                    bit.meta.values().any(|meta| {
                        meta.name.to_lowercase().contains(&search)
                            || meta.description.to_lowercase().contains(&search)
                    })
                })
            })
            .cloned()
            .collect()
    }

    pub async fn search_bits(
        &self,
        query: &BitSearchQuery,
        http_client: Arc<HTTPClient>,
    ) -> Result<Vec<Bit>> {
        let hubs = self.get_available_hubs(http_client).await?;
        let mut bits: HashMap<String, Bit> = HashMap::new();
        for bit in self.custom_bits_matching(query) {
            bits.insert(bit.id.clone(), bit);
        }
        for hub in hubs {
            let hub_bits = hub.search_bit(query).await;
            let hub_bits = match hub_bits {
                Ok(models) => models,
                Err(err) => {
                    tracing::warn!(error = %err, "Bit could not be queried");
                    continue;
                }
            };
            for bit in hub_bits {
                if !bits.contains_key(&bit.id) {
                    bits.insert(bit.id.clone(), bit.clone());
                }
            }
        }
        let bits = bits.into_values().collect();
        Ok(bits)
    }

    pub async fn get_bit(
        &self,
        bit: String,
        hub: Option<String>,
        http_client: Arc<HTTPClient>,
    ) -> Result<Bit> {
        if let Some(custom) = self.custom_bit(&bit) {
            return Ok(custom);
        }

        if let Some(hub) = hub {
            let hub = Hub::new(&hub, http_client).await?;
            let bit = hub.get_bit(&bit).await?;
            return Ok(bit);
        }

        let hubs = self.get_available_hubs(http_client).await?;
        for hub in hubs {
            let found = hub.get_bit(&bit).await;
            if let Ok(found) = found {
                return Ok(found);
            }
        }
        Err(flow_like_types::anyhow!(
            "Bit not found: {bit} (not in profile {} or any of its hubs)",
            self.id
        ))
    }

    pub async fn find_bit(&self, bit_id: &str, http_client: Arc<HTTPClient>) -> Result<Bit> {
        if let Some(custom) = self.custom_bit(bit_id) {
            return Ok(custom);
        }

        let hubs = self.get_available_hubs(http_client).await?;
        for hub in hubs {
            let bit = hub.get_bit(bit_id).await;
            if let Ok(bit) = bit {
                return Ok(bit);
            }
        }
        Err(flow_like_types::anyhow!(
            "Bit not found: {bit_id} (not in profile {} or any of its hubs)",
            self.id
        ))
    }

    /// Resolve a model selected from this profile without accepting a new catalog source.
    pub async fn resolve_model_reference(
        &self,
        reference: &str,
        http_client: Arc<HTTPClient>,
    ) -> Result<Bit> {
        let (hub, id) = split_profile_bit_reference(reference)
            .map_or((None, reference), |(hub, id)| (Some(hub), id));
        if let Some(bit) = self.custom_bit(id) {
            if hub.is_some_and(|hub| hub != bit.hub) {
                return Err(anyhow!("Model reference changes its configured hub"));
            }
            return Ok(bit);
        }
        let selected = self.bits.iter().find(|selected| {
            let (selected_hub, selected_id) = split_profile_bit_reference(selected)
                .map_or((None, selected.as_str()), |(hub, id)| (Some(hub), id));
            selected_id == id && hub.is_none_or(|hub| selected_hub == Some(hub))
        });
        let selected =
            selected.ok_or_else(|| anyhow!("Add this model to the profile before using it"))?;
        self.get_profile_bit(selected, http_client).await
    }

    async fn get_profile_bit(&self, bit_ref: &str, http_client: Arc<HTTPClient>) -> Result<Bit> {
        if bit_ref.trim().is_empty() {
            return Err(anyhow!("Invalid bit format: {}", bit_ref));
        }

        if let Some((hub, bit_id)) = split_profile_bit_reference(bit_ref) {
            if let Some(custom) = self.custom_bit(bit_id) {
                return Ok(custom);
            }
            let hub = Hub::new(hub, http_client).await?;
            return hub.get_bit(bit_id).await;
        }

        self.find_bit(bit_ref, http_client).await
    }

    pub async fn get_available_hubs(&self, http_client: Arc<HTTPClient>) -> Result<Vec<Hub>> {
        let mut hubs = HashSet::new();
        if !self.hub.trim().is_empty() {
            hubs.insert(self.hub.clone());
        }

        for hub in &self.hubs {
            if !hub.trim().is_empty() {
                hubs.insert(hub.clone());
            }
        }

        self.bits.iter().for_each(|id| {
            if let Some((hub, _bit_id)) = split_profile_bit_reference(id) {
                hubs.insert(hub.to_string());
            }
        });

        let hub_futures: Vec<_> = hubs
            .iter()
            .map(|hub| {
                let hub = hub.clone();
                let http_client = http_client.clone();
                task::spawn(async move { Hub::new(&hub, http_client).await })
            })
            .collect();

        let results = join_all(hub_futures).await;
        let built_hubs = results
            .into_iter()
            .filter_map(|f| f.ok())
            .flatten()
            .collect();

        Ok(built_hubs)
    }

    pub async fn add_bit(&mut self, bit: &Bit) {
        let bit_id = format!("{}:{}", bit.hub, bit.id);
        let bit_exists = self
            .bits
            .iter()
            .any(|reference| reference.split(':').next_back() == Some(bit.id.as_str()));
        if bit_exists {
            return;
        }
        self.bits.push(bit_id);
    }

    pub fn remove_bit(&mut self, bit: &Bit) {
        self.bits
            .retain(|reference| reference.split(':').next_back() != Some(bit.id.as_str()));
    }
}

#[cfg(test)]
mod tests {
    use super::{Profile, ProfileCustomBit, split_profile_bit_reference};
    use crate::{
        bit::{
            Bit, BitModelClassification, BitModelPreference, BitTypes, LLMParameters, VLMParameters,
        },
        state::CompletionModelCapabilities,
        utils::http::HTTPClient,
    };
    use flow_like_model_provider::provider::ModelProvider;
    use flow_like_types::tokio;
    use std::sync::Arc;

    fn completion_bit(id: &str, provider_name: &str) -> Bit {
        Bit {
            id: id.to_string(),
            bit_type: BitTypes::Llm,
            parameters: flow_like_types::json::to_value(LLMParameters {
                context_length: 20_000,
                model_classification: BitModelClassification::default(),
                provider: ModelProvider {
                    api_surface: None,
                    provider_name: provider_name.to_string(),
                    model_id: Some(id.to_string()),
                    version: None,
                    params: None,
                },
            })
            .unwrap(),
            ..Bit::default()
        }
    }

    fn profile_with_models(models: Vec<Bit>) -> Profile {
        Profile {
            bits: models.iter().map(|bit| bit.id.clone()).collect(),
            custom_bits: models.into_iter().map(ProfileCustomBit).collect(),
            ..Profile::default()
        }
    }

    fn decision_bit(id: &str, provider_name: &str) -> Bit {
        let mut bit = completion_bit(id, provider_name);
        bit.bit_type = BitTypes::SystemOne;
        bit
    }

    #[tokio::test]
    async fn decision_model_selection_uses_active_profile_order_and_valid_decision_bits() {
        let mut malformed = decision_bit("malformed", "hosted:systemone_compatible");
        malformed.parameters["context_length"] = flow_like_types::json::json!(0);
        let mut profile = profile_with_models(vec![
            decision_bit("inactive", "hosted:systemone_compatible"),
            decision_bit("second", "hosted:systemone_compatible"),
            decision_bit("first", "hosted:systemone_compatible"),
            completion_bit("chat", "hosted:openai"),
            malformed,
        ]);
        profile.bits = vec![
            "".into(),
            "chat".into(),
            "malformed".into(),
            "first".into(),
            "second".into(),
        ];

        let selected = profile
            .find_decision_model(
                CompletionModelCapabilities::default(),
                None,
                Arc::new(HTTPClient::new_without_refetch()),
            )
            .await
            .unwrap();

        assert_eq!(selected.id, "first");
    }

    #[tokio::test]
    async fn decision_model_selection_respects_local_runtime_capabilities() {
        let profile = profile_with_models(vec![
            decision_bit("local", "Local"),
            decision_bit("hosted", "hosted:systemone_compatible"),
        ]);
        for (local_server, expected) in [(false, "hosted"), (true, "local")] {
            let selected = profile
                .find_decision_model(
                    CompletionModelCapabilities {
                        local_server,
                        ..CompletionModelCapabilities::default()
                    },
                    None,
                    Arc::new(HTTPClient::new_without_refetch()),
                )
                .await
                .unwrap();
            assert_eq!(selected.id, expected);
        }
    }

    #[tokio::test]
    async fn decision_model_selection_skips_unsupported_systemone_providers() {
        let profile = profile_with_models(vec![
            decision_bit("unsupported-hosted", "hosted:openai"),
            decision_bit("unsupported-custom", "custom:ollama"),
            decision_bit("unsupported-local", "MLX"),
            decision_bit("supported", "hosted:systemone_compatible"),
        ]);
        let selected = profile
            .find_decision_model(
                CompletionModelCapabilities {
                    mlx: true,
                    ..CompletionModelCapabilities::default()
                },
                None,
                Arc::new(HTTPClient::new_without_refetch()),
            )
            .await
            .unwrap();

        assert_eq!(selected.id, "supported");
    }

    #[tokio::test]
    async fn decision_model_selection_does_not_fall_back_to_chat_or_inactive_models() {
        let mut profile = profile_with_models(vec![
            completion_bit("chat", "hosted:openai"),
            decision_bit("inactive", "hosted:systemone_compatible"),
        ]);
        profile.bits = vec!["chat".into()];

        let error = profile
            .find_decision_model(
                CompletionModelCapabilities::default(),
                None,
                Arc::new(HTTPClient::new_without_refetch()),
            )
            .await
            .unwrap_err();

        assert_eq!(
            error.to_string(),
            "No available SystemOne decision model found in this profile"
        );
    }

    #[tokio::test]
    async fn systemone_model_references_keep_profile_credentials_and_pinned_hubs() {
        let mut bit = completion_bit("decision", "custom:systemone");
        bit.bit_type = BitTypes::SystemOne;
        bit.hub = "https://api.flow-like.com".into();
        bit.parameters["provider"]["params"] =
            flow_like_types::json::json!({"api_key":"profile-key"});
        let profile = profile_with_models(vec![bit.clone()]);
        let http = Arc::new(HTTPClient::new_without_refetch());
        for reference in ["decision", "https://api.flow-like.com:decision"] {
            let resolved = profile
                .resolve_model_reference(reference, http.clone())
                .await
                .unwrap();
            assert_eq!(
                resolved.parameters["provider"]["params"]["api_key"],
                "profile-key"
            );
        }
        for reference in [
            "https://unselected.invalid:decision",
            "https://unselected.invalid:unknown",
        ] {
            assert!(
                profile
                    .resolve_model_reference(reference, http.clone())
                    .await
                    .is_err()
            );
        }
    }

    #[tokio::test]
    async fn unavailable_external_models_fall_back_for_discovery_and_explicit_preferences() {
        for provider in [
            "custom:claude-code",
            "custom:codex",
            "custom:github-copilot",
            "custom:microsoft-copilot",
        ] {
            let profile = profile_with_models(vec![
                completion_bit("personal", provider),
                completion_bit("fallback", "hosted:openai"),
            ]);
            for requested in [None, Some("personal")] {
                let selected = profile
                    .resolve_completion_model(
                        requested,
                        &BitModelPreference::default(),
                        false,
                        CompletionModelCapabilities::default(),
                        None,
                        Arc::new(HTTPClient::new_without_refetch()),
                    )
                    .await
                    .unwrap();
                assert_eq!(selected.id, "fallback", "provider {provider}");
            }
        }
    }

    #[tokio::test]
    async fn unavailable_external_models_do_not_activate_other_library_models() {
        let mut profile = profile_with_models(vec![
            completion_bit("personal", "custom:claude-code"),
            completion_bit("inactive", "hosted:openai"),
        ]);
        profile.bits = vec!["personal".into()];
        assert!(
            profile
                .resolve_completion_model(
                    None,
                    &BitModelPreference::default(),
                    false,
                    CompletionModelCapabilities::default(),
                    None,
                    Arc::new(HTTPClient::new_without_refetch()),
                )
                .await
                .is_err()
        );
    }

    #[test]
    fn local_account_providers_are_filtered_from_hosted_execution() {
        assert!(Profile::is_local_model(&completion_bit(
            "claude",
            "custom:claude-code"
        )));
        assert!(Profile::is_local_model(&completion_bit(
            "codex",
            "custom:codex"
        )));
        assert!(!Profile::is_local_model(&completion_bit(
            "microsoft",
            "custom:microsoft-copilot"
        )));
    }

    #[test]
    fn split_profile_bit_reference_handles_hub_urls() {
        assert_eq!(
            split_profile_bit_reference("https://api.flow-like.com:s14lujkm2gut2mwg0zo3imxv"),
            Some(("https://api.flow-like.com", "s14lujkm2gut2mwg0zo3imxv"))
        );
        assert_eq!(
            split_profile_bit_reference("api.flow-like.com:s14lujkm2gut2mwg0zo3imxv"),
            Some(("api.flow-like.com", "s14lujkm2gut2mwg0zo3imxv"))
        );
    }

    #[test]
    fn split_profile_bit_reference_allows_bare_bit_ids() {
        assert_eq!(
            split_profile_bit_reference("s14lujkm2gut2mwg0zo3imxv"),
            None
        );
    }

    #[tokio::test]
    async fn profile_add_bit_deduplicates_bare_and_hub_references() {
        let mut profile = Profile {
            bits: vec!["s14lujkm2gut2mwg0zo3imxv".to_string()],
            ..Profile::default()
        };
        let bit = crate::bit::Bit {
            id: "s14lujkm2gut2mwg0zo3imxv".to_string(),
            hub: "https://api.flow-like.com".to_string(),
            ..crate::bit::Bit::default()
        };
        profile.add_bit(&bit).await;

        assert_eq!(bit.id, "s14lujkm2gut2mwg0zo3imxv");
        assert_eq!(profile.bits, vec!["s14lujkm2gut2mwg0zo3imxv"]);
    }

    #[tokio::test]
    async fn best_model_skips_missing_profile_bits() {
        let profile = Profile {
            bits: vec!["missing-bit".to_string()],
            ..Profile::default()
        };
        let http_client = Arc::new(HTTPClient::new_without_refetch());

        let err = profile
            .get_best_model_filtered(
                &BitModelPreference::default(),
                false,
                false,
                false,
                http_client,
            )
            .await
            .unwrap_err();

        assert_eq!(err.to_string(), "No available model found in this profile");
    }

    #[tokio::test]
    async fn completion_model_selection_skips_local_models_for_hosted_only_hosts() {
        let profile = profile_with_models(vec![
            completion_bit("local-model", "Local"),
            completion_bit("hosted-model", "hosted:openai"),
        ]);

        let selected = profile
            .resolve_completion_model(
                None,
                &BitModelPreference::default(),
                false,
                CompletionModelCapabilities::default(),
                None,
                Arc::new(HTTPClient::new_without_refetch()),
            )
            .await
            .unwrap();

        assert_eq!(selected.id, "hosted-model");
    }

    #[tokio::test]
    async fn completion_model_selection_preserves_local_models_for_capable_hosts() {
        let profile = profile_with_models(vec![completion_bit("local-model", "Local")]);

        let selected = profile
            .resolve_completion_model(
                None,
                &BitModelPreference::default(),
                false,
                CompletionModelCapabilities {
                    local_server: true,
                    mlx: false,
                    local_credentials: false,
                    device_models: false,
                },
                None,
                Arc::new(HTTPClient::new_without_refetch()),
            )
            .await
            .unwrap();

        assert_eq!(selected.id, "local-model");
    }

    #[tokio::test]
    async fn explicit_embedded_completion_model_is_rejected_on_hosted_only_hosts() {
        let profile = profile_with_models(vec![
            completion_bit("local-model", "Local"),
            completion_bit("hosted-model", "hosted:openai"),
        ]);
        let http_client = Arc::new(HTTPClient::new_without_refetch());

        let error = profile
            .resolve_completion_model(
                Some("local-model"),
                &BitModelPreference::default(),
                false,
                CompletionModelCapabilities::default(),
                None,
                http_client.clone(),
            )
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("requires a local completion runtime")
        );

        let selected = profile
            .resolve_completion_model(
                Some("hosted-model"),
                &BitModelPreference::default(),
                false,
                CompletionModelCapabilities::default(),
                None,
                http_client,
            )
            .await
            .unwrap();
        assert_eq!(selected.id, "hosted-model");
    }

    #[tokio::test]
    async fn local_only_profile_returns_error_when_host_cannot_execute_it() {
        let profile = profile_with_models(vec![completion_bit("local-model", "Local")]);

        let error = profile
            .resolve_completion_model(
                None,
                &BitModelPreference::default(),
                false,
                CompletionModelCapabilities::default(),
                None,
                Arc::new(HTTPClient::new_without_refetch()),
            )
            .await
            .unwrap_err();

        assert_eq!(
            error.to_string(),
            "No available model found in this profile"
        );
    }

    #[tokio::test]
    async fn endpoint_backed_models_do_not_require_embedded_runtime_capabilities() {
        for provider_name in ["custom:ollama", "custom:lmstudio"] {
            let bit = completion_bit(provider_name, provider_name);
            let profile = profile_with_models(vec![bit]);

            let selected = profile
                .resolve_completion_model(
                    Some(provider_name),
                    &BitModelPreference::default(),
                    false,
                    CompletionModelCapabilities::default(),
                    None,
                    Arc::new(HTTPClient::new_without_refetch()),
                )
                .await
                .unwrap();

            assert_eq!(selected.id, provider_name);
        }
    }

    #[tokio::test]
    async fn mobile_capabilities_skip_llama_server_but_allow_mlx() {
        let profile = profile_with_models(vec![
            completion_bit("local-model", "Local"),
            completion_bit("mlx-model", "MLX"),
            completion_bit("hosted-model", "hosted:openai"),
        ]);

        let selected = profile
            .resolve_completion_model(
                None,
                &BitModelPreference::default(),
                false,
                CompletionModelCapabilities {
                    local_server: false,
                    mlx: true,
                    local_credentials: false,
                    device_models: false,
                },
                None,
                Arc::new(HTTPClient::new_without_refetch()),
            )
            .await
            .unwrap();

        assert_eq!(selected.id, "mlx-model");
    }

    #[tokio::test]
    async fn unsupported_mlx_is_skipped_when_an_executable_model_exists() {
        let profile = profile_with_models(vec![
            completion_bit("mlx-model", "MLX"),
            completion_bit("hosted-model", "hosted:openai"),
        ]);

        let selected = profile
            .resolve_completion_model(
                None,
                &BitModelPreference::default(),
                false,
                CompletionModelCapabilities {
                    local_server: true,
                    mlx: false,
                    local_credentials: false,
                    device_models: false,
                },
                None,
                Arc::new(HTTPClient::new_without_refetch()),
            )
            .await
            .unwrap();

        assert_eq!(selected.id, "hosted-model");
    }

    #[test]
    fn custom_bits_resolve_by_id_but_stay_profile_scoped_for_discovery() {
        let custom_bit = |id: &str| {
            super::ProfileCustomBit(crate::bit::Bit {
                id: id.to_string(),
                bit_type: BitTypes::Llm,
                ..crate::bit::Bit::default()
            })
        };

        let profile = Profile {
            bits: vec!["https://api.flow-like.com:activated".to_string()],
            custom_bits: vec![custom_bit("activated"), custom_bit("library-only")],
            ..Profile::default()
        };

        assert!(profile.custom_bit("activated").is_some());
        assert!(
            profile.custom_bit("library-only").is_some(),
            "an explicitly selected library model must resolve"
        );

        let activated: Vec<&str> = profile
            .activated_custom_bits()
            .iter()
            .map(|bit| bit.id.as_str())
            .collect();
        assert_eq!(activated, vec!["activated"]);
    }

    #[test]
    fn local_model_detection_includes_custom_local_providers() {
        for provider_name in ["custom:ollama", "custom:lmstudio"] {
            let bit = crate::bit::Bit {
                bit_type: BitTypes::Vlm,
                parameters: flow_like_types::json::to_value(VLMParameters {
                    context_length: 20000,
                    model_classification: BitModelClassification::default(),
                    provider: ModelProvider {
                        api_surface: None,
                        provider_name: provider_name.to_string(),
                        model_id: Some("local-model".to_string()),
                        version: None,
                        params: None,
                    },
                })
                .unwrap(),
                ..crate::bit::Bit::default()
            };

            assert!(
                Profile::is_local_model(&bit),
                "{provider_name} should be treated as local-only"
            );
        }
    }

    mod device_models {
        use super::*;
        use crate::models::device::{
            DeviceModelProbe, Interaction, ModelUnavailableReason,
            testing::{FakeConnector, FakeRouter, device_llm_bit, endpoint},
        };

        fn profile() -> Profile {
            profile_with_models(vec![
                device_llm_bit("device-model", "dev-1"),
                completion_bit("fallback", "hosted:openai"),
            ])
        }

        fn with_device_models() -> CompletionModelCapabilities {
            CompletionModelCapabilities {
                device_models: true,
                ..CompletionModelCapabilities::default()
            }
        }

        #[tokio::test]
        async fn decision_model_selection_probes_devices_and_falls_back_when_unavailable() {
            let mut device = device_llm_bit("device-decision", "dev-1");
            device.bit_type = BitTypes::SystemOne;
            let profile = profile_with_models(vec![
                device,
                decision_bit("hosted", "hosted:systemone_compatible"),
            ]);
            for (connector, expected) in [
                (
                    FakeConnector::serving(endpoint("http://127.0.0.1:9/v1")),
                    "device-decision",
                ),
                (
                    FakeConnector::failing(ModelUnavailableReason::DeviceLockedDeclined),
                    "hosted",
                ),
            ] {
                let probe = DeviceModelProbe::new(connector.clone(), Interaction::Forbidden);
                let selected = profile
                    .find_decision_model(
                        with_device_models(),
                        Some(&probe),
                        Arc::new(HTTPClient::new_without_refetch()),
                    )
                    .await
                    .unwrap();
                assert_eq!(selected.id, expected);
                assert_eq!(connector.calls(), 1);
            }
        }

        async fn select(
            profile: &Profile,
            model_id: Option<&str>,
            capabilities: CompletionModelCapabilities,
            devices: Option<&DeviceModelProbe>,
        ) -> Bit {
            profile
                .resolve_completion_model(
                    model_id,
                    &BitModelPreference::default(),
                    false,
                    capabilities,
                    devices,
                    Arc::new(HTTPClient::new_without_refetch()),
                )
                .await
                .expect("a model is selected")
        }

        #[tokio::test]
        async fn find_model_falls_back_when_the_device_stays_locked() {
            let connector = FakeConnector::failing(ModelUnavailableReason::DeviceLockedDeclined);
            let run = Interaction::Allowed {
                run_label: Some("run-1".to_string()),
            };
            let probe = DeviceModelProbe::new(connector.clone(), run.clone());

            let selected = select(&profile(), None, with_device_models(), Some(&probe)).await;
            assert_eq!(selected.id, "fallback");
            assert_eq!(*connector.interactions.lock(), vec![run]);

            let preferred = select(
                &profile(),
                Some("device-model"),
                with_device_models(),
                Some(&probe),
            )
            .await;
            assert_eq!(preferred.id, "fallback");
            assert_eq!(connector.calls(), 2, "one probe per selection");
        }

        #[tokio::test]
        async fn find_model_picks_a_reachable_device_model() {
            let probe = DeviceModelProbe::new(
                FakeConnector::serving(endpoint("http://127.0.0.1:9/v1")),
                Interaction::Forbidden,
            );

            let selected = select(&profile(), None, with_device_models(), Some(&probe)).await;
            assert_eq!(selected.id, "device-model");
        }

        #[tokio::test]
        async fn hosts_without_a_connector_skip_device_models() {
            for model_id in [None, Some("device-model")] {
                let selected = select(
                    &profile(),
                    model_id,
                    CompletionModelCapabilities::default(),
                    None,
                )
                .await;
                assert_eq!(selected.id, "fallback", "{model_id:?}");
            }

            let unprobed = select(&profile(), None, with_device_models(), None).await;
            assert_eq!(unprobed.id, "fallback");
        }

        #[test]
        fn device_models_are_filtered_from_hosted_only_selection() {
            assert!(Profile::is_local_model(&device_llm_bit(
                "device-model",
                "dev-1"
            )));
        }

        /// A placement's probe: the device's model host behind a router, and no connector.
        fn on_a_device() -> DeviceModelProbe {
            let router = Arc::new(FakeRouter(endpoint("http://127.0.0.1:9/v1")));
            DeviceModelProbe::for_host(None, Some(router), Interaction::Forbidden)
                .expect("a router makes a probe")
        }

        #[tokio::test]
        async fn find_model_on_a_device_picks_the_bits_its_model_host_serves() {
            let probe = on_a_device();
            for served in [
                completion_bit("local-model", "Local"),
                device_llm_bit("own-model", "this-device"),
            ] {
                let profile = profile_with_models(vec![
                    served.clone(),
                    completion_bit("fallback", "hosted:openai"),
                ]);
                let capabilities = CompletionModelCapabilities::default();

                let selected = select(&profile, None, capabilities, Some(&probe)).await;
                assert_eq!(selected.id, served.id);
                let preferred = select(
                    &profile,
                    Some(served.id.as_str()),
                    capabilities,
                    Some(&probe),
                )
                .await;
                assert_eq!(preferred.id, served.id);

                let unrouted = select(&profile, None, capabilities, None).await;
                assert_eq!(unrouted.id, "fallback");
            }
        }

        #[tokio::test]
        async fn bits_the_model_host_does_not_serve_still_need_their_runtime_here() {
            let probe = on_a_device();
            let profile = profile_with_models(vec![
                completion_bit("mlx-model", "MLX"),
                completion_bit("fallback", "hosted:openai"),
            ]);

            let without_mlx = select(
                &profile,
                None,
                CompletionModelCapabilities::default(),
                Some(&probe),
            )
            .await;
            assert_eq!(without_mlx.id, "fallback");

            let with_mlx = CompletionModelCapabilities {
                mlx: true,
                ..CompletionModelCapabilities::default()
            };
            let selected = select(&profile, None, with_mlx, Some(&probe)).await;
            assert_eq!(selected.id, "mlx-model");
        }

        #[tokio::test]
        async fn one_selection_asks_a_declined_device_once() {
            let connector = FakeConnector::failing(ModelUnavailableReason::DeviceLockedDeclined);
            let probe =
                DeviceModelProbe::new(connector.clone(), Interaction::Allowed { run_label: None });
            let profile = profile_with_models(vec![
                device_llm_bit("first", "dev-1"),
                device_llm_bit("second", "dev-1"),
                completion_bit("fallback", "hosted:openai"),
            ]);

            let selected = select(&profile, None, with_device_models(), Some(&probe)).await;
            assert_eq!(selected.id, "fallback");
            assert_eq!(
                connector.calls(),
                1,
                "the second model is on the declined device"
            );

            let preferred =
                select(&profile, Some("second"), with_device_models(), Some(&probe)).await;
            assert_eq!(preferred.id, "fallback");
            assert_eq!(connector.calls(), 2, "the next selection asks again");
        }

        #[tokio::test]
        async fn a_missing_model_does_not_rule_out_its_device() {
            let connector = FakeConnector::failing(ModelUnavailableReason::ModelMissing);
            let probe = DeviceModelProbe::new(connector.clone(), Interaction::Forbidden);
            let profile = profile_with_models(vec![
                device_llm_bit("first", "dev-1"),
                device_llm_bit("second", "dev-1"),
                completion_bit("fallback", "hosted:openai"),
            ]);

            let selected = select(&profile, None, with_device_models(), Some(&probe)).await;
            assert_eq!(selected.id, "fallback");
            assert_eq!(connector.calls(), 2);
        }
    }
}

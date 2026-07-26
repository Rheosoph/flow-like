use std::{
    collections::{HashMap, HashSet},
    hash::{Hash, Hasher},
    sync::Arc,
};

use crate::{
    bit::{Bit, BitModelPreference, BitTypes},
    hub::{BitSearchQuery, Hub},
    utils::http::HTTPClient,
};
use flow_like_types::{Result, Value, anyhow, tokio::task};
use futures::future::join_all;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

fn split_profile_bit_reference(reference: &str) -> Option<(&str, &str)> {
    let (hub, bit_id) = reference.rsplit_once(':')?;
    if hub.trim().is_empty() || bit_id.trim().is_empty() {
        return None;
    }

    Some((hub, bit_id))
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
    fn is_local_provider_name(provider_name: &str) -> bool {
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
        )
    }

    /// Check if a bit is a local model (requires local hosting capabilities)
    fn is_local_model(bit: &Bit) -> bool {
        if bit.bit_type == crate::bit::BitTypes::Tts {
            return true;
        }

        if let Ok(llm_params) =
            flow_like_types::json::from_value::<crate::bit::LLMParameters>(bit.parameters.clone())
        {
            if Self::is_local_provider_name(&llm_params.provider.provider_name) {
                return true;
            }
        } else if let Ok(vlm_params) =
            flow_like_types::json::from_value::<crate::bit::VLMParameters>(bit.parameters.clone())
        {
            if Self::is_local_provider_name(&vlm_params.provider.provider_name) {
                return true;
            }
        }

        false
    }

    /// Gets the best model based on the preference
    /// For remote we are also looking on hubs for available models (for recommendations, for example)
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
        let mut best_bit = (0.0, None);

        for bit in self.custom_bits.iter().map(|custom| &custom.0) {
            if only_hosted && Self::is_local_model(bit) {
                continue;
            }
            if multimodal && !bit.is_multimodal() {
                continue;
            }
            if let Ok(score) = bit.score(preference)
                && (best_bit.1.is_none() || score > best_bit.0)
            {
                best_bit = (score, Some(bit.clone()));
            }
        }

        if !remote {
            for bit_ref in &self.bits {
                let bit = match self.get_profile_bit(bit_ref, http_client.clone()).await {
                    Ok(bit) => bit,
                    Err(err) => {
                        println!("Skipping unresolved profile bit {bit_ref}: {err}");
                        continue;
                    }
                };

                // Skip local models if only_hosted is true
                if only_hosted && Self::is_local_model(&bit) {
                    continue;
                }

                if multimodal && !bit.is_multimodal() {
                    continue;
                }
                if let Ok(score) = bit.score(preference)
                    && (best_bit.1.is_none() || (score > best_bit.0))
                {
                    best_bit = (score, Some(bit.clone()));
                }
            }

            if let Some(bit) = best_bit.1 {
                return Ok(bit);
            }
        }

        let preference = preference.parse();
        let available_hubs = self.get_available_hubs(http_client).await?;
        let mut bits: HashMap<String, Bit> = HashMap::new();
        let query = BitSearchQuery::builder()
            .with_bit_types(vec![BitTypes::Vlm, BitTypes::Llm])
            .build();
        for hub in available_hubs {
            match hub.search_bit(&query).await {
                Ok(models) => {
                    bits.extend(models.into_iter().map(|bit| (bit.id.clone(), bit.clone())));
                }
                Err(_) => {
                    continue;
                }
            };
        }

        for (_, bit) in bits {
            // Skip local models if only_hosted is true
            if only_hosted && Self::is_local_model(&bit) {
                continue;
            }

            if multimodal && !bit.is_multimodal() {
                continue;
            }

            if let Ok(score) = bit.score(&preference)
                && (best_bit.1.is_none() || score > best_bit.0)
            {
                best_bit = (score, Some(bit.clone()));
            }
        }

        match best_bit.1 {
            Some(bit) => Ok(bit),
            None => Err(anyhow!("No Model found")),
        }
    }

    /// Looks up a user-owned custom bit carried on this profile by id.
    pub fn custom_bit(&self, bit_id: &str) -> Option<Bit> {
        self.custom_bits
            .iter()
            .map(|custom| &custom.0)
            .find(|bit| bit.id == bit_id)
            .cloned()
    }

    fn custom_bits_matching(&self, query: &BitSearchQuery) -> Vec<Bit> {
        self.custom_bits
            .iter()
            .map(|custom| &custom.0)
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
                    println!("Bit could not be queried: {}", err);
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
            let bit = hub.get_bit(&bit).await;
            if let Ok(bit) = bit {
                return Ok(bit);
            }
        }
        Err(flow_like_types::anyhow!("Bit not found"))
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
        Err(flow_like_types::anyhow!("Bit not found"))
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
    use super::{Profile, split_profile_bit_reference};
    use crate::{
        bit::{BitModelClassification, BitModelPreference, BitTypes, VLMParameters},
        utils::http::HTTPClient,
    };
    use flow_like_model_provider::provider::ModelProvider;
    use flow_like_types::tokio;
    use std::sync::Arc;

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

        assert_eq!(err.to_string(), "No Model found");
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
}

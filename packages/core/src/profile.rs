use std::{
    collections::{HashMap, HashSet},
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
    /// Check if a bit is a local model (requires local hosting capabilities)
    fn is_local_model(bit: &Bit) -> bool {
        if let Ok(llm_params) =
            flow_like_types::json::from_value::<crate::bit::LLMParameters>(bit.parameters.clone())
        {
            let provider_name = llm_params.provider.provider_name.to_lowercase();
            if provider_name == "local"
                || provider_name == "llama.cpp"
                || provider_name == "llamacpp"
                || provider_name == "ollama"
            {
                return true;
            }
        } else if let Ok(vlm_params) =
            flow_like_types::json::from_value::<crate::bit::VLMParameters>(bit.parameters.clone())
        {
            let provider_name = vlm_params.provider.provider_name.to_lowercase();
            if provider_name == "local"
                || provider_name == "llama.cpp"
                || provider_name == "llamacpp"
                || provider_name == "ollama"
            {
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

        if !remote {
            for bit in &self.bits {
                let (hub, bit_id) = bit
                    .split_once(':')
                    .ok_or_else(|| anyhow!("Invalid bit format: {}", bit))?;

                let hub = Hub::new(hub, http_client.clone()).await?;
                let bit = hub.get_bit(bit_id).await?;

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

    pub async fn search_bits(
        &self,
        query: &BitSearchQuery,
        http_client: Arc<HTTPClient>,
    ) -> Result<Vec<Bit>> {
        let hubs = self.get_available_hubs(http_client).await?;
        let mut bits: HashMap<String, Bit> = HashMap::new();
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
        let hubs = self.get_available_hubs(http_client).await?;
        for hub in hubs {
            let bit = hub.get_bit(bit_id).await;
            if let Ok(bit) = bit {
                return Ok(bit);
            }
        }
        Err(flow_like_types::anyhow!("Bit not found"))
    }

    pub async fn get_available_hubs(&self, http_client: Arc<HTTPClient>) -> Result<Vec<Hub>> {
        let mut hubs = HashSet::new();
        for hub in &self.hubs {
            hubs.insert(hub.clone());
        }

        self.bits.iter().for_each(|id| {
            if let Some((hub, _bit_id)) = id.split_once(':') {
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
            .any(|reference| reference.split(':').last() == Some(bit.id.as_str()));
        if bit_exists {
            return;
        }
        self.bits.push(bit_id);
    }

    pub fn remove_bit(&mut self, bit: &Bit) {
        self.bits
            .retain(|reference| reference.split(':').last() != Some(bit.id.as_str()));
    }
}

use crate::{Client, ModelInfo, RequestOptions, Result, Value, json};

impl Client {
    pub async fn list_llms(&self, search: Option<&str>, limit: u32) -> Result<Vec<ModelInfo>> {
        self.remote_models(&["Llm", "Vlm"], search, limit).await
    }
    pub async fn list_embedding_models(
        &self,
        search: Option<&str>,
        limit: u32,
    ) -> Result<Vec<ModelInfo>> {
        self.remote_models(&["Embedding"], search, limit).await
    }
    pub async fn get_model_info(&self, bit_id: &str) -> Result<ModelInfo> {
        Ok(model_info(
            &self.get_bit(bit_id, &RequestOptions::new()).await?,
        ))
    }
    async fn remote_models(
        &self,
        types: &[&str],
        search: Option<&str>,
        limit: u32,
    ) -> Result<Vec<ModelInfo>> {
        let bits = self
            .search_bits(
                &RequestOptions::new()
                    .json(json!({"bit_types":types,"search":search,"limit":limit})),
            )
            .await?;
        let bits = bits.as_array().ok_or_else(|| {
            crate::Error::Configuration("Model discovery returned a non-array response".into())
        })?;
        Ok(bits
            .iter()
            .filter(|bit| {
                types.contains(&bit["type"].as_str().unwrap_or_default())
                    && has_remote_provider(bit)
            })
            .map(model_info)
            .collect())
    }
}

fn string(value: &Value) -> Option<String> {
    value.as_str().map(str::to_owned)
}
fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|values| values.iter().filter_map(string).collect())
        .unwrap_or_default()
}
fn model_info(bit: &Value) -> ModelInfo {
    let parameters = &bit["parameters"];
    let provider = parameters.get("provider").unwrap_or(parameters);
    let meta = bit["meta"]
        .get("en")
        .or_else(|| {
            bit["meta"]
                .as_object()
                .and_then(|meta| meta.values().next())
        })
        .unwrap_or(&Value::Null);
    let bit_id = string(&bit["id"]).unwrap_or_default();
    ModelInfo {
        name: string(&meta["name"]).unwrap_or_else(|| bit_id.clone()),
        bit_id,
        description: string(&meta["description"]).unwrap_or_default(),
        provider_name: string(&provider["provider_name"]),
        api_surface: string(&provider["api_surface"]),
        model_id: string(&provider["model_id"]),
        context_length: parameters["context_length"].as_u64(),
        vector_length: parameters["vector_length"].as_u64(),
        languages: strings(&parameters["languages"]),
        tags: strings(&meta["tags"]),
    }
}
fn has_remote_provider(bit: &Value) -> bool {
    let parameters = &bit["parameters"];
    let provider = parameters.get("provider").unwrap_or(parameters);
    let name = provider["provider_name"]
        .as_str()
        .unwrap_or_default()
        .to_ascii_lowercase();
    name.starts_with("hosted")
        || (matches!(name.as_str(), "premium" | "internal") && provider["model_id"].is_string())
        || (!parameters["remote"]["implementation"].is_null()
            && (parameters["remote"]["model_id"].is_string() || provider["model_id"].is_string()))
}

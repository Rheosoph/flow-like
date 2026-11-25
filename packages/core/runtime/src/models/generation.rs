//! Resolve the weight roles declared by an image or video model Bit into local files.

use crate::flow::execution::{
    ExecutionEnvironment,
    egress::{client_builder, ensure_url_resolves_allowed},
};
use crate::{
    bit::{Bit, BitPack, safe_mlx_asset_path},
    hub::Hub,
    models::local_utils::ensure_local_weights,
    profile::Profile,
    state::FlowLikeState,
};
use flow_like_model_provider::{
    provider::ModelProvider,
    stablediffusion::{
        GeneratedAsset, GenerationHost, GenerationRequest, PROVIDER_NAME, StableDiffusionConfig,
        generate_on,
    },
};
use flow_like_storage::{Path as StorePath, files::store::FlowLikeStore};
use flow_like_types::{Context, Result, anyhow, bail, tokio::fs};
use serde::Deserialize;
use std::{collections::HashSet, path::Path, sync::Arc};

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
enum WeightRole {
    Model,
    DiffusionModel,
    Vae,
    ClipL,
    ClipG,
    T5xxl,
    Llm,
}

impl WeightRole {
    fn set_path(self, config: &mut StableDiffusionConfig, path: String) {
        *match self {
            Self::Model => &mut config.model_path,
            Self::DiffusionModel => &mut config.diffusion_model_path,
            Self::Vae => &mut config.vae_path,
            Self::ClipL => &mut config.clip_l_path,
            Self::ClipG => &mut config.clip_g_path,
            Self::T5xxl => &mut config.t5xxl_path,
            Self::Llm => &mut config.llm_path,
        } = Some(path);
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WeightAsset {
    bit: String,
    role: WeightRole,
}

fn weight_reference<'a>(
    reference: &'a str,
    default_hub: &'a str,
) -> Result<(Option<&'a str>, &'a str)> {
    let (hub, id) = match reference.rsplit_once(':') {
        Some((hub, id)) => (Some(hub), id),
        None => ((!default_hub.is_empty()).then_some(default_hub), reference),
    };
    if id.trim().is_empty() || hub.is_some_and(|hub| hub.trim().is_empty()) {
        bail!("Generation asset reference must contain a Bit ID and a nonempty hub when qualified");
    }
    Ok((hub, id))
}

fn custom_weight_for_hub(bit: Option<Bit>, hub: Option<&str>) -> Option<Bit> {
    fn host(hub: &str) -> &str {
        let hub = hub.trim_end_matches('/');
        hub.strip_prefix("https://")
            .or_else(|| hub.strip_prefix("http://"))
            .unwrap_or(hub)
    }
    bit.filter(|bit| hub.is_none_or(|hub| host(&bit.hub) == host(hub)))
}

fn weight_manifest(root: &Bit) -> Result<Option<Vec<WeightAsset>>> {
    let Some(value) = root.parameters.get("assets") else {
        return Ok(None);
    };
    let assets: Vec<WeightAsset> =
        serde_json::from_value(value.clone()).context("Invalid generation weight manifest")?;
    if assets.is_empty() {
        bail!("Generation model {} has no weight assets", root.id);
    }
    let dependencies: HashSet<_> = root
        .dependencies
        .iter()
        .map(|reference| weight_reference(reference, &root.hub))
        .collect::<Result<_>>()?;
    if dependencies.len() != root.dependencies.len() {
        bail!("Generation model {} has duplicate dependencies", root.id);
    }
    let mut roles = HashSet::new();
    let mut ids = HashSet::new();
    for asset in &assets {
        let reference = weight_reference(&asset.bit, &root.hub)?;
        if reference == weight_reference(&root.id, &root.hub)? {
            bail!("Generation asset must reference a nonempty dependency other than its root");
        }
        if !dependencies.contains(&reference) {
            bail!(
                "Generation asset {} is not a declared dependency",
                asset.bit
            );
        }
        if !ids.insert(reference) {
            bail!("Generation asset {} has more than one role", asset.bit);
        }
        if !roles.insert(asset.role) {
            bail!(
                "Generation weight role {:?} is assigned more than once",
                asset.role
            );
        }
    }
    if ids.len() != dependencies.len() {
        bail!("Every generation dependency must have a weight role");
    }
    if roles.contains(&WeightRole::Model) == roles.contains(&WeightRole::DiffusionModel) {
        bail!("Generation assets require exactly one model or diffusion_model role");
    }
    Ok(Some(assets))
}

fn validate_weight_bit(asset: &WeightAsset, bit: &Bit) -> Result<()> {
    if bit.id != weight_reference(&asset.bit, "")?.1 {
        bail!(
            "Generation asset {} resolved to a different Bit {}",
            asset.bit,
            bit.id
        );
    }
    if bit
        .download_link
        .as_deref()
        .is_none_or(|link| link.trim().is_empty())
    {
        bail!(
            "Generation asset {} must be a downloadable weight file",
            bit.id
        );
    }
    if bit.size.is_none_or(|size| size == 0) {
        bail!("Generation asset {} has no positive file size", bit.id);
    }
    if !bit.dependencies.is_empty() {
        bail!(
            "Generation asset {} must reference one weight file without dependencies",
            bit.id
        );
    }
    let file_name = bit
        .file_name
        .as_deref()
        .ok_or_else(|| anyhow!("Generation asset {} has no file name", bit.id))?;
    safe_mlx_asset_path(file_name)
        .with_context(|| format!("Invalid generation asset file name for {}", bit.id))?;
    safe_mlx_asset_path(&bit.hash)
        .with_context(|| format!("Invalid generation asset cache hash for {}", bit.id))?;
    if bit.hash.contains('/') {
        bail!(
            "Generation asset {} cache hash must be one path component",
            bit.id
        );
    }
    Ok(())
}

/// Runs stable-diffusion.cpp generation under the environment's policy.
///
/// Locally nothing changes. Server-side the managed runtime is refused before
/// any process starts, host model paths are refused, and the endpoint is
/// vetted (URL, then DNS at connect time through the guarded resolver, no
/// redirects) like every other flow-supplied URL; see `flow::execution::egress`
/// for what that policy covers (private RFC 1918 space is left to the network
/// layer, loopback / link-local / metadata are refused).
pub async fn generate_guarded(
    environment: ExecutionEnvironment,
    config: &StableDiffusionConfig,
    request: &GenerationRequest,
) -> Result<Vec<GeneratedAsset>> {
    if environment != ExecutionEnvironment::Server {
        return generate_on(&GenerationHost::Local, config, request).await;
    }
    let Some(endpoint) = config.endpoint.as_deref() else {
        bail!(
            "The local stable-diffusion.cpp runtime is not available in server-side execution; \
             configure an sd-server endpoint or use a hosted image/video provider"
        );
    };
    for (flag, path) in config.model_paths() {
        if path.is_some() {
            environment.ensure_host_filesystem_access(&format!("stable-diffusion.cpp {flag}"))?;
        }
    }
    config.validate()?;
    let endpoint_client = server_endpoint_client(environment, endpoint).await?;
    generate_on(&GenerationHost::Server { endpoint_client }, config, request).await
}

async fn server_endpoint_client(
    environment: ExecutionEnvironment,
    endpoint: &str,
) -> Result<flow_like_types::reqwest::Client> {
    let url = flow_like_types::reqwest::Url::parse(endpoint.trim())
        .context("Invalid sd-server endpoint URL")?;
    ensure_url_resolves_allowed(environment, &url).await?;
    client_builder(environment)
        .connect_timeout(std::time::Duration::from_secs(5))
        .redirect(flow_like_types::reqwest::redirect::Policy::none())
        .build()
        .context("Failed to build sd-server HTTP client")
}

fn pack_configuration(provider: &ModelProvider) -> Result<StableDiffusionConfig> {
    let mut config: StableDiffusionConfig = provider
        .params
        .as_ref()
        .and_then(|params| params.get("stablediffusion"))
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .context("Invalid stable-diffusion.cpp model configuration")?
        .unwrap_or_default();
    if config.endpoint.is_some()
        || config.model_path.is_some()
        || config.diffusion_model_path.is_some()
        || config.vae_path.is_some()
        || config.clip_l_path.is_some()
        || config.clip_g_path.is_some()
        || config.t5xxl_path.is_some()
        || config.llm_path.is_some()
    {
        bail!(
            "Generation weight packs select their files through asset roles; remove endpoint and explicit model paths"
        );
    }
    // Validate runtime settings before downloading. Actual paths are assigned after resolution.
    config.model_path = Some("pending-weight-resolution".to_string());
    config.validate()?;
    config.model_path = None;
    Ok(config)
}

async fn checked_weight_path(path: &Path, store_root: &Path, bit: &Bit) -> Result<String> {
    let metadata = fs::symlink_metadata(path)
        .await
        .with_context(|| format!("Generation weight {} was not downloaded", bit.id))?;
    if !metadata.is_file() || metadata.len() == 0 || Some(metadata.len()) != bit.size {
        bail!(
            "Generation weight {} is not a complete regular file of its declared size",
            bit.id
        );
    }
    let canonical = fs::canonicalize(path).await?;
    if !canonical.starts_with(store_root) {
        bail!(
            "Generation weight {} resolves outside the local Bit store",
            bit.id
        );
    }
    canonical
        .into_os_string()
        .into_string()
        .map_err(|_| anyhow!("Generation weight {} path is not valid UTF-8", bit.id))
}

/// Weight packs download from Bit-chosen hubs and write to the host Bit store, so the
/// server executor refuses them before any network or disk access.
fn ensure_weight_packs_allowed(environment: ExecutionEnvironment) -> Result<()> {
    if environment == ExecutionEnvironment::Server {
        bail!(
            "stable-diffusion.cpp weight packs are not available in server-side execution; \
             configure an sd-server endpoint or use a hosted image/video provider"
        );
    }
    Ok(())
}

/// Download a model Bit's declared weights and supply their local paths to its provider.
/// Explicit local-file and remote providers pass through when no asset manifest is present.
pub async fn resolve_generation_provider(
    root: &Bit,
    provider: &ModelProvider,
    profile: &Profile,
    state: &Arc<FlowLikeState>,
    environment: ExecutionEnvironment,
) -> Result<ModelProvider> {
    if provider.provider_name != PROVIDER_NAME {
        return Ok(provider.clone());
    }
    let Some(assets) = weight_manifest(root)? else {
        return Ok(provider.clone());
    };
    // Refuse before any hub request, weight download or Bit-store write.
    ensure_weight_packs_allowed(environment)?;
    let mut config = pack_configuration(provider)?;
    let FlowLikeStore::Local(store) = FlowLikeState::bit_store(state).await? else {
        bail!("stable-diffusion.cpp weight packs require a local Bit store");
    };
    let store_root = store.path_to_filesystem(&StorePath::from(""))?;
    fs::create_dir_all(&store_root).await?;
    let store_root = fs::canonicalize(store_root).await?;
    let mut bits = Vec::with_capacity(assets.len());
    for asset in &assets {
        let (hub, id) = weight_reference(&asset.bit, &root.hub)?;
        // A custom Bit can satisfy its own hub reference, but cannot shadow the
        // same ID from another registry. Registry responses may declare a hub alias.
        let bit = if let Some(bit) = custom_weight_for_hub(profile.custom_bit(id), hub) {
            bit
        } else if let Some(hub) = hub {
            Hub::new(hub, state.http_client.clone())
                .await?
                .get_bit(id)
                .await
                .with_context(|| format!("Could not resolve generation asset {}", asset.bit))?
        } else {
            profile
                .get_bit(id.to_string(), None, state.http_client.clone())
                .await
                .with_context(|| format!("Could not resolve generation asset {}", asset.bit))?
        };
        validate_weight_bit(asset, &bit)?;
        bits.push(bit);
    }
    let pack = BitPack { bits };
    ensure_local_weights(&pack, state, &root.id, "generation model").await?;
    for (asset, bit) in assets.iter().zip(&pack.bits) {
        let path = bit
            .to_path(&store)
            .ok_or_else(|| anyhow!("No local path for generation weight {}", bit.id))?;
        asset.role.set_path(
            &mut config,
            checked_weight_path(&path, &store_root, bit).await?,
        );
    }
    config.validate()?;
    let mut resolved = provider.clone();
    resolved
        .params
        .get_or_insert_default()
        .insert("stablediffusion".to_string(), serde_json::to_value(config)?);
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn root(assets: serde_json::Value, dependencies: &[&str]) -> Bit {
        Bit {
            id: "root".into(),
            parameters: json!({"assets": assets}),
            dependencies: dependencies.iter().map(|id| (*id).to_string()).collect(),
            ..Default::default()
        }
    }

    fn weight() -> Bit {
        Bit {
            id: "weights".into(),
            download_link: Some("https://example.com/model.gguf".into()),
            file_name: Some("model.gguf".into()),
            hash: "content-hash".into(),
            size: Some(4),
            ..Default::default()
        }
    }

    #[test]
    fn manifest_requires_complete_unique_role_references() {
        let valid = json!([{"bit":"weights","role":"diffusion_model"},{"bit":"vae","role":"vae"}]);
        assert_eq!(
            weight_manifest(&root(valid.clone(), &["weights", "vae"]))
                .unwrap()
                .unwrap()
                .len(),
            2
        );
        for (assets, deps) in [
            (valid.clone(), vec!["weights"]),
            (valid.clone(), vec!["weights", "vae", "unused"]),
            (valid, vec!["weights", "vae", "vae"]),
            (json!([]), vec![]),
            (json!([{"bit":"weights","role":"vae"}]), vec!["weights"]),
            (
                json!([{"bit":"weights","role":"model"},{"bit":"vae","role":"diffusion_model"}]),
                vec!["weights", "vae"],
            ),
            (
                json!([{"bit":"weights","role":"model"},{"bit":"vae","role":"model"}]),
                vec!["weights", "vae"],
            ),
            (
                json!([{"bit":"weights","role":"model"},{"bit":"weights","role":"vae"}]),
                vec!["weights"],
            ),
            (json!([{"bit":"root","role":"model"}]), vec!["root"]),
            (json!([{"bit":"weights","role":"unknown"}]), vec!["weights"]),
        ] {
            assert!(weight_manifest(&root(assets, &deps)).is_err());
        }
        assert!(weight_manifest(&Bit::default()).unwrap().is_none());
    }

    #[test]
    fn weights_require_downloadable_complete_safe_file_metadata() {
        let asset = WeightAsset {
            bit: "weights".into(),
            role: WeightRole::Model,
        };
        validate_weight_bit(&asset, &weight()).unwrap();
        for bit in [
            Bit {
                id: "other".into(),
                ..weight()
            },
            Bit {
                download_link: None,
                ..weight()
            },
            Bit {
                file_name: None,
                ..weight()
            },
            Bit {
                file_name: Some("../outside.gguf".into()),
                ..weight()
            },
            Bit {
                hash: "../outside".into(),
                ..weight()
            },
            Bit {
                hash: "".into(),
                ..weight()
            },
            Bit {
                size: Some(0),
                ..weight()
            },
            Bit {
                size: None,
                ..weight()
            },
            Bit {
                dependencies: vec!["root".into()],
                ..weight()
            },
        ] {
            assert!(validate_weight_bit(&asset, &bit).is_err());
        }
    }

    #[test]
    fn qualified_references_select_the_asset_hub_and_validate_bare_ids() {
        assert_eq!(
            weight_reference("weights", "root.example").unwrap(),
            (Some("root.example"), "weights")
        );
        assert_eq!(
            weight_reference("other.example:weights", "root.example").unwrap(),
            (Some("other.example"), "weights")
        );
        assert_eq!(
            weight_reference("https://other.example:8443:weights", "root.example").unwrap(),
            (Some("https://other.example:8443"), "weights")
        );
        let asset = WeightAsset {
            bit: "other.example:weights".into(),
            role: WeightRole::Model,
        };
        validate_weight_bit(&asset, &weight()).unwrap();
        let mut root = root(
            json!([
                {"bit":"root.example:weights","role":"diffusion_model"},
                {"bit":"other.example:vae","role":"vae"}
            ]),
            &["weights", "other.example:vae"],
        );
        root.hub = "root.example".into();
        assert!(weight_manifest(&root).is_ok());
        root.dependencies.push("root.example:weights".into());
        assert!(weight_manifest(&root).is_err());
        for reference in ["", ":weights", "hub:"] {
            assert!(weight_reference(reference, "root.example").is_err());
        }
    }

    #[test]
    fn qualified_registry_reference_cannot_be_shadowed_by_another_hubs_custom_bit() {
        let custom = Bit {
            hub: "custom.example".into(),
            ..weight()
        };
        assert!(custom_weight_for_hub(Some(custom.clone()), Some("registry.example")).is_none());
        assert!(
            custom_weight_for_hub(Some(custom.clone()), Some("https://custom.example/")).is_some()
        );
        assert!(custom_weight_for_hub(Some(custom), None).is_some());
    }

    #[test]
    fn role_mapping_keeps_runtime_settings_and_model_defaults() {
        let provider = ModelProvider {
            provider_name: PROVIDER_NAME.into(),
            model_id: None,
            version: None,
            api_surface: None,
            params: Some(std::collections::HashMap::from([
                ("stablediffusion".into(), json!({"offload_to_cpu":false})),
                ("generation_defaults".into(), json!({"width":1024})),
            ])),
        };
        let mut config = pack_configuration(&provider).unwrap();
        for (role, path) in [
            (WeightRole::DiffusionModel, "diffusion"),
            (WeightRole::Vae, "vae"),
            (WeightRole::ClipL, "clip_l"),
            (WeightRole::ClipG, "clip_g"),
            (WeightRole::T5xxl, "t5"),
            (WeightRole::Llm, "llm"),
        ] {
            role.set_path(&mut config, path.into());
        }
        assert_eq!(config.diffusion_model_path.as_deref(), Some("diffusion"));
        assert_eq!(config.vae_path.as_deref(), Some("vae"));
        assert_eq!(config.clip_l_path.as_deref(), Some("clip_l"));
        assert_eq!(config.clip_g_path.as_deref(), Some("clip_g"));
        assert_eq!(config.t5xxl_path.as_deref(), Some("t5"));
        assert_eq!(config.llm_path.as_deref(), Some("llm"));
        assert!(!config.offload_to_cpu);
        assert!(config.model_path.is_none());
        assert_eq!(
            provider.params.unwrap()["generation_defaults"]["width"],
            1024
        );
    }

    #[tokio::test]
    async fn cached_weights_reject_truncation_directories_and_paths_outside_store() {
        let store = tempfile::tempdir().unwrap();
        let store_root = fs::canonicalize(store.path()).await.unwrap();
        let path = store.path().join("model.gguf");
        fs::write(&path, b"test").await.unwrap();
        assert!(
            checked_weight_path(&path, &store_root, &weight())
                .await
                .is_ok()
        );
        fs::write(&path, b"bad").await.unwrap();
        assert!(
            checked_weight_path(&path, &store_root, &weight())
                .await
                .is_err()
        );
        assert!(
            checked_weight_path(store.path(), &store_root, &weight())
                .await
                .is_err()
        );
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("model.gguf"), b"test")
            .await
            .unwrap();
        assert!(
            checked_weight_path(&outside.path().join("model.gguf"), &store_root, &weight())
                .await
                .is_err()
        );
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(outside.path(), store.path().join("linked")).unwrap();
            assert!(
                checked_weight_path(
                    &store.path().join("linked/model.gguf"),
                    &store_root,
                    &weight()
                )
                .await
                .is_err()
            );
        }
    }

    fn sd_request() -> GenerationRequest {
        GenerationRequest {
            mode: flow_like_model_provider::stablediffusion::GenerationMode::Image,
            params: serde_json::json!({"prompt": "a landscape"}),
        }
    }

    fn endpoint_config(endpoint: &str) -> StableDiffusionConfig {
        StableDiffusionConfig {
            endpoint: Some(endpoint.into()),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn server_refuses_host_plane_endpoints_before_any_request() {
        for endpoint in [
            "http://127.0.0.1:7860",
            "http://169.254.169.254/latest",
            "http://[::1]:7860",
            "http://localhost:7860",
            "http://metadata.google.internal",
        ] {
            let error = generate_guarded(
                ExecutionEnvironment::Server,
                &endpoint_config(endpoint),
                &sd_request(),
            )
            .await
            .unwrap_err()
            .to_string();
            assert!(error.contains("refused"), "{endpoint}: {error}");
        }
    }

    #[tokio::test]
    async fn server_endpoint_vetting_allows_public_and_rfc1918_literals() {
        // Same policy as every other flow-supplied URL: RFC 1918 is the
        // deployment's own network and is left to the network layer.
        for endpoint in ["https://93.184.216.34/sd", "http://10.0.0.1:7860"] {
            server_endpoint_client(ExecutionEnvironment::Server, endpoint)
                .await
                .unwrap_or_else(|e| panic!("{endpoint}: {e}"));
        }
    }

    #[tokio::test]
    async fn server_refuses_local_runtime_and_host_paths_without_spawning() {
        let config = StableDiffusionConfig {
            model_path: Some("/etc/passwd".into()),
            ..Default::default()
        };
        let error = generate_guarded(ExecutionEnvironment::Server, &config, &sd_request())
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("server-side execution"), "{error}");
        assert!(
            !error.contains("not found"),
            "must not reach the runtime: {error}"
        );

        let mut config = endpoint_config("https://93.184.216.34/sd");
        config.model_path = Some("/etc/passwd".into());
        let error = generate_guarded(ExecutionEnvironment::Server, &config, &sd_request())
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("host filesystem"), "{error}");

        // The provider layer refuses on its own, independent of the wrapper.
        let client = flow_like_types::reqwest::Client::new();
        let error = generate_on(
            &GenerationHost::Server {
                endpoint_client: client,
            },
            &StableDiffusionConfig {
                model_path: Some("/nonexistent/model.safetensors".into()),
                ..Default::default()
            },
            &sd_request(),
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(error.contains("server-side execution"), "{error}");
    }

    #[tokio::test]
    async fn local_still_reaches_private_endpoints() {
        // Port 1 is closed: the request is attempted and fails on connect,
        // not on the egress policy.
        let error = generate_guarded(
            ExecutionEnvironment::Local,
            &endpoint_config("http://127.0.0.1:1"),
            &sd_request(),
        )
        .await
        .unwrap_err();
        assert!(!format!("{error:#}").contains("network plane"), "{error:#}");
    }

    #[test]
    fn server_refuses_weight_packs_before_io() {
        assert!(ensure_weight_packs_allowed(ExecutionEnvironment::Server).is_err());
        assert!(ensure_weight_packs_allowed(ExecutionEnvironment::Local).is_ok());
    }
}

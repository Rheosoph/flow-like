use super::forecasting::ForecastModelHandle;
#[cfg(any(feature = "studio", test))]
use flow_like::bit::{BitPack, BitTypes, ForecastingModelParameters};
use flow_like::{
    bit::Bit,
    flow::{
        execution::context::ExecutionContext,
        node::{Node, NodeLogic},
        pin::PinOptions,
        variable::VariableType,
    },
};
#[cfg(feature = "studio")]
use flow_like::{bit::ForecastingModelType, state::FlowLikeState};
#[cfg(feature = "studio")]
use flow_like_ml_forecast::ForecastFamily;
use flow_like_types::{Result, json::json};
#[cfg(any(feature = "studio", test))]
use flow_like_types::{anyhow, bail};
#[cfg(any(feature = "studio", test))]
use std::collections::BTreeMap;

const DEFAULT_MAXIMUM_BYTES: u64 = 2 * 1024 * 1024 * 1024;

#[cfg(any(feature = "studio", test))]
fn filename(name: &str) -> Result<()> {
    if name.is_empty()
        || name.contains(['/', '\\'])
        || name == "."
        || name == ".."
        || name.chars().any(char::is_control)
        || !std::path::Path::new(name)
            .components()
            .all(|part| matches!(part, std::path::Component::Normal(_)))
    {
        bail!("Forecast checkpoint asset must be a filename: {name}");
    }
    Ok(())
}

#[cfg(any(feature = "studio", test))]
fn checkpoint_filename(name: &str) -> Result<()> {
    filename(name)?;
    if name != "config.json"
        && name != "LICENSE"
        && name != "NOTICE"
        && name != "model.safetensors.index.json"
        && !name.ends_with(".safetensors")
    {
        bail!(
            "Forecast checkpoint asset must be config.json, LICENSE, NOTICE, a SafeTensors index or SafeTensors weights: {name}"
        );
    }
    Ok(())
}

#[cfg(any(feature = "studio", test))]
fn resolve_assets(
    root: &Bit,
    parameters: &ForecastingModelParameters,
    pack: &BitPack,
    maximum_bytes: u64,
) -> Result<Vec<(String, Bit)>> {
    if root.bit_type != BitTypes::Forecasting {
        bail!("Load Forecasting Bit requires a Forecasting Bit");
    }
    parameters.validate()?;
    if maximum_bytes == 0 {
        bail!("Forecast checkpoint requires a positive byte limit");
    }
    let mut files = BTreeMap::<String, Bit>::new();
    if let Some(name) = &root.file_name {
        checkpoint_filename(name)?;
        files.insert(name.clone(), root.clone());
    }
    for asset in &parameters.assets {
        checkpoint_filename(&asset.relative_path)?;
        let mut matched = Vec::new();
        for bit in &pack.bits {
            if asset.matches_reference(bit, &root.hub)? {
                matched.push(bit);
            }
        }
        let mut candidates = matched.into_iter();
        let bit = candidates.next().ok_or_else(|| {
            anyhow!(
                "Forecast asset {} for {} was not found in the Bit pack",
                asset.bit,
                asset.relative_path
            )
        })?;
        if candidates.any(|other| {
            other.hub != bit.hub
                || other.id != bit.id
                || other.hash != bit.hash
                || other.file_name != bit.file_name
        }) {
            bail!(
                "Forecast asset reference {} is ambiguous; include its hub",
                asset.bit
            );
        }
        if let Some(previous) = files.get(&asset.relative_path) {
            if previous.hub == bit.hub
                && previous.id == bit.id
                && previous.hash == bit.hash
                && previous.file_name == bit.file_name
            {
                continue;
            }
            bail!(
                "More than one Bit supplies forecast asset {}",
                asset.relative_path
            );
        }
        files.insert(asset.relative_path.clone(), bit.clone());
    }
    if !files.contains_key("config.json") {
        bail!("Forecasting Bit must map config.json in parameters.assets");
    }
    let single = files.contains_key("model.safetensors");
    let sharded = files.contains_key("model.safetensors.index.json");
    if single == sharded {
        bail!(
            "Forecasting Bit must supply either model.safetensors or model.safetensors.index.json"
        );
    }
    let mut total = 0_u64;
    for (name, bit) in &files {
        filename(&bit.hash)?;
        filename(
            bit.file_name
                .as_deref()
                .ok_or_else(|| anyhow!("Forecast asset {} has no file_name", bit.id))?,
        )?;
        let size = bit
            .size
            .filter(|size| *size > 0)
            .ok_or_else(|| anyhow!("Forecast asset {} needs its positive size in bytes", bit.id))?;
        let file_limit = match name.as_str() {
            "config.json" => maximum_bytes.min(2 * 1024 * 1024),
            "model.safetensors.index.json" => maximum_bytes.min(8 * 1024 * 1024),
            _ => maximum_bytes,
        };
        total = total
            .checked_add(size)
            .ok_or_else(|| anyhow!("Forecast checkpoint size overflows"))?;
        if size > file_limit || total > maximum_bytes {
            bail!(
                "Forecast checkpoint exceeds the {maximum_bytes} byte limit or its metadata file limits"
            );
        }
    }
    Ok(files.into_iter().collect())
}

/// A Forecasting Bit's checkpoint before anything is staged: the files it consists of and the
/// model family that loads them.
#[cfg(feature = "studio")]
pub struct ForecastCheckpoint {
    pub family: ForecastFamily,
    pub repository: Option<String>,
    pub revision: Option<String>,
    /// Checkpoint file names (`config.json`, weights, index, shards, LICENSE, NOTICE) with the
    /// Bit that supplies each.
    pub files: Vec<(String, Bit)>,
}

/// Resolves `bit`'s asset pack (cached in the Bit store, else fetched from the asset hubs) and
/// maps every checkpoint file within `maximum_bytes`, without downloading anything.
#[cfg(feature = "studio")]
pub async fn resolve_checkpoint(
    state: std::sync::Arc<FlowLikeState>,
    bit: &Bit,
    maximum_bytes: u64,
) -> Result<ForecastCheckpoint> {
    if bit.bit_type != BitTypes::Forecasting {
        bail!("Load Forecasting Bit requires a Forecasting Bit");
    }
    let parameters: ForecastingModelParameters =
        flow_like_types::json::from_value(bit.parameters.clone())?;
    parameters.validate()?;
    if maximum_bytes == 0 {
        bail!("Forecast checkpoint requires a positive byte limit");
    }
    let pack = bit.pack(state).await?;
    let files = resolve_assets(bit, &parameters, &pack, maximum_bytes)?;
    let family = match parameters.model_type {
        ForecastingModelType::Timestral => ForecastFamily::Timestral,
        ForecastingModelType::Chronos2 => ForecastFamily::Chronos2,
        ForecastingModelType::Toto2 => ForecastFamily::Toto2,
        ForecastingModelType::Timesfm25 => ForecastFamily::Timesfm25,
        ForecastingModelType::TinyTimeMixer => ForecastFamily::TinyTimeMixer,
    };
    Ok(ForecastCheckpoint {
        family,
        repository: parameters.provider.model_id.or(bit.repository.clone()),
        revision: parameters.provider.version,
        files,
    })
}

#[cfg(any(feature = "inference", feature = "forecasting"))]
async fn stage_asset(
    store: &flow_like_storage::files::store::FlowLikeStore,
    bit: &Bit,
    destination: &std::path::Path,
    maximum: u64,
) -> Result<u64> {
    use flow_like_storage::object_store::ObjectStoreExt;
    use futures::StreamExt;
    use tokio::io::AsyncWriteExt;
    let name = bit
        .file_name
        .as_deref()
        .ok_or_else(|| anyhow!("Forecast asset has no filename"))?;
    let path = flow_like_storage::Path::from(bit.hash.as_str()).join(name);
    let object = store.as_generic().get(&path).await?;
    if object.meta.size > maximum || Some(object.meta.size) != bit.size {
        bail!(
            "Cached forecast asset {} has an unexpected size; reinstall this Bit",
            bit.id
        );
    }
    let mut file = tokio::fs::File::create(destination).await?;
    let mut stream = object.into_stream();
    let mut written = 0_u64;
    while let Some(bytes) = stream.next().await {
        let bytes = bytes?;
        written = written
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| anyhow!("Forecast checkpoint size overflows"))?;
        if written > maximum || written > bit.size.unwrap_or(0) {
            bail!("Forecast asset {} exceeds its byte limit", bit.id);
        }
        file.write_all(&bytes).await?;
    }
    file.flush().await?;
    if Some(written) != bit.size {
        bail!("Forecast asset {} was truncated", bit.id);
    }
    Ok(written)
}

#[cfg(any(feature = "inference", feature = "forecasting"))]
pub(crate) struct StagedForecastBit {
    pub directory: tempfile::TempDir,
    pub handle: ForecastModelHandle,
}

#[cfg(any(feature = "inference", feature = "forecasting"))]
async fn stage_remote_assets(
    state: std::sync::Arc<FlowLikeState>,
    files: Vec<(String, Bit)>,
    maximum_bytes: u64,
    callback: flow_like_types::intercom::InterComCallback,
) -> Result<tempfile::TempDir> {
    use flow_like::state::FlowLikeConfig;
    use flow_like::utils::{
        download::download_bit, download_manager::DownloadManager, http::HTTPClient,
    };
    use flow_like_storage::files::store::{FlowLikeStore, local_store::LocalObjectStore};
    use flow_like_storage::object_store::ObjectStoreExt;
    use std::sync::Arc;

    // The worker owns its temporary directory until all download workers finish,
    // including when an HTTP caller disconnects during staging.
    tokio::spawn(async move {
        let source = FlowLikeState::bit_store(&state).await?;
        let directory = tempfile::tempdir()?;
        let downloads = directory.path().join(".downloads");
        tokio::fs::create_dir(&downloads).await?;
        let local = Arc::new(LocalObjectStore::new(downloads.clone())?);
        let mut config = FlowLikeConfig::new();
        config.register_bits_store(FlowLikeStore::Local(local.clone()));
        let mut download_state = FlowLikeState::new(config, HTTPClient::new_without_refetch());
        download_state.http_client = state.http_client.clone();
        download_state.download_manager = Arc::new(flow_like_types::sync::Mutex::new(
            DownloadManager::new_ephemeral(),
        ));
        let download_state = Arc::new(download_state);
        let mut remaining = maximum_bytes;

        for (name, bit) in files {
            let size = bit
                .size
                .filter(|size| *size > 0 && *size <= remaining)
                .ok_or_else(|| {
                    anyhow!("Forecast checkpoint exceeds the {maximum_bytes} byte limit")
                })?;
            let path = flow_like_storage::Path::from(bit.hash.as_str()).join(
                bit.file_name
                    .as_deref()
                    .ok_or_else(|| anyhow!("Forecast asset has no filename"))?,
            );
            let destination = directory.path().join(name);
            let installed = source
                .as_generic()
                .head(&path)
                .await
                .is_ok_and(|metadata| metadata.size == size);
            if installed || bit.download_link.is_none() {
                stage_asset(&source, &bit, &destination, remaining).await?;
            } else {
                // Reuse the Bit downloader's size and checksum validation. This
                // private store never changes routing for other model families.
                let downloaded = download_bit(&bit, download_state.clone(), 3, &callback).await?;
                tokio::fs::rename(local.path_to_filesystem(&downloaded)?, &destination).await?;
            }
            remaining -= size;
        }
        tokio::fs::remove_dir_all(downloads).await?;
        Ok(directory)
    })
    .await?
}

#[cfg(any(feature = "inference", feature = "forecasting"))]
pub(crate) async fn stage_bit(
    state: std::sync::Arc<FlowLikeState>,
    bit: &Bit,
    maximum_bytes: u64,
    callback: flow_like_types::intercom::InterComCallback,
) -> Result<StagedForecastBit> {
    let ForecastCheckpoint {
        family,
        repository,
        revision,
        files,
    } = resolve_checkpoint(state.clone(), bit, maximum_bytes).await?;
    let store = FlowLikeState::bit_store(&state).await?;
    let temporary = if matches!(
        store,
        flow_like_storage::files::store::FlowLikeStore::Local(_)
    ) {
        let asset_pack = BitPack {
            bits: files.iter().map(|(_, bit)| bit.clone()).collect(),
        };
        if !asset_pack.is_installed(state.clone()).await? {
            asset_pack.download(state.clone(), callback).await?;
        }
        let directory = tempfile::tempdir()?;
        let mut remaining = maximum_bytes;
        for (name, asset) in &files {
            remaining -=
                stage_asset(&store, asset, &directory.path().join(name), remaining).await?;
        }
        directory
    } else {
        stage_remote_assets(state, files, maximum_bytes, callback).await?
    };
    let handle = ForecastModelHandle {
        cache_key: flow_like_types::create_id(),
        family,
        bit_id: Some(bit.id.clone()),
        repository,
        revision,
    };
    Ok(StagedForecastBit {
        directory: temporary,
        handle,
    })
}

#[cfg(any(feature = "inference", feature = "forecasting"))]
async fn load_bit(
    context: &ExecutionContext,
    bit: Bit,
    maximum_bytes: u64,
) -> Result<ForecastModelHandle> {
    let staged = stage_bit(
        context.app_state.clone(),
        &bit,
        maximum_bytes,
        context.callback().clone(),
    )
    .await?;
    let handle = staged.handle;
    let family = handle.family;
    let loaded = tokio::task::spawn_blocking(move || {
        flow_like_ml_forecast::load_cpu(family, staged.directory.path())
    })
    .await??;
    super::forecasting::cache_model(context, loaded, handle).await
}

#[crate::register_node]
#[derive(Default)]
pub struct LoadForecastingBitNode;

#[flow_like_types::async_trait]
impl NodeLogic for LoadForecastingBitNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ml_load_forecasting_bit",
            "Load Forecasting Bit",
            "Load a Forecasting Bit and its checkpoint assets for native Rust inference",
            "AI/ML/Forecasting",
        );
        node.set_flowscript_name("inspection", "loadForecastingBit");
        node.add_icon("/flow/icons/chart-network.svg");
        node.set_long_running(true);
        node.add_input_pin(
            "exec_in",
            "Input",
            "Load the forecast model",
            VariableType::Execution,
        );
        node.add_input_pin(
            "bit",
            "Model Bit",
            "A Forecasting Bit from the model catalog",
            VariableType::Struct,
        )
        .set_schema::<Bit>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "maximum_bytes",
            "Maximum Bytes",
            "Maximum checkpoint bytes to stage for inference",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(DEFAULT_MAXIMUM_BYTES)));
        node.add_output_pin(
            "exec_out",
            "Done",
            "The model is ready",
            VariableType::Execution,
        );
        node.add_output_pin(
            "model",
            "Model",
            "Loaded forecast model for this execution",
            VariableType::Struct,
        )
        .set_schema::<ForecastModelHandle>();
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        #[cfg(any(feature = "inference", feature = "forecasting"))]
        {
            context.deactivate_exec_pin("exec_out").await?;
            let bit = context.evaluate_pin("bit").await?;
            let maximum_bytes = context.evaluate_pin("maximum_bytes").await?;
            let model = load_bit(context, bit, maximum_bytes).await?;
            context.set_pin_value("model", json!(model)).await?;
            context.activate_exec_pin("exec_out").await?;
            Ok(())
        }
        #[cfg(not(any(feature = "inference", feature = "forecasting")))]
        {
            let _ = context;
            Err(super::published::inference_unavailable())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(id: &str, name: &str) -> Bit {
        Bit {
            id: id.into(),
            hub: "models.example".into(),
            bit_type: BitTypes::File,
            file_name: Some(name.into()),
            hash: format!("hash-{id}"),
            size: Some(16),
            ..Bit::default()
        }
    }

    fn fixture() -> (Bit, ForecastingModelParameters, BitPack) {
        let config = asset("config", "upstream-config.json");
        let root = Bit {
            bit_type: BitTypes::Forecasting,
            parameters: json!({
                "model_type": "timestral",
                "provider": {"provider_name":"local:forecast"},
                "assets": [{"bit":"models.example:config", "relative_path":"config.json"}]
            }),
            ..asset("forecast", "model.safetensors")
        };
        let parameters = flow_like_types::json::from_value(root.parameters.clone()).unwrap();
        let pack = BitPack {
            bits: vec![root.clone(), config],
        };
        (root, parameters, pack)
    }

    #[test]
    fn bit_assets_resolve_from_separate_content_addressed_files() {
        let (root, parameters, pack) = fixture();
        let files = resolve_assets(&root, &parameters, &pack, 32).unwrap();
        assert_eq!(
            files
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>(),
            ["config.json", "model.safetensors"]
        );
        assert_eq!(
            files[0].1.file_name.as_deref(),
            Some("upstream-config.json")
        );
        assert!(resolve_assets(&root, &parameters, &pack, 31).is_err());
    }

    #[test]
    fn checkpoint_license_and_notice_are_retained_and_counted_in_the_download_budget() {
        let (root, mut parameters, mut pack) = fixture();
        for name in ["LICENSE", "NOTICE"] {
            parameters.assets.push(flow_like::bit::ForecastingAssetRef {
                bit: format!("models.example:{name}"),
                relative_path: name.into(),
            });
            pack.bits.push(asset(name, name));
        }
        parameters.validate().unwrap();
        let files = resolve_assets(&root, &parameters, &pack, 64).unwrap();
        assert_eq!(files.len(), 4);
        assert!(files.iter().any(|(name, _)| name == "LICENSE"));
        assert!(files.iter().any(|(name, _)| name == "NOTICE"));
        assert!(resolve_assets(&root, &parameters, &pack, 63).is_err());
    }

    #[test]
    fn bit_assets_reject_missing_ambiguous_and_unsafe_mappings() {
        let (root, mut parameters, mut pack) = fixture();
        parameters.assets[0].relative_path = "../config.json".into();
        assert!(resolve_assets(&root, &parameters, &pack, 1024).is_err());
        parameters.assets[0].relative_path = "config.json".into();
        parameters.assets[0].bit = "absent".into();
        assert!(resolve_assets(&root, &parameters, &pack, 1024).is_err());
        parameters.assets[0].bit = "config".into();
        let mut other = asset("config", "config.json");
        other.hash = "conflicting-config".into();
        pack.bits.push(other);
        assert!(resolve_assets(&root, &parameters, &pack, 1024).is_err());
        parameters.assets[0].bit = "models.example:config".into();
        pack.bits[1].hash = "../private".into();
        assert!(resolve_assets(&root, &parameters, &pack, 1024).is_err());
    }

    #[test]
    fn bit_assets_require_exactly_one_checkpoint_layout() {
        let (mut root, parameters, mut pack) = fixture();
        root.file_name = None;
        assert!(resolve_assets(&root, &parameters, &pack, 1024).is_err());
        root.file_name = Some("model.safetensors.index.json".into());
        assert!(resolve_assets(&root, &parameters, &pack, 1024).is_ok());
        let mut parameters = parameters;
        parameters.assets.push(flow_like::bit::ForecastingAssetRef {
            bit: "weights".into(),
            relative_path: "model.safetensors".into(),
        });
        pack.bits.push(asset("weights", "model.safetensors"));
        assert!(resolve_assets(&root, &parameters, &pack, 1024).is_err());
    }

    #[cfg(any(feature = "inference", feature = "forecasting"))]
    #[tokio::test]
    async fn staging_preserves_bytes_and_enforces_cached_size_and_budget() {
        use flow_like_storage::files::store::{FlowLikeStore, local_store::LocalObjectStore};
        let directory = tempfile::tempdir().unwrap();
        let destination = tempfile::tempdir().unwrap();
        let bit = asset("weights", "model.safetensors");
        let parent = directory.path().join(&bit.hash);
        std::fs::create_dir(&parent).unwrap();
        let bytes = [1_u8; 16];
        std::fs::write(parent.join("model.safetensors"), bytes).unwrap();
        let store = FlowLikeStore::Local(std::sync::Arc::new(
            LocalObjectStore::new(directory.path().to_path_buf()).unwrap(),
        ));
        let path = destination.path().join("model.safetensors");
        assert_eq!(stage_asset(&store, &bit, &path, 16).await.unwrap(), 16);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert!(stage_asset(&store, &bit, &path, 15).await.is_err());
        let wrong_size = Bit {
            size: Some(17),
            ..bit
        };
        assert!(stage_asset(&store, &wrong_size, &path, 32).await.is_err());
    }

    #[cfg(any(feature = "inference", feature = "forecasting"))]
    mod remote_staging {
        use super::*;
        use flow_like::{
            state::FlowLikeConfig,
            utils::{compression::compress_to_file_json, http::HTTPClient},
        };
        use flow_like_storage::{
            Path, blake3,
            files::store::FlowLikeStore,
            object_store::{ObjectStoreExt, memory::InMemory},
        };
        use std::{
            io::{BufRead, BufReader, Write},
            net::TcpListener,
            sync::{
                Arc,
                atomic::{AtomicBool, AtomicUsize, Ordering},
            },
            thread::JoinHandle,
            time::Duration,
        };

        struct ArtifactServer {
            url: String,
            gets: Arc<AtomicUsize>,
            stop: Arc<AtomicBool>,
            worker: Option<JoinHandle<()>>,
        }

        impl ArtifactServer {
            fn new(bytes: Vec<u8>) -> Self {
                let listener = TcpListener::bind("127.0.0.1:0").unwrap();
                listener.set_nonblocking(true).unwrap();
                let url = format!(
                    "http://{}/model.safetensors",
                    listener.local_addr().unwrap()
                );
                let gets = Arc::new(AtomicUsize::new(0));
                let stop = Arc::new(AtomicBool::new(false));
                let worker = {
                    let gets = gets.clone();
                    let stop = stop.clone();
                    std::thread::spawn(move || {
                        while !stop.load(Ordering::Relaxed) {
                            let (mut stream, _) = match listener.accept() {
                                Ok(connection) => connection,
                                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                                    std::thread::sleep(Duration::from_millis(5));
                                    continue;
                                }
                                Err(error) => panic!("Artifact server failed: {error}"),
                            };
                            stream.set_nonblocking(false).unwrap();
                            stream
                                .set_read_timeout(Some(Duration::from_secs(5)))
                                .unwrap();
                            let mut reader = BufReader::new(&mut stream);
                            let mut line = String::new();
                            reader.read_line(&mut line).unwrap();
                            let get = line.starts_with("GET ");
                            loop {
                                line.clear();
                                if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                                    break;
                                }
                            }
                            write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", bytes.len()).unwrap();
                            if get {
                                gets.fetch_add(1, Ordering::Relaxed);
                                stream.write_all(&bytes).unwrap();
                            }
                        }
                    })
                };
                Self {
                    url,
                    gets,
                    stop,
                    worker: Some(worker),
                }
            }
        }

        impl Drop for ArtifactServer {
            fn drop(&mut self) {
                self.stop.store(true, Ordering::Relaxed);
                let result = self.worker.take().unwrap().join();
                if !std::thread::panicking() {
                    result.unwrap();
                }
            }
        }

        async fn remote_fixture(url: String, expected: &[u8]) -> (Arc<FlowLikeState>, Bit) {
            let (mut root, _, mut pack) = fixture();
            root.hash = blake3::hash(expected).to_hex().to_string();
            root.size = Some(expected.len() as u64);
            root.download_link = Some(url);
            pack.bits[0] = root.clone();
            let store = FlowLikeStore::Memory(Arc::new(InMemory::new()));
            let config_bit = &pack.bits[1];
            store
                .as_generic()
                .put(
                    &Path::from(config_bit.hash.as_str())
                        .join(config_bit.file_name.as_deref().unwrap()),
                    vec![2_u8; 16].into(),
                )
                .await
                .unwrap();
            compress_to_file_json(
                store.as_generic(),
                Path::from("deps-cache/bit-deps-forecast.bin"),
                &pack,
            )
            .await
            .unwrap();
            let mut config = FlowLikeConfig::new();
            config.register_bits_store(store);
            (
                Arc::new(FlowLikeState::new(
                    config,
                    HTTPClient::new_without_refetch(),
                )),
                root,
            )
        }

        #[tokio::test]
        async fn remote_store_downloads_only_missing_assets_without_changing_model_routing() {
            let bytes = vec![7_u8; 16];
            let server = ArtifactServer::new(bytes.clone());
            let (state, root) = remote_fixture(server.url.clone(), &bytes).await;
            assert!(!FlowLikeState::can_execute_local_bit_models(&state).await);
            let staged = stage_bit(state.clone(), &root, 32, None).await.unwrap();
            assert_eq!(
                std::fs::read(staged.directory.path().join("model.safetensors")).unwrap(),
                bytes
            );
            assert_eq!(
                std::fs::read(staged.directory.path().join("config.json")).unwrap(),
                vec![2_u8; 16]
            );
            assert!(!staged.directory.path().join(".downloads").exists());
            assert_eq!(server.gets.load(Ordering::Relaxed), 1);
            assert!(!FlowLikeState::can_execute_local_bit_models(&state).await);
            let store = FlowLikeState::bit_store(&state).await.unwrap();
            assert!(matches!(store, FlowLikeStore::Memory(_)));
            assert!(
                store
                    .as_generic()
                    .head(&Path::from(root.hash.as_str()).join("model.safetensors"))
                    .await
                    .is_err()
            );
            let staged_path = staged.directory.path().to_path_buf();
            drop(staged);
            assert!(!staged_path.exists());
        }

        #[tokio::test]
        async fn remote_staging_rejects_budget_before_download_and_checks_the_content_hash() {
            let server = ArtifactServer::new(vec![9_u8; 16]);
            let (state, root) = remote_fixture(server.url.clone(), &[7_u8; 16]).await;
            let error = stage_bit(state.clone(), &root, 31, None)
                .await
                .err()
                .unwrap();
            assert!(error.to_string().contains("byte limit"));
            assert_eq!(server.gets.load(Ordering::Relaxed), 0);
            let error = stage_bit(state, &root, 32, None).await.err().unwrap();
            assert!(error.to_string().to_lowercase().contains("hash"), "{error}");
            assert!(server.gets.load(Ordering::Relaxed) > 0);
        }

        #[tokio::test]
        async fn remote_staging_rejects_unexpected_download_size_before_fetching_the_body() {
            let server = ArtifactServer::new(vec![7_u8; 17]);
            let (state, root) = remote_fixture(server.url.clone(), &[7_u8; 16]).await;
            let error = stage_bit(state, &root, 32, None).await.err().unwrap();
            assert!(
                error.to_string().contains("Refusing to download"),
                "{error}"
            );
            assert_eq!(server.gets.load(Ordering::Relaxed), 0);
        }
    }
}

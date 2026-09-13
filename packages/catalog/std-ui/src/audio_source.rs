use flow_like::flow::execution::context::ExecutionContext;
use flow_like_catalog_core::FlowPath;
use flow_like_storage::{
    Path,
    files::store::FlowLikeStore,
    object_store::{GetResult, ObjectStoreExt},
};
use flow_like_types::{Result, anyhow, bail, reqwest::Url};
use futures::StreamExt;
use std::time::Duration;

const MAX_AUDIO_BYTES: usize = 32 * 1024 * 1024;
const MAX_URL_BYTES: usize = 8192;

pub(crate) async fn resolve_audio_source(
    context: &mut ExecutionContext,
    url: String,
    path: Option<FlowPath>,
    timeout: Duration,
) -> Result<String> {
    let url = url.trim();
    let local = context.execution_environment().is_local();
    match (url.is_empty(), path) {
        (false, None) => validate_audio_url(url, local),
        (true, Some(path)) => resolve_audio_path(context, path, timeout, local).await,
        _ => bail!("Provide exactly one Audio Path or Audio URL"),
    }
}

fn validate_audio_url(value: &str, local: bool) -> Result<String> {
    if value.is_empty() || value.len() > MAX_URL_BYTES || value.chars().any(char::is_control) {
        bail!("Audio URL must contain 1 to 8192 bytes without control characters");
    }
    let url = Url::parse(value).map_err(|_| anyhow!("Audio URL is not a valid absolute URL"))?;
    if !url.username().is_empty() || url.password().is_some() {
        bail!("Audio URL must not include a username or password");
    }
    let client_host = url.host_str().is_some_and(|host| {
        matches!(
            host.trim_end_matches('.'),
            "asset.localhost" | "tauri.localhost"
        )
    });
    let allowed = match url.scheme() {
        "http" | "https" => url.host_str().is_some() && (local || !client_host),
        "asset" => local && url.host_str() == Some("localhost"),
        "blob" => local,
        _ => false,
    };
    if !allowed {
        bail!("Audio URL must use HTTP(S), or a local asset/blob URL during local execution");
    }
    Ok(url.to_string())
}

async fn playable_store_url(
    store: &FlowLikeStore,
    path: &Path,
    ttl: Duration,
    local: bool,
) -> Result<Option<String>> {
    let url = match store {
        FlowLikeStore::AWS(_) | FlowLikeStore::Google(_) | FlowLikeStore::Azure(_) => {
            store.sign("GET", path, ttl).await?.to_string()
        }
        FlowLikeStore::Local(store) if local => {
            let path = store.path_to_filesystem(path)?;
            let path = path
                .to_str()
                .ok_or_else(|| anyhow!("Audio file path is not valid UTF-8"))?;
            #[cfg(any(windows, target_os = "android"))]
            let base = "http://asset.localhost/";
            #[cfg(not(any(windows, target_os = "android")))]
            let base = "asset://localhost/";
            format!("{base}{}", urlencoding::encode(path))
        }
        _ => return Ok(None),
    };
    validate_audio_url(&url, local).map(Some)
}

async fn read_bounded_audio(result: GetResult) -> Result<Vec<u8>> {
    if result.meta.size > MAX_AUDIO_BYTES as u64 {
        bail!("Audio file exceeds the 32 MiB temporary playback limit");
    }
    let mut stream = result.into_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if bytes.len().saturating_add(chunk.len()) > MAX_AUDIO_BYTES {
            bail!("Audio file exceeds the 32 MiB temporary playback limit");
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

async fn resolve_audio_path(
    context: &mut ExecutionContext,
    path: FlowPath,
    timeout: Duration,
    local: bool,
) -> Result<String> {
    let runtime = path.to_runtime(context).await?;
    let ttl = timeout.saturating_add(Duration::from_secs(60));
    if let Some(url) = playable_store_url(&runtime.store, &runtime.path, ttl, local).await? {
        return Ok(url);
    }

    let temporary = FlowPath::from_cache_dir(context, false, false)
        .await
        .map_err(|_| anyhow!("Temporary storage is unavailable for audio playback. Provide a reachable HTTP(S) URL or configure temporary file storage"))?;
    let mut destination = temporary.to_runtime(context).await?;
    let extension = runtime.path.extension().unwrap_or_default();
    let extension =
        if extension.len() <= 16 && extension.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
            extension
        } else {
            ""
        };
    let filename = if extension.is_empty() {
        format!("playback-{}", uuid::Uuid::now_v7())
    } else {
        format!("playback-{}.{}", uuid::Uuid::now_v7(), extension)
    };
    destination.path = destination.path.join(filename);

    // Validate the destination before reading audio. Memory stores and remote local
    // disks cannot provide a URL that the invoking frontend can load.
    playable_store_url(&destination.store, &destination.path, ttl, local)
        .await?
        .ok_or_else(|| anyhow!("Temporary audio storage cannot provide a playable URL. Remote playback requires HTTP(S) file storage"))?;
    let result = runtime.store.as_generic().get(&runtime.path).await?;
    let bytes = read_bounded_audio(result).await?;
    destination.store.put(&destination.path, bytes).await?;
    // Start the download lifetime after the upload, which may itself take time.
    playable_store_url(&destination.store, &destination.path, ttl, local)
        .await?
        .ok_or_else(|| anyhow!("Temporary audio storage no longer provides a playable URL"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_storage::{
        files::store::local_store::LocalObjectStore, object_store::memory::InMemory,
    };
    use std::sync::Arc;

    #[test]
    fn urls_require_supported_schemes_and_same_device_for_local_sources() {
        for value in [
            "data:audio/wav;base64,AAAA",
            "file:///tmp/speech.wav",
            "javascript:alert(1)",
            "/tmp/speech.wav",
            "https://user:secret@example.com/speech.mp3",
            "https://example.com/a\n.wav",
            "asset://another-host/speech.wav",
        ] {
            assert!(validate_audio_url(value, true).is_err(), "{value}");
        }
        for value in [
            "asset://localhost/tmp/speech.wav",
            "http://asset.localhost/speech.wav",
            "https://tauri.localhost/speech.wav",
            "https://ASSET.localhost./speech.wav",
            "blob:https://example.com/recording",
        ] {
            assert!(validate_audio_url(value, false).is_err(), "{value}");
            assert!(validate_audio_url(value, true).is_ok(), "{value}");
        }
        assert!(validate_audio_url("https://files.example/speech.wav?token=signed", false).is_ok());
        assert!(
            validate_audio_url(&format!("https://example.com/{}", "é".repeat(4096)), false)
                .is_err()
        );
    }

    #[tokio::test]
    async fn local_store_urls_are_assets_without_inlining_and_remote_requires_copy() {
        let directory =
            std::env::temp_dir().join(format!("flow-like-playback-{}", uuid::Uuid::now_v7()));
        let store =
            FlowLikeStore::Local(Arc::new(LocalObjectStore::new(directory.clone()).unwrap()));
        let path = Path::from("speech with spaces.wav");
        store
            .put(&path, b"RIFF small audio fixture".to_vec())
            .await
            .unwrap();
        let url = playable_store_url(&store, &path, Duration::from_secs(180), true)
            .await
            .unwrap()
            .unwrap();
        assert!(!url.starts_with("data:"));
        let decoded =
            crate::a2ui::elements::get_file_input_files::decode_local_file_url(&url).unwrap();
        assert_eq!(std::fs::read(decoded).unwrap(), b"RIFF small audio fixture");
        assert!(
            playable_store_url(&store, &path, Duration::from_secs(180), false)
                .await
                .unwrap()
                .is_none()
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[tokio::test]
    async fn memory_sources_are_bounded_before_materialization() {
        let store = FlowLikeStore::Memory(Arc::new(InMemory::new()));
        let path = Path::from("speech.wav");
        store
            .put(&path, vec![1_u8; MAX_AUDIO_BYTES + 1])
            .await
            .unwrap();
        let result = store.as_generic().get(&path).await.unwrap();
        assert!(
            read_bounded_audio(result)
                .await
                .unwrap_err()
                .to_string()
                .contains("32 MiB")
        );
        let mut result = store.as_generic().get(&path).await.unwrap();
        result.meta.size = 0;
        assert!(
            read_bounded_audio(result)
                .await
                .unwrap_err()
                .to_string()
                .contains("32 MiB")
        );
        store.put(&path, b"RIFF audio".to_vec()).await.unwrap();
        let mut result = store.as_generic().get(&path).await.unwrap();
        result.meta.size = 0;
        assert_eq!(read_bounded_audio(result).await.unwrap(), b"RIFF audio");
        assert!(
            playable_store_url(&store, &path, Duration::from_secs(180), true)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn cloud_sources_get_short_lived_http_urls_without_reading_the_object() {
        let store = flow_like_storage::object_store::aws::AmazonS3Builder::new()
            .with_bucket_name("audio")
            .with_region("us-east-1")
            .with_access_key_id("test-key")
            .with_secret_access_key("test-secret")
            .build()
            .unwrap();
        let url = playable_store_url(
            &FlowLikeStore::AWS(Arc::new(store)),
            &Path::from("speech.mp3"),
            Duration::from_secs(660),
            false,
        )
        .await
        .unwrap()
        .unwrap();
        let url = Url::parse(&url).unwrap();
        assert_eq!(url.scheme(), "https");
        assert!(url.path().ends_with("speech.mp3"));
        assert!(
            url.query_pairs()
                .any(|(key, value)| key.eq_ignore_ascii_case("X-Amz-Expires") && value == "660")
        );
    }

    #[cfg(feature = "execute")]
    async fn execution_context(temporary_store: Option<FlowLikeStore>) -> ExecutionContext {
        use ahash::AHashMap;
        use flow_like::{
            flow::{
                board::ExecutionStage,
                execution::{
                    LogLevel, context::ExecutionContextCache, internal_node::InternalNode,
                },
                node::NodeLogic,
            },
            profile::Profile,
            state::{FlowLikeConfig, FlowLikeState, FlowLikeStores},
            utils::http::HTTPClient,
        };
        use flow_like_types::sync::{Mutex, RwLock};
        use std::sync::Weak;
        let logic: Arc<dyn NodeLogic> =
            Arc::new(crate::a2ui::elements::set_media_source::SetMediaSource);
        let current = Arc::new(InternalNode::new(
            logic.get_node(),
            AHashMap::new(),
            logic,
            AHashMap::new(),
        ));
        let mut context = ExecutionContext::new(
            Arc::new(AHashMap::new()),
            &Weak::new(),
            &Arc::new(FlowLikeState::new(
                FlowLikeConfig::new(),
                HTTPClient::new_without_refetch(),
            )),
            &current,
            &Arc::new(Mutex::new(AHashMap::new())),
            &Arc::new(RwLock::new(AHashMap::new())),
            LogLevel::Debug,
            ExecutionStage::Dev,
            Arc::new(Profile::default()),
            None,
            Arc::new(RwLock::new(Vec::new())),
            None,
            None,
            Arc::new(AHashMap::new()),
            None,
        )
        .await;
        context.execution_cache = Some(ExecutionContextCache {
            stores: FlowLikeStores {
                temporary_store,
                ..FlowLikeStores::default()
            },
            app_id: "audio-app".into(),
            model_usage_app_id: None,
            board_dir: Path::from("board"),
            board_id: "board".into(),
            node_id: current.shared_node_id(),
            sub: "user".into(),
            shadow: false,
        });
        context
    }

    #[cfg(feature = "execute")]
    #[tokio::test]
    async fn resolver_materializes_memory_audio_as_unique_playable_files_with_its_extension() {
        let directory =
            std::env::temp_dir().join(format!("flow-like-playback-{}", uuid::Uuid::now_v7()));
        let destination =
            FlowLikeStore::Local(Arc::new(LocalObjectStore::new(directory.clone()).unwrap()));
        let source = FlowLikeStore::Memory(Arc::new(InMemory::new()));
        let path = Path::from("generated/speech.wav");
        source
            .put(&path, b"RIFF speech bytes".to_vec())
            .await
            .unwrap();
        let mut context = execution_context(Some(destination)).await;
        context.set_cache("audio-source", Arc::new(source)).await;
        let path = FlowPath::new(path.to_string(), "audio-source".into(), None);
        let mut urls = Vec::new();
        for _ in 0..2 {
            let url = resolve_audio_source(
                &mut context,
                String::new(),
                Some(path.clone()),
                Duration::from_secs(300),
            )
            .await
            .unwrap();
            let file =
                crate::a2ui::elements::get_file_input_files::decode_local_file_url(&url).unwrap();
            assert_eq!(std::path::Path::new(&file).extension().unwrap(), "wav");
            assert_eq!(std::fs::read(file).unwrap(), b"RIFF speech bytes");
            urls.push(url);
        }
        assert_ne!(urls[0], urls[1]);
        assert!(
            resolve_audio_source(
                &mut context,
                urls[0].clone(),
                Some(path),
                Duration::from_secs(300)
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("exactly one")
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(feature = "execute")]
    #[tokio::test]
    async fn resolver_rejects_unreachable_temporary_storage_before_reading_source_audio() {
        let source = FlowLikeStore::Memory(Arc::new(InMemory::new()));
        let path = FlowPath::new("missing-source.wav".into(), "audio-source".into(), None);
        let mut context = execution_context(None).await;
        context
            .set_cache("audio-source", Arc::new(source.clone()))
            .await;
        let error = resolve_audio_path(&mut context, path.clone(), Duration::from_secs(300), false)
            .await
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("Temporary storage is unavailable"),
            "{error}"
        );

        let directory =
            std::env::temp_dir().join(format!("flow-like-playback-{}", uuid::Uuid::now_v7()));
        let local =
            FlowLikeStore::Local(Arc::new(LocalObjectStore::new(directory.clone()).unwrap()));
        for destination in [local, FlowLikeStore::Memory(Arc::new(InMemory::new()))] {
            let mut context = execution_context(Some(destination)).await;
            context
                .set_cache("audio-source", Arc::new(source.clone()))
                .await;
            let error =
                resolve_audio_path(&mut context, path.clone(), Duration::from_secs(300), false)
                    .await
                    .unwrap_err()
                    .to_string();
            assert!(
                error.contains("Remote playback requires HTTP(S) file storage"),
                "{error}"
            );
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
}

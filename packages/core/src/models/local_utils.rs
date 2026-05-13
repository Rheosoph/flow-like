use crate::{bit::BitPack, state::FlowLikeState};
use flow_like_storage::files::store::FlowLikeStore;
use flow_like_types::reqwest;
use flow_like_types::tokio::{fs as async_fs, time};
use flow_like_types::{Result, anyhow};
use std::{sync::Arc, time::Duration};

/// Ensures that local model weights are available before loading a model.
/// Attempts to redownload missing files, but falls back to cached weights when present.
pub async fn ensure_local_weights(
    pack: &BitPack,
    app_state: &Arc<FlowLikeState>,
    bit_id: &str,
    model_kind: &str,
) -> Result<()> {
    let was_installed = is_pack_installed(pack, app_state).await;
    if was_installed {
        let refresh_needed = should_refresh_pack(pack, app_state).await?;
        if !refresh_needed {
            return Ok(());
        }
    }

    if let Err(err) = pack.download(app_state.clone(), None).await {
        if was_installed || is_pack_installed(pack, app_state).await {
            println!(
                "Failed to refresh {} {}. Using cached weights instead. Error: {}",
                model_kind, bit_id, err
            );
            return Ok(());
        }
        return Err(err);
    }

    if is_pack_installed(pack, app_state).await {
        return Ok(());
    }

    let missing = missing_local_artifacts(pack, app_state).await?;
    if missing.is_empty() {
        println!(
            "{} {} has cached files with metadata mismatches; continuing with local artifacts.",
            model_kind, bit_id
        );
        return Ok(());
    }

    Err(anyhow!(
        "Local cache for {} {} is unavailable. Missing artifacts: {}",
        model_kind,
        bit_id,
        missing.join(", ")
    ))
}

async fn should_refresh_pack(pack: &BitPack, app_state: &Arc<FlowLikeState>) -> Result<bool> {
    let store = FlowLikeState::bit_store(app_state).await?;
    let FlowLikeStore::Local(local_store) = store else {
        return Ok(false);
    };

    let client = app_state.http_client.client();
    let timeout = Duration::from_millis(1200);

    for bit in &pack.bits {
        let download_link = match &bit.download_link {
            Some(link) => link,
            None => continue,
        };

        let local_path = match bit.to_path(&local_store) {
            Some(path) => path,
            None => continue,
        };

        let local_size = async_fs::metadata(&local_path)
            .await
            .ok()
            .map(|meta| meta.len())
            .unwrap_or(0);

        let remote_size = match quick_remote_size(&client, download_link, timeout).await? {
            Some(size) => size,
            None => return Ok(false),
        };

        if local_size == 0 || remote_size != local_size {
            return Ok(true);
        }
    }

    Ok(false)
}

async fn quick_remote_size(
    client: &reqwest::Client,
    url: &str,
    timeout: Duration,
) -> Result<Option<u64>> {
    let response = match time::timeout(timeout, client.head(url).send()).await {
        Ok(Ok(response)) => response,
        Ok(Err(_)) => return Ok(None),
        Err(_) => return Ok(None),
    };

    if response.status().is_server_error() {
        return Ok(None);
    }

    let size = response
        .headers()
        .get("content-length")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());

    Ok(size)
}

async fn is_pack_installed(pack: &BitPack, app_state: &Arc<FlowLikeState>) -> bool {
    pack.is_installed(app_state.clone()).await.unwrap_or(false)
}

async fn missing_local_artifacts(
    pack: &BitPack,
    app_state: &Arc<FlowLikeState>,
) -> Result<Vec<String>> {
    let store = FlowLikeState::bit_store(app_state).await?;
    let FlowLikeStore::Local(local_store) = store else {
        return Ok(pack
            .bits
            .iter()
            .map(|bit| format!("{} (no local store configured)", bit.id))
            .collect());
    };

    let mut missing = Vec::new();
    for bit in &pack.bits {
        if bit.download_link.is_none() {
            continue;
        }

        let path = match bit.to_path(&local_store) {
            Some(path) => path,
            None => {
                missing.push(format!("{} (no path)", bit.id));
                continue;
            }
        };

        if !async_fs::try_exists(&path).await.unwrap_or(false) {
            missing.push(format!("{} ({})", bit.id, path.display()));
        }
    }

    Ok(missing)
}

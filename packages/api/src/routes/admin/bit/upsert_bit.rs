use std::sync::Arc;

use crate::{
    entity::bit, error::ApiError, middleware::jwt::AppUser,
    permission::global_permission::GlobalPermission, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
    response::sse::{Event, KeepAlive, Sse},
};
use flow_like::{bit::Bit, utils::http::HTTPClient};
use flow_like_storage::object_store::PutPayload;
use flow_like_types::{
    Bytes,
    tokio::{self, sync::mpsc},
};
use flow_like_types::{create_id, reqwest};
use futures_util::StreamExt;
use futures_util::stream::{self, Stream};
use hyper::header::{ACCEPT_RANGES, CONTENT_LENGTH, ETAG};
use sea_orm::{ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter};
use serde::Serialize;
use serde_json::json;
use std::convert::Infallible;
use std::time::Duration;

#[derive(Serialize, Clone)]
struct Progress {
    stage: &'static str,
    message: Option<String>,
    downloaded: Option<u64>,
    total: Option<u64>,
    percent: Option<f32>,
    hash: Option<String>,
}

enum StreamMsg {
    Progress(Progress),
    Done(Bit),
    Error(String),
}

#[tracing::instrument(name = "PUT /admin/bit/{bit_id}", skip(state, user, bit))]
pub async fn upsert_bit(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bit_id): Path<String>,
    Json(bit): Json<Bit>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::WriteBits)
        .await?;

    let (tx, rx) = mpsc::channel::<StreamMsg>(64);
    let state_cloned = state.clone();
    let bit_id_cloned = bit_id.clone();

    tokio::spawn(async move {
        let mut model: bit::Model = bit.into();
        match bit::Entity::find_by_id(&bit_id_cloned)
            .one(&state_cloned.db)
            .await
        {
            Ok(Some(existing_bit)) => {
                let mut updated_bit: bit::ActiveModel = existing_bit.into();
                if updated_bit.download_link != Set(model.download_link.clone()) {
                    let _ = tx
                        .send(StreamMsg::Progress(Progress {
                            stage: "start",
                            message: Some("downloading".into()),
                            downloaded: None,
                            total: None,
                            percent: None,
                            hash: None,
                        }))
                        .await;
                    if let Err(e) =
                        download_and_hash(&mut model, state_cloned.clone(), Some(tx.clone())).await
                    {
                        let _ = tx.send(StreamMsg::Error(e.to_string())).await;
                        return;
                    }
                    if let Err(e) =
                        build_dependency_hash(&mut model, state_cloned.clone(), Some(tx.clone()))
                            .await
                    {
                        let _ = tx.send(StreamMsg::Error(e.to_string())).await;
                        return;
                    }
                    updated_bit.download_link = Set(model.download_link.clone());
                    updated_bit.hash = Set(model.hash.clone());
                    updated_bit.dependency_tree_hash = Set(model.dependency_tree_hash.clone());
                }
                updated_bit.hub = Set(state_cloned.platform_config.domain.clone());
                updated_bit.authors = Set(model.authors);
                updated_bit.updated_at = Set(chrono::Utc::now().naive_utc());
                updated_bit.dependencies = Set(model.dependencies);
                updated_bit.file_name = Set(model.file_name);
                updated_bit.hub = Set(model.hub);
                updated_bit.license = Set(model.license);
                updated_bit.parameters = Set(model.parameters);
                updated_bit.repository = Set(model.repository);
                updated_bit.size = Set(model.size);
                updated_bit.r#type = Set(model.r#type);
                updated_bit.version = Set(model.version);
                updated_bit.model_slug = Set(model.model_slug);
                match updated_bit.update(&state_cloned.db).await {
                    Ok(updated) => {
                        let _ = tx.send(StreamMsg::Done(Bit::from(updated))).await;
                    }
                    Err(e) => {
                        let _ = tx.send(StreamMsg::Error(e.to_string())).await;
                    }
                }
            }
            Ok(None) => {
                let _ = tx
                    .send(StreamMsg::Progress(Progress {
                        stage: "start",
                        message: Some("downloading".into()),
                        downloaded: None,
                        total: None,
                        percent: None,
                        hash: None,
                    }))
                    .await;
                if let Err(e) =
                    download_and_hash(&mut model, state_cloned.clone(), Some(tx.clone())).await
                {
                    let _ = tx.send(StreamMsg::Error(e.to_string())).await;
                    return;
                }
                if let Err(e) =
                    build_dependency_hash(&mut model, state_cloned.clone(), Some(tx.clone())).await
                {
                    let _ = tx.send(StreamMsg::Error(e.to_string())).await;
                    return;
                }
                let dependency_tree_hash = model.dependency_tree_hash.clone();
                let mut new_bit: bit::ActiveModel = model.into();
                new_bit.id = Set(create_id());
                new_bit.hub = Set(state_cloned.platform_config.domain.clone());
                new_bit.created_at = Set(chrono::Utc::now().naive_utc());
                new_bit.updated_at = Set(chrono::Utc::now().naive_utc());
                match new_bit.insert(&state_cloned.db).await {
                    Ok(inserted) => {
                        let _ = tx.send(StreamMsg::Done(Bit::from(inserted))).await;
                    }
                    Err(_e) => {
                        match bit::Entity::find()
                            .filter(bit::Column::DependencyTreeHash.eq(dependency_tree_hash))
                            .one(&state_cloned.db)
                            .await
                        {
                            Ok(Some(existing_bit)) => {
                                let _ = tx.send(StreamMsg::Done(Bit::from(existing_bit))).await;
                            }
                            Ok(None) => {
                                let _ = tx.send(StreamMsg::Error("Bit with the same dependency tree hash not found after insert error".into())).await;
                            }
                            Err(e) => {
                                let _ = tx.send(StreamMsg::Error(e.to_string())).await;
                            }
                        }
                    }
                }
            }
            Err(e) => {
                let _ = tx.send(StreamMsg::Error(e.to_string())).await;
            }
        }
    });

    let stream = stream::unfold(rx, |mut rx| async move {
        match rx.recv().await {
            Some(StreamMsg::Progress(p)) => {
                let data = serde_json::to_string(&p).unwrap_or_else(|_| "{}".into());
                Some((Ok(Event::default().event("progress").data(data)), rx))
            }
            Some(StreamMsg::Done(bit)) => {
                let data = json!(bit).to_string();
                Some((Ok(Event::default().event("done").data(data)), rx))
            }
            Some(StreamMsg::Error(msg)) => {
                let data = json!({"message": msg}).to_string();
                Some((Ok(Event::default().event("error").data(data)), rx))
            }
            None => None,
        }
    });

    let sse = Sse::new(stream).keep_alive(
        KeepAlive::new()
            .text("keep-alive")
            .interval(Duration::from_secs(15)),
    );
    Ok(sse)
}

#[tracing::instrument(name = "download_and_hash_bit", skip(bit, state))]
async fn download_and_hash(
    bit: &mut bit::Model,
    state: AppState,
    tx: Option<mpsc::Sender<StreamMsg>>,
) -> flow_like_types::Result<()> {
    if bit.download_link.is_none() {
        tracing::info!(
            "No download link provided for bit {}, using ID as hash",
            bit.id
        );
        if bit.hash.as_ref().is_none_or(|h| h.is_empty()) {
            bit.hash = Some(bit.id.clone());
        }
        if bit
            .dependency_tree_hash
            .as_ref()
            .is_none_or(|h| h.is_empty())
        {
            bit.dependency_tree_hash = Some(bit.id.clone());
        }
        return Ok(());
    }

    let store = state.cdn_bucket.clone();

    let old_location = flow_like_storage::object_store::path::Path::from("bits")
        .child(bit.hash.clone().unwrap_or(bit.id.clone()));
    let _delete = store.as_generic().delete(&old_location).await;

    let url = match bit.download_link {
        Some(ref link) => link,
        None => return Ok(()),
    };

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60 * 60 * 2))
        .connect_timeout(std::time::Duration::from_secs(30))
        .pool_idle_timeout(std::time::Duration::from_secs(90))
        .pool_max_idle_per_host(1)
        .http2_keep_alive_interval(Some(std::time::Duration::from_secs(30)))
        .http2_keep_alive_timeout(std::time::Duration::from_secs(60))
        .build()?;

    let response = client.head(url).send().await?;
    let content_length = response
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());

    let supports_ranges = response
        .headers()
        .get(ACCEPT_RANGES)
        .map(|v| v.to_str().unwrap_or("").contains("bytes"))
        .unwrap_or(false);

    let e_tag = response
        .headers()
        .get(ETAG)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().trim_matches('"').to_string())
        .unwrap_or_else(create_id);

    if let Some(tx) = &tx {
        let _ = tx
            .send(StreamMsg::Progress(Progress {
                stage: "head",
                message: None,
                downloaded: Some(0),
                total: content_length,
                percent: Some(0.0),
                hash: None,
            }))
            .await;
    }

    let path = flow_like_storage::object_store::path::Path::from("bits").child(e_tag.clone());

    // For ranged downloads
    const CHUNK_SIZE: usize = 50 * 1024 * 1024; // 50MB chunks
    // Multipart minimum part size on S3-compatible backends
    const MIN_MULTIPART_PART_SIZE: usize = 5 * 1024 * 1024;
    // Use single PUT for small objects
    const SINGLE_PUT_THRESHOLD: u64 = MIN_MULTIPART_PART_SIZE as u64;

    let mut hasher = blake3::Hasher::new();
    let mut total_downloaded = 0u64;

    // Fast path: small files -> single put (avoid multipart altogether)
    if content_length.is_some() && content_length.unwrap() <= SINGLE_PUT_THRESHOLD {
        let resp = client.get(url).send().await?;
        let bytes = resp.bytes().await?;
        hasher.update(&bytes);
        total_downloaded = bytes.len() as u64;

        store
            .as_generic()
            .put(&path, PutPayload::from_bytes(bytes))
            .await?;

        if let Some(tx) = &tx {
            let _ = tx
                .send(StreamMsg::Progress(Progress {
                    stage: "downloading",
                    message: None,
                    downloaded: Some(total_downloaded),
                    total: content_length,
                    percent: Some(100.0),
                    hash: None,
                }))
                .await;
        }
    } else if supports_ranges && content_length.is_some() {
        // Ranged download with large parts (>= MIN_MULTIPART_PART_SIZE)
        let file_size = content_length.unwrap();
        let mut start = 0u64;
        let mut pending_upload = None;
        let mut upload_request = store.as_generic().put_multipart(&path).await?;

        while start < file_size {
            let end = std::cmp::min(start + CHUNK_SIZE as u64 - 1, file_size - 1);
            let range_header = format!("bytes={}-{}", start, end);

            let mut retry_count = 0;
            const MAX_RETRIES: u32 = 3;

            loop {
                match client.get(url).header("Range", &range_header).send().await {
                    Ok(chunk_response) => {
                        let chunk_bytes = chunk_response.bytes().await?;
                        hasher.update(&chunk_bytes);
                        let payload = PutPayload::from_bytes(chunk_bytes);

                        if let Some(handle) = pending_upload.take() {
                            handle.await??;
                        }

                        let upload_fut = upload_request.put_part(payload);
                        pending_upload = Some(flow_like_types::tokio::spawn(upload_fut));

                        total_downloaded += end - start + 1;

                        if let Some(tx) = &tx {
                            let percent = (total_downloaded as f32 / file_size as f32) * 100.0;
                            let _ = tx
                                .send(StreamMsg::Progress(Progress {
                                    stage: "downloading",
                                    message: None,
                                    downloaded: Some(total_downloaded),
                                    total: Some(file_size),
                                    percent: Some(percent),
                                    hash: None,
                                }))
                                .await;
                        }
                        break;
                    }
                    Err(e) if retry_count < MAX_RETRIES => {
                        retry_count += 1;
                        tracing::warn!(
                            "Retry {}/{} for range {}-{}: {}",
                            retry_count,
                            MAX_RETRIES,
                            start,
                            end,
                            e
                        );
                        flow_like_types::tokio::time::sleep(std::time::Duration::from_secs(
                            2u64.pow(retry_count),
                        ))
                        .await;
                    }
                    Err(e) => return Err(e.into()),
                }
            }

            start = end + 1;
        }

        if let Some(upload_task) = pending_upload {
            upload_task.await??;
        }

        upload_request.complete().await?;
    } else {
        // Streaming download without range support: buffer to meet multipart minimum part size
        let mut download_stream = client.get(url).send().await?.bytes_stream();
        let mut upload_request = store.as_generic().put_multipart(&path).await?;
        let mut buffer: Vec<u8> = Vec::with_capacity(MIN_MULTIPART_PART_SIZE * 2);

        while let Some(chunk_result) = download_stream.next().await {
            match chunk_result {
                Ok(chunk) => {
                    hasher.update(&chunk);
                    buffer.extend_from_slice(&chunk);

                    // Upload full-size parts as we accumulate them
                    while buffer.len() >= MIN_MULTIPART_PART_SIZE {
                        let part = buffer.split_off(MIN_MULTIPART_PART_SIZE);
                        let to_upload = std::mem::replace(&mut buffer, part);
                        total_downloaded += to_upload.len() as u64;
                        let byte = Bytes::from(to_upload);
                        upload_request
                            .put_part(PutPayload::from_bytes(byte))
                            .await?;

                        if let Some(tx) = &tx {
                            let percent = content_length
                                .map(|total| (total_downloaded as f32 / total as f32) * 100.0)
                                .unwrap_or(0.0);
                            let _ = tx
                                .send(StreamMsg::Progress(Progress {
                                    stage: "downloading",
                                    message: None,
                                    downloaded: Some(total_downloaded),
                                    total: content_length,
                                    percent: Some(percent),
                                    hash: None,
                                }))
                                .await;
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("Stream error: {}", e);
                    return Err(e.into());
                }
            }
        }

        // Flush the remaining (last) part, can be smaller than MIN_MULTIPART_PART_SIZE
        if !buffer.is_empty() {
            total_downloaded += buffer.len() as u64;
            let byte = Bytes::from(buffer);
            upload_request
                .put_part(PutPayload::from_bytes(byte))
                .await?;
        }

        upload_request.complete().await?;
    }

    let file_hash = hasher.finalize().to_hex().to_string().to_lowercase();
    bit.hash = Some(file_hash.clone());
    if bit
        .dependency_tree_hash
        .as_ref()
        .is_none_or(|h| h.is_empty())
    {
        bit.dependency_tree_hash = Some(file_hash.clone());
    }

    bit.size = Some(total_downloaded as i64);

    let url = state.platform_config.cdn.clone().unwrap_or("".to_string());
    let url = format!("{}/bits/{}", url, e_tag);
    bit.download_link = Some(url.to_string());

    if let Some(tx) = &tx {
        let _ = tx
            .send(StreamMsg::Progress(Progress {
                stage: "hashed",
                message: None,
                downloaded: Some(total_downloaded),
                total: content_length,
                percent: Some(100.0),
                hash: Some(file_hash.clone()),
            }))
            .await;
    }

    tracing::info!(
        "Successfully processed {} bytes with hash {}",
        total_downloaded,
        file_hash
    );
    Ok(())
}

#[tracing::instrument(name = "build_dependency_hash", skip(bit, state))]
async fn build_dependency_hash(
    bit: &mut bit::Model,
    state: AppState,
    tx: Option<mpsc::Sender<StreamMsg>>,
) -> flow_like_types::Result<()> {
    let mut dependencies = match &bit.dependencies {
        Some(deps) => deps.clone(),
        None => {
            tracing::info!("No dependencies provided for bit {}", bit.id);
            if bit
                .dependency_tree_hash
                .as_ref()
                .is_none_or(|h| h.is_empty())
            {
                bit.dependency_tree_hash = Some(bit.hash.clone().unwrap_or_else(|| bit.id.clone()));
            }
            return Ok(());
        }
    };

    if dependencies.is_empty() {
        if bit
            .dependency_tree_hash
            .as_ref()
            .is_none_or(|h| h.is_empty())
        {
            bit.dependency_tree_hash = Some(bit.hash.clone().unwrap_or_else(|| bit.id.clone()));
        }
        return Ok(());
    }

    dependencies.sort();
    let mut hasher = blake3::Hasher::new();
    let http_client = HTTPClient::new_without_refetch();
    let http_client = Arc::new(http_client);

    if let Some(tx) = &tx {
        let _ = tx
            .send(StreamMsg::Progress(Progress {
                stage: "dep-hash",
                message: Some("start".into()),
                downloaded: None,
                total: Some(dependencies.len() as u64),
                percent: Some(0.0),
                hash: None,
            }))
            .await;
    }

    let total = dependencies.len() as f32;
    let mut idx = 0f32;
    for dependency in dependencies {
        let (hub, id) = dependency.split_once(':').ok_or_else(|| {
            flow_like_types::Error::msg(format!("Invalid dependency format: {}", dependency))
        })?;

        if hub == state.platform_config.domain {
            let local_bit = bit::Entity::find_by_id(id)
                .one(&state.db)
                .await?
                .ok_or_else(|| {
                    flow_like_types::Error::msg(format!("Local bit not found: {}", id))
                })?;
            let dep_hash = local_bit
                .dependency_tree_hash
                .unwrap_or_else(|| local_bit.id.clone());
            hasher.update(dep_hash.as_bytes());
        } else {
            let hub = flow_like::hub::Hub::new(hub, http_client.clone()).await?;
            let remote_bit = hub.get_bit(id).await.map_err(|e| {
                flow_like_types::Error::msg(format!("Failed to fetch remote bit {}: {}", id, e))
            })?;
            hasher.update(remote_bit.dependency_tree_hash.as_bytes());
        }

        idx += 1.0;
        if let Some(tx) = &tx {
            let _ = tx
                .send(StreamMsg::Progress(Progress {
                    stage: "dep-hash",
                    message: None,
                    downloaded: Some(idx as u64),
                    total: Some(total as u64),
                    percent: Some((idx / total) * 100.0),
                    hash: None,
                }))
                .await;
        }
    }

    let dependency_hash = hasher.finalize().to_hex().to_string().to_lowercase();
    bit.dependency_tree_hash = Some(dependency_hash.clone());
    tracing::info!(
        "Built dependency hash for bit {}: {}",
        bit.id,
        dependency_hash
    );

    if let Some(tx) = &tx {
        let _ = tx
            .send(StreamMsg::Progress(Progress {
                stage: "dep-hash",
                message: Some("done".into()),
                downloaded: None,
                total: None,
                percent: Some(100.0),
                hash: None,
            }))
            .await;
    }

    Ok(())
}

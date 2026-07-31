use std::sync::Arc;

use crate::{
    entity::{bit, bit_tree_cache},
    error::ApiError,
    middleware::jwt::AppUser,
    permission::global_permission::GlobalPermission,
    state::AppState,
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
use sea_orm::{
    ActiveEnum, ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter,
};
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

const ARTIFACT_IDENTITY_DOMAIN: &[u8] = b"flow-like-bit-artifact-v2";
const DEPENDENCY_TREE_IDENTITY_DOMAIN: &[u8] = b"flow-like-bit-tree-v2";

fn update_hash_field(hasher: &mut blake3::Hasher, value: &str) {
    hasher.update(&(value.len() as u64).to_le_bytes());
    hasher.update(value.as_bytes());
}

/// Content hashes alone are insufficient for artifacts that must be materialized
/// at distinct paths or interpreted as different Bit types. Keep intentional
/// deduplication for an exact content/path/type match while preserving either
/// identity distinction.
fn artifact_dependency_tree_hash(
    content_hash: &str,
    file_name: Option<&str>,
    bit_type: &str,
) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(ARTIFACT_IDENTITY_DOMAIN);
    update_hash_field(&mut hasher, content_hash);
    update_hash_field(&mut hasher, bit_type);
    match file_name {
        Some(file_name) => {
            hasher.update(&[1]);
            update_hash_field(&mut hasher, file_name);
        }
        None => {
            hasher.update(&[0]);
        }
    }
    hasher.finalize().to_hex().to_string().to_lowercase()
}

fn standalone_dependency_tree_hash(bit: &bit::Model) -> String {
    let content_hash = bit
        .hash
        .as_deref()
        .filter(|hash| !hash.is_empty())
        .unwrap_or(&bit.id);

    artifact_dependency_tree_hash(
        content_hash,
        bit.file_name.as_deref(),
        &bit.r#type.to_value(),
    )
}

/// Build a deterministic tree identity from the exact dependency references and
/// their resolved identities. Length-prefixing prevents ambiguous concatenation,
/// and sorting makes dependency order irrelevant while preserving duplicates.
fn resolved_dependency_tree_hash(
    root_type: &str,
    root_file_name: Option<&str>,
    own_artifact_identity: Option<&str>,
    dependencies: &[(String, String)],
) -> String {
    let mut dependencies = dependencies.to_vec();
    dependencies.sort();

    let mut hasher = blake3::Hasher::new();
    hasher.update(DEPENDENCY_TREE_IDENTITY_DOMAIN);
    update_hash_field(&mut hasher, root_type);
    match root_file_name {
        Some(file_name) => {
            hasher.update(&[1]);
            update_hash_field(&mut hasher, file_name);
        }
        None => {
            hasher.update(&[0]);
        }
    }
    match own_artifact_identity {
        Some(identity) => {
            hasher.update(&[1]);
            update_hash_field(&mut hasher, identity);
        }
        None => {
            hasher.update(&[0]);
        }
    }
    hasher.update(&(dependencies.len() as u64).to_le_bytes());
    for (dependency_ref, dependency_identity) in dependencies {
        update_hash_field(&mut hasher, &dependency_ref);
        update_hash_field(&mut hasher, &dependency_identity);
    }
    hasher.finalize().to_hex().to_string().to_lowercase()
}

fn validate_upstream_status(
    status: reqwest::StatusCode,
    operation: &str,
) -> flow_like_types::Result<()> {
    if status.is_success() {
        return Ok(());
    }

    Err(flow_like_types::Error::msg(format!(
        "Upstream {operation} request failed with HTTP status {status}"
    )))
}

fn successful_upstream_response(
    response: reqwest::Response,
    operation: &str,
) -> flow_like_types::Result<reqwest::Response> {
    validate_upstream_status(response.status(), operation)?;
    Ok(response)
}

fn validate_range_status(status: reqwest::StatusCode) -> flow_like_types::Result<()> {
    validate_upstream_status(status, "range GET")?;
    if status != reqwest::StatusCode::PARTIAL_CONTENT {
        return Err(flow_like_types::Error::msg(format!(
            "Upstream range GET request returned {status}; expected 206 Partial Content"
        )));
    }
    Ok(())
}

fn successful_range_response(
    response: reqwest::Response,
) -> flow_like_types::Result<reqwest::Response> {
    validate_range_status(response.status())?;
    Ok(response)
}

fn validate_range_body_length(received: usize, expected: u64) -> flow_like_types::Result<()> {
    if received as u64 == expected {
        return Ok(());
    }

    Err(flow_like_types::Error::msg(format!(
        "Upstream range GET returned {received} bytes; expected {expected}"
    )))
}

fn bit_identity_changed(existing: &bit::Model, incoming: &bit::Model) -> bool {
    existing.download_link != incoming.download_link
        || existing.file_name != incoming.file_name
        || existing.r#type != incoming.r#type
        || existing.dependencies != incoming.dependencies
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
        if model
            .download_link
            .as_ref()
            .is_some_and(|link| link.trim().is_empty())
        {
            model.download_link = None;
        }
        match bit::Entity::find_by_id(&bit_id_cloned)
            .one(&state_cloned.db)
            .await
        {
            Ok(Some(existing_bit)) => {
                let should_download = existing_bit.download_link != model.download_link;
                let identity_changed = bit_identity_changed(&existing_bit, &model);

                if !should_download {
                    // The registry's content identity and measured size remain
                    // authoritative when only the stored path/tree changes.
                    model.hash = existing_bit.hash.clone();
                    model.size = existing_bit.size;
                }

                let previous_tree_hash = existing_bit.dependency_tree_hash.clone();
                let mut updated_bit: bit::ActiveModel = existing_bit.into();
                if should_download {
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
                }
                if identity_changed {
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
                if identity_changed
                    && let Some(tree_hash) = &previous_tree_hash
                    && let Err(error) = bit_tree_cache::Entity::delete_by_id(tree_hash)
                        .exec(&state_cloned.db)
                        .await
                {
                    tracing::warn!(
                        bit_id = %bit_id_cloned,
                        dependency_tree_hash = %tree_hash,
                        %error,
                        "Failed to invalidate persisted Bit dependency cache"
                    );
                }
                match updated_bit.update(&state_cloned.db).await {
                    Ok(updated) => {
                        if identity_changed {
                            state_cloned
                                .invalidate_cache(&format!("get_with_dependencies:{}", updated.id));
                        }
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

#[tracing::instrument(name = "download_and_hash_bit", skip(bit, state, tx))]
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
        bit.dependency_tree_hash = Some(standalone_dependency_tree_hash(bit));
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

    let response =
        successful_upstream_response(client.head(url).send().await?, "HEAD").map_err(|error| {
            tracing::warn!(
                "Rejected upstream HEAD response for bit {}: {}",
                bit.id,
                error
            );
            error
        })?;
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
        let resp = successful_upstream_response(client.get(url).send().await?, "GET")?;
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
                        let chunk_response = successful_range_response(chunk_response)?;
                        let chunk_bytes = chunk_response.bytes().await?;
                        let expected_chunk_length = end - start + 1;
                        validate_range_body_length(chunk_bytes.len(), expected_chunk_length)?;
                        hasher.update(&chunk_bytes);
                        let payload = PutPayload::from_bytes(chunk_bytes);

                        if let Some(handle) = pending_upload.take() {
                            handle.await??;
                        }

                        let upload_fut = upload_request.put_part(payload);
                        pending_upload = Some(flow_like_types::tokio::spawn(upload_fut));

                        total_downloaded += expected_chunk_length;

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
        let response = successful_upstream_response(client.get(url).send().await?, "GET")?;
        let mut download_stream = response.bytes_stream();
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
    bit.dependency_tree_hash = Some(artifact_dependency_tree_hash(
        &file_hash,
        bit.file_name.as_deref(),
        &bit.r#type.to_value(),
    ));

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

#[tracing::instrument(name = "build_dependency_hash", skip(bit, state, tx))]
async fn build_dependency_hash(
    bit: &mut bit::Model,
    state: AppState,
    tx: Option<mpsc::Sender<StreamMsg>>,
) -> flow_like_types::Result<()> {
    let mut dependencies = bit.dependencies.clone().unwrap_or_default();

    if dependencies.is_empty() {
        tracing::info!("No dependencies provided for bit {}", bit.id);
        bit.dependency_tree_hash = Some(standalone_dependency_tree_hash(bit));
        return Ok(());
    }

    dependencies.sort();
    let mut resolved_dependencies = Vec::with_capacity(dependencies.len());
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
            resolved_dependencies.push((dependency.clone(), dep_hash));
        } else {
            let hub = flow_like::hub::Hub::new(hub, http_client.clone()).await?;
            let remote_bit = hub.get_bit(id).await.map_err(|e| {
                flow_like_types::Error::msg(format!("Failed to fetch remote bit {}: {}", id, e))
            })?;
            let dependency_identity = if remote_bit.dependency_tree_hash.is_empty() {
                remote_bit.id
            } else {
                remote_bit.dependency_tree_hash
            };
            resolved_dependencies.push((dependency.clone(), dependency_identity));
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

    let own_artifact_identity = bit
        .download_link
        .as_ref()
        .map(|_| standalone_dependency_tree_hash(bit));
    let dependency_hash = resolved_dependency_tree_hash(
        &bit.r#type.to_value(),
        bit.file_name.as_deref(),
        own_artifact_identity.as_deref(),
        &resolved_dependencies,
    );
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

#[cfg(test)]
mod tests {
    use super::*;

    const FILE_TYPE: &str = "FILE";

    fn artifact_identity(content_hash: &str, file_name: Option<&str>) -> String {
        artifact_dependency_tree_hash(content_hash, file_name, FILE_TYPE)
    }

    fn tree_identity(
        own_artifact_identity: Option<&str>,
        dependencies: &[(String, String)],
    ) -> String {
        resolved_dependency_tree_hash("LLM", None, own_artifact_identity, dependencies)
    }

    fn existing_downloadable_model() -> bit::Model {
        Bit {
            bit_type: flow_like::bit::BitTypes::File,
            download_link: Some("https://cdn.example.test/bits/artifact".to_string()),
            file_name: Some("config.json".to_string()),
            hash: "content-hash".to_string(),
            dependencies: vec!["hub:dependency".to_string()],
            ..Bit::default()
        }
        .into()
    }

    #[test]
    fn update_identity_detects_layout_type_and_dependency_changes_without_a_new_url() {
        let existing = existing_downloadable_model();
        assert!(!bit_identity_changed(&existing, &existing.clone()));

        let mut renamed = existing.clone();
        renamed.file_name = Some("nested/config.json".to_string());
        assert!(bit_identity_changed(&existing, &renamed));

        let mut retyped = existing.clone();
        retyped.r#type = flow_like::bit::BitTypes::Config.into();
        assert!(bit_identity_changed(&existing, &retyped));

        let mut rewired = existing.clone();
        rewired.dependencies = Some(vec!["hub:other-dependency".to_string()]);
        assert!(bit_identity_changed(&existing, &rewired));
    }

    #[test]
    fn artifact_identity_deduplicates_only_matching_content_path_and_type() {
        let first = artifact_identity("same-content", Some("config.json"));
        let duplicate = artifact_identity("same-content", Some("config.json"));
        let nested = artifact_identity("same-content", Some("nested/config.json"));
        let unnamed = artifact_identity("same-content", None);
        let other_content = artifact_identity("other-content", Some("config.json"));
        let other_type =
            artifact_dependency_tree_hash("same-content", Some("config.json"), "CONFIG");

        assert_eq!(first, duplicate);
        assert_ne!(first, nested);
        assert_ne!(first, unnamed);
        assert_ne!(first, other_content);
        assert_ne!(first, other_type);
    }

    #[test]
    fn dependency_tree_identity_is_order_independent_but_ref_sensitive() {
        let config = artifact_identity("config-bytes", Some("config.json"));
        let weights = artifact_identity("weight-bytes", Some("model.safetensors"));
        let dependencies = vec![
            ("hub:config".to_string(), config.clone()),
            ("hub:weights".to_string(), weights.clone()),
        ];
        let mut reversed = dependencies.clone();
        reversed.reverse();

        let expected = tree_identity(None, &dependencies);
        assert_eq!(expected, tree_identity(None, &reversed));

        let aliased = vec![
            ("hub:other-config-ref".to_string(), config.clone()),
            ("hub:weights".to_string(), weights.clone()),
        ];
        assert_ne!(expected, tree_identity(None, &aliased));

        let duplicate_ref = vec![
            ("hub:config".to_string(), config),
            ("hub:weights".to_string(), weights.clone()),
            ("hub:weights".to_string(), weights),
        ];
        assert_ne!(expected, tree_identity(None, &duplicate_ref));
    }

    #[test]
    fn dependency_tree_identity_includes_type_layout_and_own_artifact() {
        let root_config = artifact_identity("same", Some("config.json"));
        let nested_config = artifact_identity("same", Some("nested/config.json"));

        let root_layout = tree_identity(None, &[("hub:config".into(), root_config)]);
        let nested_layout = tree_identity(None, &[("hub:config".into(), nested_config)]);
        assert_ne!(root_layout, nested_layout);

        let own_artifact = artifact_identity("root-content", Some("model.bundle"));
        assert_ne!(
            root_layout,
            tree_identity(
                Some(&own_artifact),
                &[(
                    "hub:config".into(),
                    artifact_identity("same", Some("config.json"))
                )]
            )
        );

        let dependencies = &[(
            "hub:config".into(),
            artifact_identity("same", Some("config.json")),
        )];
        assert_ne!(
            resolved_dependency_tree_hash("LLM", None, None, dependencies),
            resolved_dependency_tree_hash("VLM", None, None, dependencies)
        );
        assert_ne!(
            resolved_dependency_tree_hash("LLM", None, None, dependencies),
            resolved_dependency_tree_hash("LLM", Some("root.bundle"), None, dependencies)
        );
    }

    #[test]
    fn upstream_status_validation_rejects_error_responses() {
        for status in [
            reqwest::StatusCode::MULTIPLE_CHOICES,
            reqwest::StatusCode::UNAUTHORIZED,
            reqwest::StatusCode::FORBIDDEN,
            reqwest::StatusCode::NOT_FOUND,
            reqwest::StatusCode::TOO_MANY_REQUESTS,
            reqwest::StatusCode::INTERNAL_SERVER_ERROR,
        ] {
            assert!(validate_upstream_status(status, "GET").is_err(), "{status}");
        }

        for status in [
            reqwest::StatusCode::OK,
            reqwest::StatusCode::PARTIAL_CONTENT,
        ] {
            assert!(validate_upstream_status(status, "GET").is_ok(), "{status}");
        }
    }

    #[test]
    fn range_validation_requires_partial_content_and_exact_length() {
        assert!(validate_range_status(reqwest::StatusCode::PARTIAL_CONTENT).is_ok());
        assert!(validate_range_status(reqwest::StatusCode::OK).is_err());
        assert!(validate_range_status(reqwest::StatusCode::NO_CONTENT).is_err());
        assert!(validate_range_status(reqwest::StatusCode::NOT_FOUND).is_err());

        assert!(validate_range_body_length(1_024, 1_024).is_ok());
        assert!(validate_range_body_length(1_023, 1_024).is_err());
        assert!(validate_range_body_length(2_048, 1_024).is_err());
    }
}

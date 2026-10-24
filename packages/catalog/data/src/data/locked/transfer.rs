use super::{
    client::{LockedAction, LockedApi},
    types::{
        CreateLockedUploadRequest, LockedDownload, LockedDownloadRequest, LockedIfExists,
        LockedItemDetail, LockedItemVersion, LockedMetadata, LockedScope, LockedStorageStatus,
        LockedUploadTarget, LockedUploadTicket, LockedVersionRef, LockedVersionStatus,
    },
};
use flow_like::flow::execution::{LogLevel, context::ExecutionContext, egress::GuardedHttpClient};
use flow_like_catalog_core::{FlowPath, mime_type_for};
use flow_like_storage::object_store::{ObjectStoreExt, PutPayload};
use flow_like_types::{anyhow, reqwest};
use futures::{StreamExt, TryStreamExt};
use sha2::{Digest, Sha256};
use std::{
    fmt::Write,
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};

const ERROR_SNIPPET_BYTES: usize = 300;

pub fn sha256_hex(digest: impl AsRef<[u8]>) -> String {
    let digest = digest.as_ref();
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

#[derive(Default)]
struct StreamDigest {
    hasher: Sha256,
    bytes: u64,
}

impl StreamDigest {
    fn update(&mut self, chunk: &[u8]) {
        self.hasher.update(chunk);
        self.bytes += chunk.len() as u64;
    }

    fn finish(self) -> (u64, String) {
        (self.bytes, sha256_hex(self.hasher.finalize()))
    }
}

pub fn ensure_content_matches(
    label: &str,
    expected_size: u64,
    expected_sha256: &str,
    size: u64,
    sha256: &str,
) -> flow_like_types::Result<()> {
    if size != expected_size {
        return Err(anyhow!(
            "{label}: expected {expected_size} bytes, got {size}"
        ));
    }
    if !sha256.eq_ignore_ascii_case(expected_sha256.trim()) {
        return Err(anyhow!(
            "{label}: SHA-256 {sha256} does not match the recorded {expected_sha256}"
        ));
    }
    Ok(())
}

pub fn ensure_enabled(status: &LockedStorageStatus) -> flow_like_types::Result<()> {
    if !status.enabled {
        return Err(anyhow!(
            "Locked storage is not configured on this server; ask an administrator to set a locked bucket"
        ));
    }
    Ok(())
}

/// What an upload asks for besides the file itself.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LockedUploadFields {
    pub item_id: Option<String>,
    pub scope: Option<LockedScope>,
    pub folder: Option<String>,
    pub name: Option<String>,
    pub retention_days: Option<u32>,
    pub delete_after_retention: bool,
    pub comment: Option<String>,
    pub metadata: Option<LockedMetadata>,
    pub if_exists: Option<LockedIfExists>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HashedFile {
    pub file_name: String,
    pub content_type: String,
    pub size: u64,
    pub sha256: String,
}

impl LockedUploadFields {
    pub fn into_request(self, file: &HashedFile) -> CreateLockedUploadRequest {
        CreateLockedUploadRequest {
            item_id: self.item_id,
            scope: self.scope,
            folder: self.folder,
            name: self.name,
            file_name: file.file_name.clone(),
            content_type: file.content_type.clone(),
            size: file.size,
            sha256: file.sha256.clone(),
            retention_days: self.retention_days,
            delete_after_retention: self.delete_after_retention,
            comment: self.comment,
            metadata: self.metadata,
            if_exists: self.if_exists,
        }
    }
}

fn transfer_error(operation: &str, error: reqwest::Error) -> flow_like_types::Error {
    anyhow!("{operation} failed: {}", error.without_url())
}

async fn rejected(operation: &str, response: reqwest::Response) -> flow_like_types::Error {
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    let body = body.trim();
    let mut end = body.len().min(ERROR_SNIPPET_BYTES);
    while !body.is_char_boundary(end) {
        end -= 1;
    }
    match &body[..end] {
        "" => anyhow!("{operation} was rejected by storage ({status})"),
        snippet => anyhow!("{operation} was rejected by storage ({status}): {snippet}"),
    }
}

/// Signed URLs are never re-sent to another host, and the stored bytes must arrive as they
/// were hashed, so neither redirects nor transparent decompression apply.
fn transfer_client(
    context: &ExecutionContext,
    read_timeout: Option<Duration>,
) -> flow_like_types::Result<GuardedHttpClient> {
    GuardedHttpClient::configured(context.execution_environment(), |builder| {
        let builder = builder
            .redirect(reqwest::redirect::Policy::none())
            .no_gzip()
            .no_brotli()
            .connect_timeout(Duration::from_secs(10));
        match read_timeout {
            Some(timeout) => builder.read_timeout(timeout),
            None => builder,
        }
    })
}

async fn hash_source(
    context: &mut ExecutionContext,
    path: &FlowPath,
    max_bytes: u64,
) -> flow_like_types::Result<HashedFile> {
    let file_name = flow_like_storage::display_file_name(&path.object_path())
        .filter(|name| !name.trim().is_empty())
        .ok_or_else(|| anyhow!("FlowPath '{}' names no file to lock", path.path))?;
    let runtime = path.to_runtime(context).await?;
    let store = runtime.store.as_generic();
    let size = store
        .head(&runtime.path)
        .await
        .map_err(|error| anyhow!("Cannot read '{}' to lock it: {error}", path.path))?
        .size;
    if size > max_bytes {
        return Err(anyhow!(
            "'{file_name}' has {size} bytes; locked storage accepts at most {max_bytes} bytes per version"
        ));
    }
    let mut stream = store.get(&runtime.path).await?.into_stream();
    let mut digest = StreamDigest::default();
    while let Some(chunk) = stream.next().await {
        digest.update(&chunk?);
    }
    let (read, sha256) = digest.finish();
    if read != size {
        return Err(anyhow!(
            "'{file_name}' changed while it was hashed ({size} bytes listed, {read} read)"
        ));
    }
    Ok(HashedFile {
        content_type: mime_type_for(&file_name).to_owned(),
        file_name,
        size,
        sha256,
    })
}

/// Streams the file to the signed URL with exactly the headers the server signed, hashing the
/// bytes on the way so a file that changed after pass one is never committed.
async fn put_source(
    context: &mut ExecutionContext,
    path: &FlowPath,
    target: &LockedUploadTarget,
    file: &HashedFile,
) -> flow_like_types::Result<()> {
    const OPERATION: &str = "Uploading the locked file";
    let runtime = path.to_runtime(context).await?;
    let stream = runtime
        .store
        .as_generic()
        .get(&runtime.path)
        .await?
        .into_stream();
    let digest = Arc::new(Mutex::new(StreamDigest::default()));
    let observed = digest.clone();
    let body = stream.inspect_ok(move |chunk| {
        observed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .update(chunk);
    });
    let method = reqwest::Method::from_bytes(target.method.trim().as_bytes()).map_err(|_| {
        anyhow!(
            "The server asked for an invalid upload method '{}'",
            target.method
        )
    })?;
    let mut request = transfer_client(context, None)?.request(method, &target.url)?;
    for (name, value) in &target.headers {
        request = request.header(name.as_str(), value.as_str());
    }
    if !target
        .headers
        .keys()
        .any(|name| name.eq_ignore_ascii_case("content-length"))
    {
        request = request.header(reqwest::header::CONTENT_LENGTH, file.size);
    }
    let response = request
        .body(reqwest::Body::wrap_stream(body))
        .send()
        .await
        .map_err(|error| transfer_error(OPERATION, error))?;
    if !response.status().is_success() {
        return Err(rejected(OPERATION, response).await);
    }
    let (sent, sha256) =
        std::mem::take(&mut *digest.lock().unwrap_or_else(PoisonError::into_inner)).finish();
    ensure_content_matches(
        "The file changed during the upload",
        file.size,
        &file.sha256,
        sent,
        &sha256,
    )
}

async fn abort_upload(
    context: &mut ExecutionContext,
    api: &LockedApi,
    version: &LockedVersionRef<'_>,
) {
    if let Err(error) = api
        .post::<flow_like_types::Value>(context, LockedAction::Abort, version)
        .await
    {
        context.log_message(
            &format!("Could not abort the unfinished locked upload: {error}"),
            LogLevel::Warn,
        );
    }
}

pub struct LockedUploadResult {
    pub detail: LockedItemDetail,
    pub version_id: String,
}

pub async fn upload_locked(
    context: &mut ExecutionContext,
    api: &LockedApi,
    path: &FlowPath,
    fields: LockedUploadFields,
) -> flow_like_types::Result<LockedUploadResult> {
    let status = api.status(context).await?;
    ensure_enabled(&status)?;
    let file = hash_source(context, path, status.max_file_bytes).await?;
    let ticket: LockedUploadTicket = api
        .post(context, LockedAction::Upload, &fields.into_request(&file))
        .await?;
    let version = LockedVersionRef {
        item_id: &ticket.item.id,
        version_id: Some(&ticket.version.id),
    };
    if let Err(error) = put_source(context, path, &ticket.upload, &file).await {
        abort_upload(context, api, &version).await;
        return Err(error);
    }
    let detail = api.post(context, LockedAction::Commit, &version).await?;
    Ok(LockedUploadResult {
        detail,
        version_id: ticket.version.id,
    })
}

fn cache_file_name(name: &str) -> String {
    let name: String = name
        .chars()
        .map(|c| {
            if matches!(c, '/' | '\\') || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    match name.trim() {
        "" | "." | ".." => "file".to_owned(),
        name => name.to_owned(),
    }
}

async fn cache_target(
    context: &mut ExecutionContext,
    file_name: &str,
) -> flow_like_types::Result<FlowPath> {
    let dir = FlowPath::from_cache_dir(context, true, false).await?;
    let path = format!(
        "{}/locked-{}/{}",
        dir.path.trim_end_matches('/'),
        flow_like_types::create_id(),
        cache_file_name(file_name)
    );
    Ok(FlowPath::new(path, dir.store_ref, dir.cache_store_ref))
}

/// Streams one locked version into the run's cache and fails unless its size and SHA-256 match
/// the record; the cache file only appears once both did.
pub async fn download_locked(
    context: &mut ExecutionContext,
    api: &LockedApi,
    version: &LockedItemVersion,
) -> flow_like_types::Result<FlowPath> {
    const OPERATION: &str = "Downloading the locked file";
    if version.status != LockedVersionStatus::Locked {
        return Err(anyhow!(
            "Locked version '{}' is {} and has no stored bytes to download",
            version.id,
            version.status.label()
        ));
    }
    let number = version
        .version
        .ok_or_else(|| anyhow!("Locked version '{}' has no version number", version.id))?;
    let download: LockedDownload = api
        .post(
            context,
            LockedAction::Download,
            &LockedDownloadRequest {
                item_id: &version.item_id,
                version: Some(number),
            },
        )
        .await?;
    if download.version_id != version.id {
        return Err(anyhow!(
            "{OPERATION}: the server offered version '{}' instead of '{}'",
            download.version_id,
            version.id
        ));
    }
    let response = transfer_client(context, Some(Duration::from_secs(120)))?
        .get(&download.url)?
        .send()
        .await
        .map_err(|error| transfer_error(OPERATION, error))?;
    if !response.status().is_success() {
        return Err(rejected(OPERATION, response).await);
    }
    if let Some(length) = response.content_length()
        && length != download.size
    {
        return Err(anyhow!(
            "{OPERATION}: storage announced {length} bytes, the record has {}",
            download.size
        ));
    }
    let target = cache_target(context, &version.file_name).await?;
    let runtime = target.to_runtime(context).await?;
    let store = runtime.store.as_generic();
    let mut writer = store.put_multipart(&runtime.path).await?;
    let mut stream = response.bytes_stream();
    let transfer = async {
        let mut digest = StreamDigest::default();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|error| transfer_error(OPERATION, error))?;
            digest.update(&chunk);
            if digest.bytes > download.size {
                return Err(anyhow!(
                    "{OPERATION}: storage sent more than the recorded {} bytes",
                    download.size
                ));
            }
            writer.put_part(PutPayload::from_bytes(chunk)).await?;
        }
        let (size, sha256) = digest.finish();
        ensure_content_matches(
            "The downloaded locked file is not the recorded one",
            download.size,
            &download.sha256,
            size,
            &sha256,
        )?;
        writer.complete().await?;
        Ok::<_, flow_like_types::Error>(())
    }
    .await;
    if let Err(error) = transfer {
        let _ = writer.abort().await;
        return Err(error);
    }
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::json::{json, to_value};

    #[test]
    fn sha256_is_lowercase_hex() {
        assert_eq!(
            sha256_hex(Sha256::digest(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_hex(Sha256::digest(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        let mut digest = StreamDigest::default();
        digest.update(b"a");
        digest.update(b"bc");
        assert_eq!(
            digest.finish(),
            (
                3,
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".to_owned()
            )
        );
    }

    fn hashed() -> HashedFile {
        HashedFile {
            file_name: "Rechnung 2026.pdf".to_owned(),
            content_type: "application/pdf".to_owned(),
            size: 42,
            sha256: "ab".repeat(32),
        }
    }

    #[test]
    fn a_new_item_upload_sends_only_what_was_set() {
        let mut metadata = LockedMetadata::new();
        metadata.insert("customer".to_owned(), json!("ACME"));
        let request = LockedUploadFields {
            scope: Some(LockedScope::User),
            folder: Some("invoices/2026".to_owned()),
            retention_days: Some(3650),
            delete_after_retention: true,
            metadata: Some(metadata),
            if_exists: Some(LockedIfExists::NewVersion),
            ..Default::default()
        }
        .into_request(&hashed());
        assert_eq!(
            to_value(&request).unwrap(),
            json!({
                "scope": "user",
                "folder": "invoices/2026",
                "file_name": "Rechnung 2026.pdf",
                "content_type": "application/pdf",
                "size": 42,
                "sha256": "ab".repeat(32),
                "retention_days": 3650,
                "delete_after_retention": true,
                "metadata": { "customer": "ACME" },
                "if_exists": "new_version",
            })
        );
    }

    #[test]
    fn a_version_upload_names_the_item() {
        let request = LockedUploadFields {
            item_id: Some("item-1".to_owned()),
            comment: Some("signed copy".to_owned()),
            ..Default::default()
        }
        .into_request(&hashed());
        assert_eq!(
            to_value(&request).unwrap(),
            json!({
                "item_id": "item-1",
                "file_name": "Rechnung 2026.pdf",
                "content_type": "application/pdf",
                "size": 42,
                "sha256": "ab".repeat(32),
                "comment": "signed copy",
            })
        );
    }

    #[test]
    fn content_must_match_size_and_hash() {
        let sha = "ab".repeat(32);
        assert!(ensure_content_matches("x", 42, &sha, 42, &sha).is_ok());
        assert!(ensure_content_matches("x", 42, &sha.to_uppercase(), 42, &sha).is_ok());
        assert!(ensure_content_matches("x", 42, &sha, 41, &sha).is_err());
        assert!(ensure_content_matches("x", 42, &sha, 42, &"cd".repeat(32)).is_err());
    }

    #[test]
    fn cache_file_names_stay_one_segment() {
        assert_eq!(cache_file_name("report.pdf"), "report.pdf");
        assert_eq!(cache_file_name("a/b\\c.txt"), "a_b_c.txt");
        for name in ["", "  ", ".", ".."] {
            assert_eq!(cache_file_name(name), "file");
        }
    }
}

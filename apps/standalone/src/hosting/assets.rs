use super::HostState;
use axum::{
    body::Body,
    extract::Request,
    http::{HeaderMap, Method, StatusCode},
    response::{IntoResponse, Response},
};
use flow_like_runtime::state::FlowLikeStores;
use flow_like_storage::{
    Path as ObjectPath, decode_path_segment,
    files::store::FlowLikeStore,
    object_store::{GetOptions, GetRange, ObjectStoreExt},
};
use futures_util::StreamExt;
use serde_json::Value;
use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::io::{AsyncReadExt, AsyncSeekExt};

const MAX_ASSET_BYTES: u64 = 32 * 1024 * 1024;
const MAX_HANDLES: usize = 1024;
const HANDLE_LIFETIME: Duration = Duration::from_secs(15 * 60);
const READ_DEADLINE: Duration = Duration::from_secs(30);
const PREFIX: &str = "/ui/assets";

#[derive(Clone)]
struct Target {
    store: FlowLikeStore,
    path: ObjectPath,
    kind: AssetStore,
}

#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum AssetStore {
    Upload,
    Storage,
    Temporary,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Location {
    kind: AssetStore,
    path: String,
}

struct Handle {
    target: Target,
    source: blake3::Hash,
    fingerprint: blake3::Hash,
    expires: Instant,
}

/// References emitted by this deployment may authorize individual storage or scratch files.
/// A browser can also name project uploads, but cannot enumerate or read general storage.
pub(super) struct Registry {
    project_id: String,
    stores: FlowLikeStores,
    handles: Mutex<HashMap<String, Handle>>,
    capacity: Arc<tokio::sync::Semaphore>,
    incarnation: Option<String>,
}

impl Registry {
    pub(super) fn new(
        project_id: &str,
        stores: FlowLikeStores,
        incarnation: Option<String>,
    ) -> Self {
        Self {
            project_id: project_id.into(),
            stores,
            handles: Mutex::new(HashMap::new()),
            capacity: Arc::new(tokio::sync::Semaphore::new(8)),
            incarnation,
        }
    }

    pub(super) fn rewrite(&self, value: &mut Value, fingerprint: blake3::Hash) {
        let Ok(mut handles) = self.handles.lock() else {
            return;
        };
        let now = Instant::now();
        handles.retain(|_, handle| handle.expires > now);
        let mut remaining = 256;
        self.walk(value, fingerprint, &mut handles, &mut remaining, 0);
    }

    fn walk(
        &self,
        value: &mut Value,
        fingerprint: blake3::Hash,
        handles: &mut HashMap<String, Handle>,
        remaining: &mut usize,
        depth: usize,
    ) {
        if depth > 64 || *remaining == 0 {
            return;
        }
        match value {
            Value::String(source) => {
                if !(source.starts_with("asset://")
                    || source.starts_with("file://")
                    || source.starts_with("http://asset.localhost/"))
                {
                    return;
                }
                let source_hash = blake3::hash(source.as_bytes());
                if let Some((id, handle)) = handles.iter_mut().find(|(_, handle)| {
                    handle.source == source_hash && handle.fingerprint == fingerprint
                }) {
                    handle.expires = Instant::now() + HANDLE_LIFETIME;
                    *source = self.handle_url(id);
                    *remaining -= 1;
                    return;
                }
                let Some(target) = self.local_reference(source) else {
                    return;
                };
                if handles.len() == MAX_HANDLES {
                    return;
                }
                *remaining -= 1;
                let id = uuid::Uuid::new_v4().simple().to_string();
                handles.insert(
                    id.clone(),
                    Handle {
                        target,
                        source: source_hash,
                        fingerprint,
                        expires: Instant::now() + HANDLE_LIFETIME,
                    },
                );
                *source = self.handle_url(&id);
            }
            Value::Array(values) => {
                for value in values {
                    self.walk(value, fingerprint, handles, remaining, depth + 1);
                }
            }
            Value::Object(values) => {
                for value in values.values_mut() {
                    self.walk(value, fingerprint, handles, remaining, depth + 1);
                }
            }
            _ => (),
        }
    }

    fn upload(&self, relative: &str) -> Option<Target> {
        let path = relative_path(relative)?;
        Some(Target {
            store: self.stores.app_storage_store.clone()?,
            path: ObjectPath::parse(format!("apps/{}/upload/{}", self.project_id, path.as_ref()))
                .ok()?,
            kind: AssetStore::Upload,
        })
    }

    fn local_reference(&self, source: &str) -> Option<Target> {
        if source.len() > 4096 {
            return None;
        }
        let url = url::Url::parse(source).ok()?;
        if !(url.scheme() == "asset" && url.host_str() == Some("localhost")
            || url.scheme() == "http" && url.host_str() == Some("asset.localhost")
            || url.scheme() == "file" && url.host_str().is_none_or(|host| host == "localhost"))
            || !url.username().is_empty()
            || url.password().is_some()
            || url.port().is_some()
        {
            return None;
        }
        // Tauri encodes the entire absolute path after its first slash.
        let decoded = decode_path_segment(url.path()).into_owned();
        let mut written = decoded
            .strip_prefix("//")
            .map(|p| format!("/{p}"))
            .unwrap_or(decoded);
        let drive_path = written.strip_prefix('/').unwrap_or(&written).as_bytes();
        if drive_path.len() >= 3
            && drive_path[0].is_ascii_alphabetic()
            && drive_path[1] == b':'
            && matches!(drive_path[2], b'\\' | b'/')
        {
            written = format!("/{}", written.trim_start_matches('/').replace('\\', "/"));
        }
        let path = Path::new(&written);
        if !path.is_absolute()
            || path.components().any(|part| {
                matches!(
                    part,
                    std::path::Component::ParentDir | std::path::Component::CurDir
                )
            })
        {
            return None;
        }
        // Imported Pages can retain a desktop upload URL. Its project-scoped key is portable.
        let upload_prefix = format!("/apps/{}/upload/", self.project_id);
        if let Some((_, relative)) = written.split_once(&upload_prefix) {
            return self.upload(relative);
        }
        for (kind, store, prefixes) in [
            (
                AssetStore::Storage,
                self.stores.app_storage_store.as_ref(),
                vec![format!("apps/{}/storage", self.project_id)],
            ),
            (
                AssetStore::Temporary,
                self.stores.temporary_store.as_ref(),
                vec![
                    format!("tmp/global/apps/{}", self.project_id),
                    format!(
                        "tmp/user/{}/apps/{}",
                        flow_like_runtime::flow::execution::LOCAL_USER_SUB,
                        self.project_id
                    ),
                ],
            ),
        ] {
            let Some(FlowLikeStore::Local(local)) = store else {
                continue;
            };
            let root = local.directory_to_filesystem(&ObjectPath::default()).ok()?;
            let Ok(relative) = path.strip_prefix(&root) else {
                continue;
            };
            let relative = relative.to_str()?;
            if prefixes.iter().any(|prefix| {
                relative
                    .strip_prefix(prefix)
                    .is_some_and(|suffix| suffix.starts_with('/'))
            }) {
                return Some(Target {
                    store: store?.clone(),
                    path: relative_path(relative)?,
                    kind,
                });
            }
        }
        None
    }

    fn resolve(&self, request: &Request, fingerprint: blake3::Hash) -> Option<Target> {
        if let Some((owner, id)) = handle_parts(request) {
            if owner != self.incarnation.as_deref() {
                return None;
            }
            return self.target(self.resolve_handle(id, fingerprint)?);
        }
        if request.uri().path() != PREFIX {
            return None;
        }
        let mut store = None;
        let mut path = None;
        let query = request.uri().query()?;
        if query.len() > 4096 {
            return None;
        }
        for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
            match key.as_ref() {
                "store" if store.is_none() => store = Some(value.into_owned()),
                "path" if path.is_none() => path = Some(value.into_owned()),
                _ => return None,
            }
        }
        (store.as_deref() == Some("upload"))
            .then(|| self.upload(path.as_deref()?))
            .flatten()
    }

    fn handle_url(&self, id: &str) -> String {
        match &self.incarnation {
            Some(owner) => format!("{PREFIX}/{owner}/{id}"),
            None => format!("{PREFIX}/{id}"),
        }
    }

    pub(super) fn resolve_handle(&self, id: &str, fingerprint: blake3::Hash) -> Option<Location> {
        if !valid_id(id) {
            return None;
        }
        let handles = self.handles.lock().ok()?;
        let handle = handles.get(id)?;
        (handle.expires > Instant::now() && handle.fingerprint == fingerprint).then(|| Location {
            kind: handle.target.kind,
            path: handle.target.path.as_ref().into(),
        })
    }

    fn target(&self, location: Location) -> Option<Target> {
        let path = relative_path(&location.path)?;
        let (store, prefixes) = match location.kind {
            AssetStore::Upload => (
                self.stores.app_storage_store.clone()?,
                vec![format!("apps/{}/upload/", self.project_id)],
            ),
            AssetStore::Storage => (
                self.stores.app_storage_store.clone()?,
                vec![format!("apps/{}/storage/", self.project_id)],
            ),
            AssetStore::Temporary => (
                self.stores.temporary_store.clone()?,
                vec![
                    format!("tmp/global/apps/{}/", self.project_id),
                    format!(
                        "tmp/user/{}/apps/{}/",
                        flow_like_runtime::flow::execution::LOCAL_USER_SUB,
                        self.project_id
                    ),
                ],
            ),
        };
        prefixes
            .iter()
            .any(|prefix| path.as_ref().starts_with(prefix))
            .then_some(Target {
                store,
                path,
                kind: location.kind,
            })
    }
}

fn valid_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

fn handle_parts(request: &Request) -> Option<(Option<&str>, &str)> {
    if request.uri().query().is_some() {
        return None;
    }
    let suffix = request.uri().path().strip_prefix("/ui/assets/")?;
    if let Some((owner, id)) = suffix.split_once('/') {
        return (valid_id(owner) && valid_id(id)).then_some((Some(owner), id));
    }
    valid_id(suffix).then_some((None, suffix))
}

#[cfg(unix)]
pub(super) fn resolve_peer(host: &HostState, id: &str, fingerprint: &str) -> Option<Location> {
    let current = super::service_fingerprint(host.secret.as_deref()).ok()?;
    if host.cancel.is_cancelled() || current.to_hex().as_str() != fingerprint {
        return None;
    }
    host.assets.resolve_handle(id, current)
}

fn relative_path(path: &str) -> Option<ObjectPath> {
    if path.is_empty() || path.len() > 2048 || path.starts_with('/') {
        return None;
    }
    let mut out = ObjectPath::default();
    for part in path.split('/') {
        let decoded = decode_path_segment(part);
        if decoded.is_empty() || matches!(decoded.as_ref(), "." | "..")
            || decoded.contains(['/', '\\', '\0']) || decoded.chars().any(char::is_control)
            // A second decode must never create traversal or another separator.
            || decode_path_segment(&decoded) != decoded
        {
            return None;
        }
        out = out.join(decoded.as_ref());
    }
    Some(out)
}

pub(super) fn route(path: &str) -> bool {
    path == PREFIX || path.starts_with("/ui/assets/")
}

fn range(headers: &HeaderMap, size: u64) -> Result<(std::ops::Range<u64>, bool), ()> {
    let Some(header) = headers.get("range") else {
        return Ok((0..size, false));
    };
    if headers.get_all("range").iter().count() != 1 || size == 0 {
        return Err(());
    }
    let value = header
        .to_str()
        .map_err(|_| ())?
        .strip_prefix("bytes=")
        .ok_or(())?;
    let (start, end) = value.split_once('-').ok_or(())?;
    let (start, end) = if start.is_empty() {
        let length = end.parse::<u64>().map_err(|_| ())?;
        if length == 0 {
            return Err(());
        }
        (size.saturating_sub(length), size)
    } else {
        let start = start.parse::<u64>().map_err(|_| ())?;
        let end = if end.is_empty() {
            size
        } else {
            end.parse::<u64>()
                .map_err(|_| ())?
                .saturating_add(1)
                .min(size)
        };
        (start, end)
    };
    if start >= end || start >= size {
        return Err(());
    }
    Ok((start..end, true))
}

pub(super) async fn read(
    host: &HostState,
    request: Request,
    fingerprint: blake3::Hash,
) -> Response {
    if !matches!(*request.method(), Method::GET | Method::HEAD) {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }
    let mut target = host.assets.resolve(&request, fingerprint);
    #[cfg(unix)]
    if target.is_none()
        && let Some((Some(owner), id)) = handle_parts(&request)
        && let Some(route) = &host.reply_route
        && owner != route.incarnation
    {
        target = tokio::select! {
            _ = host.cancel.cancelled() => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
            location = route.resolve_asset(owner, id, fingerprint) => location.ok().and_then(|location| host.assets.target(location)),
        };
    }
    let Some(target) = target else {
        return StatusCode::NOT_FOUND.into_response();
    };
    // Assets must remain readable while an interactive run holds the workflow permit.
    let Ok(permit) = host.assets.capacity.clone().try_acquire_owned() else {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    };
    let headers = request.headers().clone();
    let opened = tokio::select! {
        _ = host.cancel.cancelled() => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
        opened = tokio::time::timeout(READ_DEADLINE, open(&target, &headers)) => opened,
    };
    let (size, range, partial, source) = match opened {
        Ok(Ok(opened)) => opened,
        Ok(Err(status)) => return status.into_response(),
        Err(_) => return StatusCode::GATEWAY_TIMEOUT.into_response(),
    };
    let length = range.end - range.start;
    let mut response = Response::builder()
        .status(if partial {
            StatusCode::PARTIAL_CONTENT
        } else {
            StatusCode::OK
        })
        .header("content-type", content_type(target.path.as_ref()))
        .header("content-length", length)
        .header("accept-ranges", "bytes")
        .header("content-security-policy", "default-src 'none'; sandbox")
        .header("referrer-policy", "no-referrer");
    if partial {
        response = response.header(
            "content-range",
            format!("bytes {}-{}/{size}", range.start, range.end - 1),
        );
    }
    let body = if request.method() == Method::HEAD {
        Body::empty()
    } else {
        let cancel = host.cancel.clone();
        let stream = futures_util::stream::unfold(
            (source, length, permit),
            move |(mut source, remaining, permit)| {
                let cancel = cancel.clone();
                async move {
                    if remaining == 0 {
                        return None;
                    }
                    let next = tokio::select! {
                        _ = cancel.cancelled() => Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "Service stopped")),
                        next = tokio::time::timeout(READ_DEADLINE, source.next(remaining)) => next.unwrap_or_else(|_| Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "Asset read timed out"))),
                    };
                    let left = match &next {
                        Ok(bytes) => remaining - bytes.len() as u64,
                        Err(_) => 0,
                    };
                    Some((next, (source, left, permit)))
                }
            },
        );
        Body::from_stream(stream)
    };
    response
        .body(body)
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

enum Source {
    Local(tokio::fs::File),
    Remote(
        futures_util::stream::BoxStream<
            'static,
            flow_like_storage::object_store::Result<bytes::Bytes>,
        >,
    ),
}

impl Source {
    async fn next(&mut self, remaining: u64) -> std::io::Result<bytes::Bytes> {
        let bytes = match self {
            Self::Local(file) => {
                let mut bytes = vec![0; remaining.min(16 * 1024) as usize];
                let read = file.read(&mut bytes).await?;
                bytes.truncate(read);
                bytes::Bytes::from(bytes)
            }
            Self::Remote(stream) => stream
                .next()
                .await
                .transpose()
                .map_err(std::io::Error::other)?
                .unwrap_or_default(),
        };
        if bytes.is_empty() || bytes.len() as u64 > remaining {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "Asset changed while being read",
            ));
        }
        Ok(bytes)
    }
}

async fn open(
    target: &Target,
    headers: &HeaderMap,
) -> Result<(u64, std::ops::Range<u64>, bool, Source), StatusCode> {
    if let FlowLikeStore::Local(local) = &target.store {
        let root = local
            .directory_to_filesystem(&ObjectPath::default())
            .map_err(|_| StatusCode::NOT_FOUND)?;
        let path = local
            .path_to_filesystem(&target.path)
            .map_err(|_| StatusCode::NOT_FOUND)?;
        let mut file = tokio::fs::File::from_std(
            tokio::task::spawn_blocking(move || confined_file(&root, &path))
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
                .map_err(|_| StatusCode::NOT_FOUND)?,
        );
        let size = file
            .metadata()
            .await
            .map_err(|_| StatusCode::NOT_FOUND)?
            .len();
        if size > MAX_ASSET_BYTES {
            return Err(StatusCode::PAYLOAD_TOO_LARGE);
        }
        let (range, partial) =
            range(headers, size).map_err(|_| StatusCode::RANGE_NOT_SATISFIABLE)?;
        file.seek(std::io::SeekFrom::Start(range.start))
            .await
            .map_err(|_| StatusCode::NOT_FOUND)?;
        return Ok((size, range, partial, Source::Local(file)));
    }
    let store = target.store.as_generic();
    let meta = store
        .head(&target.path)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    if meta.size > MAX_ASSET_BYTES {
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }
    let (range, partial) =
        range(headers, meta.size).map_err(|_| StatusCode::RANGE_NOT_SATISFIABLE)?;
    let mut options = GetOptions::default();
    options.range = (meta.size > 0).then(|| GetRange::Bounded(range.clone()));
    options.if_match = meta.e_tag;
    let result = store
        .get_opts(&target.path, options)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    if result.meta.size > MAX_ASSET_BYTES {
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }
    Ok((
        meta.size,
        range,
        partial,
        Source::Remote(result.into_stream()),
    ))
}

/// Open every component relative to an already open directory so symlink swaps cannot escape.
#[cfg(unix)]
fn confined_file(root: &Path, path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::{
        fd::{AsRawFd, FromRawFd},
        unix::{
            ffi::OsStrExt,
            fs::{MetadataExt, OpenOptionsExt},
        },
    };
    let relative = path.strip_prefix(root).map_err(std::io::Error::other)?;
    let parts = relative.components().collect::<Vec<_>>();
    if parts.is_empty() {
        return Err(std::io::Error::other("Missing asset path"));
    }
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(root)?;
    for (index, part) in parts.iter().enumerate() {
        let std::path::Component::Normal(name) = part else {
            return Err(std::io::Error::other("Invalid asset path"));
        };
        let name = std::ffi::CString::new(name.as_bytes()).map_err(std::io::Error::other)?;
        let final_part = index + 1 == parts.len();
        let flags = libc::O_RDONLY
            | libc::O_NOFOLLOW
            | libc::O_CLOEXEC
            | libc::O_NONBLOCK
            | if final_part { 0 } else { libc::O_DIRECTORY };
        let fd = unsafe { libc::openat(file.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        file = unsafe { std::fs::File::from_raw_fd(fd) };
    }
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.nlink() != 1 {
        return Err(std::io::Error::other(
            "Asset must be a regular file with one link",
        ));
    }
    Ok(file)
}

#[cfg(not(unix))]
fn confined_file(_root: &Path, _path: &Path) -> std::io::Result<std::fs::File> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "Local asset serving requires directory-relative file access",
    ))
}

fn content_type(path: &str) -> &'static str {
    match path
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "svg" => "image/svg+xml",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" | "oga" => "audio/ogg",
        "m4a" => "audio/mp4",
        "flac" => "audio/flac",
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "pdf" => "application/pdf",
        "json" | "geojson" => "application/json",
        "txt" | "csv" | "md" => "text/plain; charset=utf-8",
        "glb" => "model/gltf-binary",
        "gltf" => "model/gltf+json",
        _ => "application/octet-stream",
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use flow_like_storage::files::store::local_store::LocalObjectStore;
    use serde_json::json;
    use std::sync::Arc;

    fn registry(root: &Path) -> Registry {
        Registry::new(
            "project",
            FlowLikeStores {
                app_storage_store: Some(FlowLikeStore::Local(Arc::new(
                    LocalObjectStore::new(root.into()).unwrap(),
                ))),
                temporary_store: Some(FlowLikeStore::Local(Arc::new(
                    LocalObjectStore::new(root.join("tmp")).unwrap(),
                ))),
                ..Default::default()
            },
            None,
        )
    }

    fn request(path: &str) -> Request {
        Request::builder().uri(path).body(Body::empty()).unwrap()
    }

    #[test]
    fn upload_keys_reject_encoded_traversal_and_keep_nested_names() {
        let dir = tempfile::tempdir().unwrap();
        let registry = registry(dir.path());
        let fingerprint = blake3::hash(b"token");
        let path = registry
            .resolve(
                &request("/ui/assets?store=upload&path=media%2Fmy%20image.png"),
                fingerprint,
            )
            .unwrap();
        assert_eq!(path.path.as_ref(), "apps/project/upload/media/my image.png");
        for path in [
            "../storage/private.db",
            "%2e%2e/private.db",
            "%252e%252e/private.db",
            "a/%2f/private",
            "a\\b",
            "a//b",
            "/etc/passwd",
        ] {
            assert!(registry.upload(path).is_none(), "{path}");
        }
        for query in [
            "store=storage&path=private.db",
            "store=upload&path=a&path=b",
            "store=upload&path=a&url=file%3A%2F%2F%2Fetc%2Fpasswd",
            "url=file%3A%2F%2F%2Fetc%2Fpasswd",
        ] {
            assert!(
                registry
                    .resolve(&request(&format!("/ui/assets?{query}")), fingerprint)
                    .is_none(),
                "{query}"
            );
        }
    }

    #[test]
    fn emitted_file_handles_bind_the_project_token_and_lifetime() {
        let dir = tempfile::tempdir().unwrap();
        // LocalObjectStore emits canonical filesystem paths; macOS aliases /var to /private/var.
        let root = dir.path().canonicalize().unwrap();
        let registry = registry(&root);
        let fingerprint = blake3::hash(b"token");
        let source = url::Url::from_file_path(root.join("apps/project/storage/report.pdf"))
            .unwrap()
            .to_string();
        let mut payload = json!({"attachment":{"url":source}, "private":"file:///etc/passwd", "other":url::Url::from_file_path(dir.path().join("apps/other/storage/file")).unwrap().to_string()});
        registry.rewrite(&mut payload, fingerprint);
        let issued = payload["attachment"]["url"].as_str().unwrap();
        assert!(issued.starts_with("/ui/assets/"));
        assert_eq!(payload["private"], "file:///etc/passwd");
        assert!(payload["other"].as_str().unwrap().starts_with("file:"));
        let target = registry.resolve(&request(issued), fingerprint).unwrap();
        assert_eq!(target.path.as_ref(), "apps/project/storage/report.pdf");
        assert!(
            registry
                .resolve(&request(issued), blake3::hash(b"replacement"))
                .is_none()
        );
        registry
            .handles
            .lock()
            .unwrap()
            .values_mut()
            .for_each(|h| h.expires = Instant::now());
        assert!(registry.resolve(&request(issued), fingerprint).is_none());
    }

    #[test]
    fn imported_upload_urls_translate_only_the_deployed_project() {
        let dir = tempfile::tempdir().unwrap();
        let registry = registry(dir.path());
        let mut payload = json!([
            "asset://localhost/%2Fhome%2Fstudio%2Fapps%2Fproject%2Fupload%2Flogo.png",
            "http://asset.localhost/C%3A%5CUsers%5Cstudio%5Capps%5Cother%5Cupload%5Csecret.txt",
            "https://remote.example/image.png",
            "data:image/png;base64,AAAA",
            "http://asset.localhost/C%3A%5CUsers%5Cstudio%5Capps%5Cproject%5Cupload%5Clogo.png"
        ]);
        registry.rewrite(&mut payload, blake3::hash(b"token"));
        assert!(payload[0].as_str().unwrap().starts_with("/ui/assets/"));
        assert!(
            payload[1]
                .as_str()
                .unwrap()
                .starts_with("http://asset.localhost/")
        );
        assert_eq!(payload[2], "https://remote.example/image.png");
        assert_eq!(payload[3], "data:image/png;base64,AAAA");
        assert!(payload[4].as_str().unwrap().starts_with("/ui/assets/"));
    }

    #[test]
    fn asset_handle_memory_is_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let registry = registry(dir.path());
        for batch in 0..8 {
            let mut payload = json!(
                (0..256)
                    .map(|id| format!("file:///old/apps/project/upload/image-{batch}-{id}.png"))
                    .collect::<Vec<_>>()
            );
            registry.rewrite(&mut payload, blake3::hash(b"token"));
        }
        assert_eq!(registry.handles.lock().unwrap().len(), MAX_HANDLES);
    }

    #[tokio::test]
    async fn local_assets_stream_selected_ranges_and_refuse_large_files() {
        let dir = tempfile::tempdir().unwrap();
        let registry = registry(dir.path());
        let directory = dir.path().join("apps/project/upload");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("asset.txt"), b"abcdefghij").unwrap();
        let target = registry.upload("asset.txt").unwrap();
        let mut request = request("/ui/assets?store=upload&path=asset.txt");
        request
            .headers_mut()
            .insert("range", "bytes=3-6".parse().unwrap());
        let (size, range, partial, mut source) = open(&target, request.headers()).await.unwrap();
        assert_eq!((size, range, partial), (10, 3..7, true));
        assert_eq!(source.next(4).await.unwrap(), &b"defg"[..]);
        request
            .headers_mut()
            .insert("range", "bytes=-2".parse().unwrap());
        let (_, range, _, mut source) = open(&target, request.headers()).await.unwrap();
        assert_eq!(range, 8..10);
        assert_eq!(source.next(2).await.unwrap(), &b"ij"[..]);
        for value in ["bytes=10-12", "bytes=4-1", "bytes=1-2,4-5", "bytes=-0"] {
            request
                .headers_mut()
                .insert("range", value.parse().unwrap());
            assert!(matches!(
                open(&target, request.headers()).await,
                Err(StatusCode::RANGE_NOT_SATISFIABLE)
            ));
        }
        std::fs::OpenOptions::new()
            .write(true)
            .open(directory.join("asset.txt"))
            .unwrap()
            .set_len(MAX_ASSET_BYTES + 1)
            .unwrap();
        assert!(matches!(
            open(&target, request.headers()).await,
            Err(StatusCode::PAYLOAD_TOO_LARGE)
        ));
    }

    #[tokio::test]
    async fn object_store_assets_preserve_range_and_project_scope() {
        let store = Arc::new(flow_like_storage::object_store::memory::InMemory::new());
        store
            .put(
                &ObjectPath::from("apps/project/upload/report.json"),
                bytes::Bytes::from_static(b"0123456789").into(),
            )
            .await
            .unwrap();
        let registry = Registry::new(
            "project",
            FlowLikeStores {
                app_storage_store: Some(FlowLikeStore::Memory(store)),
                ..Default::default()
            },
            None,
        );
        let target = registry.upload("report.json").unwrap();
        let mut request = request("/ui/assets?store=upload&path=report.json");
        request
            .headers_mut()
            .insert("range", "bytes=4-6".parse().unwrap());
        let (size, range, partial, mut source) = open(&target, request.headers()).await.unwrap();
        assert_eq!((size, range, partial), (10, 4..7, true));
        assert_eq!(source.next(3).await.unwrap(), &b"456"[..]);
        assert!(
            registry
                .target(Location {
                    kind: AssetStore::Upload,
                    path: "apps/other/upload/report.json".into()
                })
                .is_none()
        );
    }

    #[tokio::test]
    async fn upload_reads_cannot_follow_symlinks_or_hardlinks_outside_the_project() {
        let dir = tempfile::tempdir().unwrap();
        let registry = registry(dir.path());
        let directory = dir.path().join("apps/project/upload");
        std::fs::create_dir_all(&directory).unwrap();
        let private = dir.path().join("private");
        std::fs::create_dir(&private).unwrap();
        std::fs::write(private.join("secret"), b"secret").unwrap();
        std::os::unix::fs::symlink(&private, directory.join("linked")).unwrap();
        std::os::unix::fs::symlink(private.join("secret"), directory.join("symlink")).unwrap();
        std::fs::hard_link(private.join("secret"), directory.join("hardlink")).unwrap();
        for name in ["linked/secret", "symlink", "hardlink"] {
            let target = registry.upload(name).unwrap();
            assert!(
                matches!(
                    open(&target, request("/ui/assets").headers()).await,
                    Err(StatusCode::NOT_FOUND)
                ),
                "{name}"
            );
        }
    }
}

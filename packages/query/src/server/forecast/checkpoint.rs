//! The service's checkpoint cache. Each file is fetched once per content identity from the URL the
//! API handed over, checked against its size and BLAKE3 digest, and kept between requests
//! (Lambda's /tmp survives warm invocations). Least recently used files go first when the cache
//! would outgrow its cap. An interrupted download keeps its partial file, so the next request
//! continues where this one stopped. URLs can carry credentials and never reach an error.

use super::run::time_left;
use crate::protocol::forecast::{ForecastModelSpec, ModelFile};
use crate::protocol::{ErrorCode, WireError};
use crate::server::env_number;
use crate::server::ops::{internal, invalid};
use flow_like_runtime::flow::execution::ExecutionEnvironment;
use flow_like_runtime::flow::execution::egress::GuardedHttpClient;
use flow_like_storage::blake3;
use flow_like_types::reqwest::{self, StatusCode, header};
use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const LAMBDA_CACHE_BYTES: u64 = 1024 * 1024 * 1024;
const CACHE_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const MAX_FILES: usize = 256;
const MAX_NAME_BYTES: usize = 255;
const MAX_HASH_CHARS: usize = 256;
const ATTEMPTS: usize = 3;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const PARTIAL_SUFFIX: &str = ".part";

pub(super) struct CheckpointCache {
    directory: PathBuf,
    max_bytes: u64,
    environment: ExecutionEnvironment,
}

impl CheckpointCache {
    /// `QUERY_MODEL_CACHE_DIR` (default `<temp dir>/flow-like-query-models`), capped at
    /// `QUERY_MODEL_CACHE_MAX_BYTES`: 1 GiB on Lambda, whose /tmp also holds spill files, else
    /// 4 GiB. The cap is also the largest checkpoint this service runs.
    pub(super) fn from_env() -> Self {
        let directory = std::env::var_os("QUERY_MODEL_CACHE_DIR")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::temp_dir().join("flow-like-query-models"));
        let default_bytes = if std::env::var_os("AWS_LAMBDA_FUNCTION_MEMORY_SIZE").is_some() {
            LAMBDA_CACHE_BYTES
        } else {
            CACHE_BYTES
        };
        Self {
            directory,
            max_bytes: env_number("QUERY_MODEL_CACHE_MAX_BYTES").unwrap_or(default_bytes),
            environment: ExecutionEnvironment::server_default(),
        }
    }

    /// The model's files in one directory under their checkpoint names, fetching what the cache
    /// lacks before `deadline`. The directory lives as long as the returned guard.
    pub(super) async fn stage(
        &self,
        model: &ForecastModelSpec,
        deadline: tokio::time::Instant,
    ) -> Result<StagedCheckpoint, WireError> {
        validate(model)?;
        let total = model.total_bytes();
        if total > self.max_bytes {
            return Err(WireError::new(
                ErrorCode::ResourcesExhausted,
                format!(
                    "The forecasting model's checkpoint has {} MiB; this query service stages at \
                     most {} MiB (QUERY_MODEL_CACHE_MAX_BYTES)",
                    mib(total),
                    mib(self.max_bytes)
                ),
            ));
        }
        let files = self.directory.join("files");
        let wanted: Vec<(String, u64)> = model
            .files
            .iter()
            .map(|file| (file_key(file), file.size))
            .collect();
        let (directory, plan, max_bytes) = (self.directory.clone(), wanted.clone(), self.max_bytes);
        blocking(move || make_room(&directory, &plan, max_bytes))
            .await
            .map_err(cache_error)?;

        let client = GuardedHttpClient::configured(self.environment, |builder| {
            builder
                .connect_timeout(CONNECT_TIMEOUT)
                .user_agent("flow-like-query")
        })
        .map_err(|error| internal("Building the model download client failed", error))?;
        for (file, (key, _)) in model.files.iter().zip(&wanted) {
            let target = files.join(key);
            if is_complete(&target, file.size).await {
                continue;
            }
            let left = time_left(deadline, "the forecasting model was downloaded")?;
            tokio::time::timeout(left, fetch(&client, file, &target))
                .await
                .map_err(|_| {
                    WireError::new(
                        ErrorCode::Timeout,
                        format!(
                            "Downloading forecasting model file '{}' did not finish in time; \
                             retry to continue the download",
                            file.name
                        ),
                    )
                })??;
        }

        let staged = StagedCheckpoint {
            directory: self
                .directory
                .join("runs")
                .join(flow_like_types::create_id()),
        };
        let links: Vec<(PathBuf, String)> = model
            .files
            .iter()
            .zip(&wanted)
            .map(|(file, (key, _))| (files.join(key), file.name.clone()))
            .collect();
        let run = staged.directory.clone();
        blocking(move || link(&run, &links))
            .await
            .map_err(cache_error)?;
        Ok(staged)
    }
}

/// One request's checkpoint directory: links into the cache under the checkpoint's file names.
pub(super) struct StagedCheckpoint {
    directory: PathBuf,
}

impl StagedCheckpoint {
    pub(super) fn directory(&self) -> &Path {
        &self.directory
    }
}

impl Drop for StagedCheckpoint {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

fn mib(bytes: u64) -> u64 {
    bytes.div_ceil(1024 * 1024)
}

/// Cache file name of one checkpoint file: its content identity, never its URL.
fn file_key(file: &ModelFile) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"flow-like-query checkpoint file v1\0");
    hasher.update(file.hash.as_bytes());
    hasher.update(b"\0");
    hasher.update(file.name.as_bytes());
    hasher.update(b"\0");
    hasher.update(&file.size.to_le_bytes());
    hasher.update(&[u8::from(file.verify)]);
    hasher.finalize().to_hex().to_string()
}

fn plain_file_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME_BYTES
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\'])
        && !name.chars().any(char::is_control)
}

fn is_blake3_hex(hash: &str) -> bool {
    hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// The API resolved and bounded the checkpoint; this keeps a malformed one off the disk.
fn validate(model: &ForecastModelSpec) -> Result<(), WireError> {
    if model.files.is_empty() || model.files.len() > MAX_FILES {
        return Err(invalid(format!(
            "A forecasting checkpoint needs 1 to {MAX_FILES} files"
        )));
    }
    let mut names = HashSet::new();
    for file in &model.files {
        if !plain_file_name(&file.name) {
            return Err(invalid(format!(
                "'{}' is not a plain checkpoint file name",
                file.name.escape_debug()
            )));
        }
        if !names.insert(file.name.as_str()) {
            return Err(invalid(format!(
                "The checkpoint lists '{}' twice",
                file.name
            )));
        }
        if file.size == 0 {
            return Err(invalid(format!(
                "Checkpoint file '{}' has no size",
                file.name
            )));
        }
        if file.hash.is_empty() || file.hash.chars().count() > MAX_HASH_CHARS {
            return Err(invalid(format!(
                "Checkpoint file '{}' has no usable content hash",
                file.name
            )));
        }
        if file.verify && !is_blake3_hex(&file.hash) {
            return Err(invalid(format!(
                "Checkpoint file '{}' is marked verifiable but its hash is not a BLAKE3 digest",
                file.name
            )));
        }
        let scheme_ok = reqwest::Url::parse(&file.url)
            .is_ok_and(|url| matches!(url.scheme(), "http" | "https"));
        if !scheme_ok {
            return Err(invalid(format!(
                "Checkpoint file '{}' has no HTTP(S) download URL",
                file.name
            )));
        }
    }
    Ok(())
}

fn cache_error(error: io::Error) -> WireError {
    WireError::new(
        ErrorCode::Internal,
        format!(
            "The forecasting model cache failed: {error}; QUERY_MODEL_CACHE_DIR must be a writable \
             directory with room for the checkpoint"
        ),
    )
}

async fn blocking(work: impl FnOnce() -> io::Result<()> + Send + 'static) -> io::Result<()> {
    tokio::task::spawn_blocking(work)
        .await
        .unwrap_or_else(|error| Err(io::Error::other(error.to_string())))
}

fn partial_path(target: &Path) -> PathBuf {
    let mut name = target.as_os_str().to_owned();
    name.push(PARTIAL_SUFFIX);
    PathBuf::from(name)
}

async fn is_complete(target: &Path, size: u64) -> bool {
    tokio::fs::metadata(target)
        .await
        .is_ok_and(|metadata| metadata.is_file() && metadata.len() == size)
}

fn touch(path: &Path) -> io::Result<()> {
    std::fs::File::options()
        .write(true)
        .open(path)?
        .set_modified(SystemTime::now())
}

/// Clears run directories left by an earlier process (forecasts hold the process-wide slot, so
/// none is in use), marks the model's cached files as just used and evicts others until the
/// model's missing bytes fit under the cap.
fn make_room(directory: &Path, wanted: &[(String, u64)], max_bytes: u64) -> io::Result<()> {
    let files = directory.join("files");
    std::fs::create_dir_all(&files)?;
    match std::fs::remove_dir_all(directory.join("runs")) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
        _ => {}
    }
    let mut needed = 0_u64;
    for (key, size) in wanted {
        let complete = files.join(key);
        if std::fs::metadata(&complete).is_ok_and(|metadata| metadata.len() == *size) {
            touch(&complete)?;
            continue;
        }
        let partial = std::fs::metadata(partial_path(&complete))
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        needed = needed.saturating_add(size.saturating_sub(partial.min(*size)));
    }
    let keep: HashSet<&str> = wanted.iter().map(|(key, _)| key.as_str()).collect();
    evict(&files, &keep, needed, max_bytes)
}

/// Removes the least recently used files outside `keep` until `needed` more bytes fit.
fn evict(files: &Path, keep: &HashSet<&str>, needed: u64, max_bytes: u64) -> io::Result<()> {
    let mut usage = 0_u64;
    let mut candidates = Vec::new();
    for entry in std::fs::read_dir(files)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        if !metadata.is_file() {
            continue;
        }
        usage = usage.saturating_add(metadata.len());
        let name = entry.file_name().to_string_lossy().into_owned();
        let key = name.strip_suffix(PARTIAL_SUFFIX).unwrap_or(&name);
        if !keep.contains(key) {
            candidates.push((
                metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                entry.path(),
                metadata.len(),
            ));
        }
    }
    candidates.sort_by_key(|(modified, ..)| *modified);
    for (_, path, size) in candidates {
        if usage.saturating_add(needed) <= max_bytes {
            break;
        }
        std::fs::remove_file(&path)?;
        usage = usage.saturating_sub(size);
    }
    Ok(())
}

fn link(run: &Path, files: &[(PathBuf, String)]) -> io::Result<()> {
    std::fs::create_dir_all(run)?;
    for (source, name) in files {
        let target = run.join(name);
        if std::fs::hard_link(source, &target).is_err() {
            std::fs::copy(source, &target)?;
        }
    }
    Ok(())
}

/// Whether a retry may get further: a dropped connection may, a refusal or a bad file will not.
enum Failure {
    Transient(WireError),
    Final(WireError),
}

fn unavailable(file: &ModelFile, reason: impl std::fmt::Display) -> WireError {
    WireError::new(
        ErrorCode::Unavailable,
        format!(
            "Fetching forecasting model file '{}' failed: {reason}",
            file.name
        ),
    )
}

/// A transfer failure with its causes, never its URL.
fn transport(file: &ModelFile, error: reqwest::Error) -> Failure {
    let error = error.without_url();
    let mut reason = error.to_string();
    let mut cause = std::error::Error::source(&error);
    while let Some(inner) = cause {
        reason.push_str(": ");
        reason.push_str(&inner.to_string());
        cause = inner.source();
    }
    Failure::Transient(unavailable(file, reason))
}

fn local(error: io::Error) -> Failure {
    Failure::Final(cache_error(error))
}

/// `start` of a `Content-Range: bytes start-end/size` header.
fn content_range_start(headers: &header::HeaderMap) -> Option<u64> {
    headers
        .get(header::CONTENT_RANGE)?
        .to_str()
        .ok()?
        .strip_prefix("bytes ")?
        .split_once('-')?
        .0
        .trim()
        .parse()
        .ok()
}

/// Fetches `file` into `target`, resuming a partial file and retrying dropped transfers.
async fn fetch(
    client: &GuardedHttpClient,
    file: &ModelFile,
    target: &Path,
) -> Result<(), WireError> {
    let partial = partial_path(target);
    let mut outcome = Ok(());
    for _ in 0..ATTEMPTS {
        outcome = match attempt(client, file, &partial).await {
            Ok(()) => Ok(()),
            Err(Failure::Transient(error)) => Err(error),
            Err(Failure::Final(error)) => return Err(error),
        };
        if outcome.is_ok() {
            break;
        }
    }
    outcome?;
    tokio::fs::rename(&partial, target)
        .await
        .map_err(cache_error)
}

async fn attempt(
    client: &GuardedHttpClient,
    file: &ModelFile,
    partial: &Path,
) -> Result<(), Failure> {
    let mut written = match tokio::fs::metadata(partial).await {
        Ok(metadata) if metadata.len() <= file.size => metadata.len(),
        Ok(_) => {
            tokio::fs::remove_file(partial).await.map_err(local)?;
            0
        }
        Err(_) => 0,
    };
    let mut hasher = blake3::Hasher::new();
    if written > 0 && file.verify {
        hash_file(partial, &mut hasher).await.map_err(local)?;
    }
    if written < file.size {
        written = transfer(client, file, partial, written, &mut hasher).await?;
    }
    if written != file.size {
        return Err(Failure::Transient(unavailable(
            file,
            format!("the transfer ended after {written} of {} bytes", file.size),
        )));
    }
    if file.verify
        && !file
            .hash
            .eq_ignore_ascii_case(hasher.finalize().to_hex().as_str())
    {
        let _ = tokio::fs::remove_file(partial).await;
        return Err(Failure::Final(unavailable(
            file,
            "the bytes do not match the Bit's BLAKE3 digest",
        )));
    }
    Ok(())
}

async fn hash_file(path: &Path, hasher: &mut blake3::Hasher) -> io::Result<()> {
    let mut input = tokio::fs::File::open(path).await?;
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let read = input.read(&mut buffer).await?;
        if read == 0 {
            return Ok(());
        }
        hasher.update(&buffer[..read]);
    }
}

/// Appends the rest of `file` to `partial` (or rewrites it when the server cannot resume) and
/// returns how many bytes it now holds.
async fn transfer(
    client: &GuardedHttpClient,
    file: &ModelFile,
    partial: &Path,
    mut written: u64,
    hasher: &mut blake3::Hasher,
) -> Result<u64, Failure> {
    let mut request = client
        .get(&file.url)
        .map_err(|error| {
            Failure::Final(invalid(format!(
                "Forecasting model file '{}' has a download URL this service may not fetch: {error}",
                file.name
            )))
        })?
        .header(header::ACCEPT_ENCODING, "identity");
    if written > 0 {
        request = request.header(header::RANGE, format!("bytes={written}-"));
    }
    let mut response = request
        .send()
        .await
        .map_err(|error| transport(file, error))?;
    let status = response.status();
    if written > 0
        && !(status == StatusCode::PARTIAL_CONTENT
            && content_range_start(response.headers()) == Some(written))
    {
        if status == StatusCode::OK {
            written = 0;
            hasher.reset();
        } else if status.is_success() || status == StatusCode::RANGE_NOT_SATISFIABLE {
            tokio::fs::remove_file(partial).await.map_err(local)?;
            return Err(Failure::Transient(unavailable(
                file,
                "the server did not continue the interrupted download",
            )));
        }
    }
    if !status.is_success() {
        let error = unavailable(file, format!("the server answered HTTP {status}"));
        return Err(if status.is_server_error() {
            Failure::Transient(error)
        } else {
            Failure::Final(error)
        });
    }
    let expected = file.size - written;
    if let Some(announced) = response.content_length()
        && announced != expected
    {
        let _ = tokio::fs::remove_file(partial).await;
        return Err(Failure::Final(unavailable(
            file,
            format!("the server announced {announced} bytes where {expected} were expected"),
        )));
    }
    let mut output = if written == 0 {
        tokio::fs::File::create(partial).await
    } else {
        tokio::fs::OpenOptions::new()
            .append(true)
            .open(partial)
            .await
    }
    .map_err(local)?;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| transport(file, error))?
    {
        let Some(total) = written
            .checked_add(chunk.len() as u64)
            .filter(|total| *total <= file.size)
        else {
            drop(output);
            let _ = tokio::fs::remove_file(partial).await;
            return Err(Failure::Final(unavailable(
                file,
                "the server sent more bytes than the file has",
            )));
        };
        if file.verify {
            hasher.update(&chunk);
        }
        output.write_all(&chunk).await.map_err(local)?;
        written = total;
    }
    output.flush().await.map_err(local)?;
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread::JoinHandle;

    fn temp_dir() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "flow-like-query-models-{}",
            flow_like_types::create_id()
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn file(name: &str, bytes: &[u8], verify: bool, url: String) -> ModelFile {
        ModelFile {
            name: name.to_string(),
            hash: if verify {
                blake3::hash(bytes).to_hex().to_string()
            } else {
                format!("{name}-identity")
            },
            verify,
            size: bytes.len() as u64,
            url,
        }
    }

    fn model(files: Vec<ModelFile>) -> ForecastModelSpec {
        ForecastModelSpec {
            bit_id: "model".to_string(),
            family: "tiny_time_mixer".to_string(),
            repository: None,
            revision: None,
            files,
        }
    }

    fn cache_at(directory: &Path, max_bytes: u64) -> CheckpointCache {
        CheckpointCache {
            directory: directory.to_path_buf(),
            max_bytes,
            environment: ExecutionEnvironment::Local,
        }
    }

    fn deadline() -> tokio::time::Instant {
        tokio::time::Instant::now() + Duration::from_secs(20)
    }

    /// Serves fixed bodies by path and honors `Range: bytes=N-`.
    struct FileServer {
        base: String,
        gets: Arc<AtomicUsize>,
        ranges: Arc<Mutex<Vec<String>>>,
        stop: Arc<AtomicBool>,
        worker: Option<JoinHandle<()>>,
    }

    impl FileServer {
        fn new(files: Vec<(&'static str, Vec<u8>)>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let gets = Arc::new(AtomicUsize::new(0));
            let ranges = Arc::new(Mutex::new(Vec::new()));
            let stop = Arc::new(AtomicBool::new(false));
            let worker = {
                let (gets, ranges, stop) = (gets.clone(), ranges.clone(), stop.clone());
                std::thread::spawn(move || {
                    while !stop.load(Ordering::Relaxed) {
                        let (mut stream, _) = match listener.accept() {
                            Ok(connection) => connection,
                            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                                std::thread::sleep(Duration::from_millis(5));
                                continue;
                            }
                            Err(error) => panic!("file server failed: {error}"),
                        };
                        stream.set_nonblocking(false).unwrap();
                        let mut reader = BufReader::new(stream.try_clone().unwrap());
                        let mut line = String::new();
                        reader.read_line(&mut line).unwrap();
                        let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
                        let mut start = 0_usize;
                        loop {
                            line.clear();
                            if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                                break;
                            }
                            if let Some(range) = line
                                .to_ascii_lowercase()
                                .strip_prefix("range: bytes=")
                                .map(|value| value.trim().trim_end_matches('-').to_string())
                            {
                                ranges.lock().unwrap().push(range.clone());
                                start = range.parse().unwrap();
                            }
                        }
                        gets.fetch_add(1, Ordering::Relaxed);
                        let Some((_, body)) =
                            files.iter().find(|(name, _)| path == format!("/{name}"))
                        else {
                            write!(stream, "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
                            continue;
                        };
                        if start > 0 {
                            write!(
                                stream,
                                "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {start}-{}/{}\r\nConnection: close\r\n\r\n",
                                body.len() - start,
                                body.len() - 1,
                                body.len()
                            )
                            .unwrap();
                        } else {
                            write!(
                                stream,
                                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                body.len()
                            )
                            .unwrap();
                        }
                        stream.write_all(&body[start..]).unwrap();
                    }
                })
            };
            Self {
                base,
                gets,
                ranges,
                stop,
                worker: Some(worker),
            }
        }

        fn url(&self, name: &str) -> String {
            format!("{}/{name}", self.base)
        }
    }

    impl Drop for FileServer {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            let result = self.worker.take().unwrap().join();
            if !std::thread::panicking() {
                result.unwrap();
            }
        }
    }

    #[test]
    fn keys_follow_content_identity_not_urls() {
        let a = file(
            "config.json",
            b"{}",
            true,
            "https://a.example/x?sig=1".to_string(),
        );
        let mut b = a.clone();
        b.url = "https://b.example/y?sig=2".to_string();
        assert_eq!(file_key(&a), file_key(&b));
        b.size += 1;
        assert_ne!(file_key(&a), file_key(&b));
        let mut c = a.clone();
        c.verify = false;
        assert_ne!(file_key(&a), file_key(&c));
        assert_eq!(file_key(&a).len(), 64);
    }

    #[test]
    fn malformed_checkpoints_are_refused_before_touching_the_disk() {
        let good = file(
            "config.json",
            b"{}",
            true,
            "https://hub.example/c".to_string(),
        );
        assert!(validate(&model(vec![good.clone()])).is_ok());
        assert!(validate(&model(vec![])).is_err());
        for name in [
            "",
            ".",
            "..",
            "../config.json",
            "a/b",
            "a\\b",
            "line\nbreak",
        ] {
            let mut bad = good.clone();
            bad.name = name.to_string();
            assert!(validate(&model(vec![bad])).is_err(), "{name:?}");
        }
        assert!(validate(&model(vec![good.clone(), good.clone()])).is_err());
        let mut empty = good.clone();
        empty.size = 0;
        assert!(validate(&model(vec![empty])).is_err());
        let mut unhashed = good.clone();
        unhashed.hash = "not-a-digest".to_string();
        assert!(validate(&model(vec![unhashed.clone()])).is_err());
        unhashed.verify = false;
        assert!(validate(&model(vec![unhashed])).is_ok());
        for url in ["file:///etc/passwd", "s3://bucket/key", "not a url"] {
            let mut local = good.clone();
            local.url = url.to_string();
            let error = validate(&model(vec![local])).unwrap_err();
            assert!(!error.message.contains(url), "{}", error.message);
        }
    }

    #[test]
    fn content_ranges_yield_their_start() {
        let mut headers = header::HeaderMap::new();
        assert_eq!(content_range_start(&headers), None);
        headers.insert(
            header::CONTENT_RANGE,
            header::HeaderValue::from_static("bytes 100-199/200"),
        );
        assert_eq!(content_range_start(&headers), Some(100));
        headers.insert(
            header::CONTENT_RANGE,
            header::HeaderValue::from_static("bytes */200"),
        );
        assert_eq!(content_range_start(&headers), None);
    }

    #[test]
    fn eviction_drops_the_least_recently_used_files_outside_the_model() {
        let directory = temp_dir();
        let files = directory.join("files");
        std::fs::create_dir_all(&files).unwrap();
        let now = SystemTime::now();
        for (name, age) in [
            ("old", 30),
            ("newer", 20),
            ("kept", 40),
            ("newest.part", 10),
        ] {
            let path = files.join(name);
            std::fs::write(&path, [0_u8; 10]).unwrap();
            std::fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(now - Duration::from_secs(age))
                .unwrap();
        }
        let keep = HashSet::from(["kept"]);
        evict(&files, &keep, 15, 40).unwrap();
        let mut left: Vec<String> = std::fs::read_dir(&files)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left, vec!["kept", "newest.part"]);
        std::fs::remove_dir_all(directory).ok();
    }

    #[tokio::test]
    async fn checkpoints_download_once_resume_and_verify() {
        let config = b"{\"family\":\"tiny\"}".to_vec();
        let weights: Vec<u8> = (0..64_u8).collect();
        let shard: Vec<u8> = (0..32_u8).rev().collect();
        let server = FileServer::new(vec![
            ("config.json", config.clone()),
            ("weights", weights.clone()),
            ("shard", shard.clone()),
            ("tampered", vec![9_u8; 8]),
        ]);
        let directory = temp_dir();
        let cache = cache_at(&directory, 1024);
        let spec = model(vec![
            file("config.json", &config, true, server.url("config.json")),
            file("model.safetensors", &weights, false, server.url("weights")),
        ]);

        let staged = cache.stage(&spec, deadline()).await.unwrap();
        assert_eq!(
            std::fs::read(staged.directory().join("config.json")).unwrap(),
            config
        );
        assert_eq!(
            std::fs::read(staged.directory().join("model.safetensors")).unwrap(),
            weights
        );
        let run = staged.directory().to_path_buf();
        drop(staged);
        assert!(!run.exists());
        assert_eq!(server.gets.load(Ordering::Relaxed), 2);
        drop(cache.stage(&spec, deadline()).await.unwrap());
        assert_eq!(
            server.gets.load(Ordering::Relaxed),
            2,
            "cached files are not fetched again"
        );

        let resumed = file("shard.safetensors", &shard, true, server.url("shard"));
        let target = directory.join("files").join(file_key(&resumed));
        std::fs::write(partial_path(&target), &shard[..10]).unwrap();
        let staged = cache
            .stage(&model(vec![resumed]), deadline())
            .await
            .unwrap();
        assert_eq!(
            std::fs::read(staged.directory().join("shard.safetensors")).unwrap(),
            shard
        );
        assert_eq!(server.ranges.lock().unwrap().as_slice(), ["10"]);
        drop(staged);

        let tampered = file("bad.safetensors", &[7_u8; 8], true, server.url("tampered"));
        let error = cache
            .stage(&model(vec![tampered.clone()]), deadline())
            .await
            .err()
            .unwrap();
        assert_eq!(error.code, ErrorCode::Unavailable);
        assert!(error.message.contains("BLAKE3"), "{}", error.message);
        assert!(!error.message.contains(&server.base));
        let target = directory.join("files").join(file_key(&tampered));
        assert!(!target.exists() && !partial_path(&target).exists());

        let missing = file("gone.json", b"{}", false, server.url("missing"));
        let error = cache
            .stage(&model(vec![missing]), deadline())
            .await
            .err()
            .unwrap();
        assert!(error.message.contains("HTTP 404"), "{}", error.message);
        assert!(!error.message.contains(&server.base));

        let before = server.gets.load(Ordering::Relaxed);
        let error = CheckpointCache {
            max_bytes: 16,
            ..cache
        }
        .stage(&spec, deadline())
        .await
        .err()
        .unwrap();
        assert_eq!(error.code, ErrorCode::ResourcesExhausted);
        assert_eq!(server.gets.load(Ordering::Relaxed), before);
        std::fs::remove_dir_all(directory).ok();
    }
}

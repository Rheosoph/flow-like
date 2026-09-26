use crate::state::FlowLikeState;
use flow_like_storage::files::store::FlowLikeStore;
use flow_like_storage::{Path, blake3};
use flow_like_types::intercom::{InterComCallback, InterComEvent};
use flow_like_types::reqwest::Client;
use flow_like_types::tokio::fs::{self as async_fs, OpenOptions};
use flow_like_types::tokio::io::{AsyncReadExt, AsyncWriteExt, BufWriter};
use flow_like_types::tokio::spawn;
use flow_like_types::tokio::sync::Semaphore;
use flow_like_types::tokio::task::yield_now;
use flow_like_types::tokio::time::Instant;
use flow_like_types::{anyhow, bail, reqwest};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Duration;

#[derive(Serialize, Deserialize, Clone)]
pub struct BitDownloadEvent {
    pub max: u64,
    pub downloaded: u64,
    pub path: String,
    pub hash: String,
}

/// Filesystem targets for a single artifact.
struct ArtifactPaths {
    store: Path,
    file: PathBuf,
    temp: PathBuf,
}

// Global concurrency limit for active downloads.
fn global_download_semaphore() -> &'static Semaphore {
    static SEM: OnceLock<Semaphore> = OnceLock::new();
    SEM.get_or_init(|| {
        let max = std::env::var("FLOW_LIKE_MAX_CONCURRENT_DOWNLOADS")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|&n| n > 0)
            .unwrap_or(10);
        Semaphore::new(max)
    })
}

/// Announced length of the remote artifact.
///
/// HEAD is the cheap probe, but gateways and proxies regularly reject the
/// method or answer it with an interstitial, so a failed HEAD is retried as a
/// single-byte ranged GET before the artifact is declared unreachable.
async fn get_remote_size(client: &Client, url: &str) -> flow_like_types::Result<u64> {
    let head_error = match head_remote_size(client, url).await {
        Ok(size) => return Ok(size),
        Err(err) => err,
    };

    match range_probe_remote_size(client, url).await {
        Ok(size) => Ok(size),
        Err(range_error) => bail!("HEAD probe: {head_error}; ranged GET probe: {range_error}"),
    }
}

async fn head_remote_size(client: &Client, url: &str) -> flow_like_types::Result<u64> {
    // Transfer compression hides the real artifact length: a Brotli/gzip
    // response carries no content-length, and byte ranges would refer to the
    // encoded stream instead of the stored file.
    let res = client
        .head(url)
        .header(reqwest::header::ACCEPT_ENCODING, "identity")
        .send()
        .await
        .map_err(reqwest::Error::without_url)?;

    // Block pages, captive portals and auth challenges are well-formed
    // responses; only a success status describes the artifact itself.
    if !res.status().is_success() {
        bail!("server responded with {}", res.status());
    }

    let total_size = res
        .headers()
        .get(reqwest::header::CONTENT_LENGTH)
        .ok_or(anyhow!("response carried no content-length"))?
        .to_str()?
        .trim()
        .parse::<u64>()?;

    Ok(total_size)
}

async fn range_probe_remote_size(client: &Client, url: &str) -> flow_like_types::Result<u64> {
    let res = client
        .get(url)
        .header(reqwest::header::ACCEPT_ENCODING, "identity")
        .header(reqwest::header::RANGE, "bytes=0-0")
        .send()
        .await
        .map_err(reqwest::Error::without_url)?;

    if !res.status().is_success() {
        bail!("server responded with {}", res.status());
    }

    let content_range = res
        .headers()
        .get(reqwest::header::CONTENT_RANGE)
        .ok_or(anyhow!("response carried no content-range"))?
        .to_str()?
        .to_string();

    parse_total_from_content_range(&content_range)
}

/// Total artifact length out of a `bytes 0-0/1234` content-range header.
fn parse_total_from_content_range(content_range: &str) -> flow_like_types::Result<u64> {
    let total = content_range
        .rsplit('/')
        .next()
        .map(str::trim)
        .filter(|total| !total.is_empty() && *total != "*")
        .ok_or(anyhow!(
            "content-range {content_range} carries no total length"
        ))?;

    Ok(total.parse::<u64>()?)
}

/// Reject a response whose announced length cannot be the artifact.
///
/// The declared size is the only description of the artifact available before
/// the first byte is fetched, which makes it the one check that separates the
/// real file from an error page served with a success status.
fn validate_announced_size(expected: Option<u64>, announced: u64) -> flow_like_types::Result<()> {
    let Some(expected) = expected.filter(|size| *size > 0) else {
        return Ok(());
    };

    if announced != expected {
        bail!("server announced {announced} bytes, expected {expected}");
    }

    Ok(())
}

fn checked_download_size(
    downloaded: u64,
    chunk: usize,
    expected: u64,
) -> flow_like_types::Result<u64> {
    downloaded
        .checked_add(chunk as u64)
        .filter(|total| *total <= expected)
        .ok_or_else(|| anyhow!("Artifact transfer exceeds its declared size"))
}

/// Whether the artifact can be checked against a known content hash.
///
/// User-created custom bits don't know their artifact hash upfront. Legacy
/// roots use `hash == id`; newer GGUF roots use a source-derived identity so
/// edits to pinned URLs select a fresh cache target. Both are trust-on-first-use.
fn hash_is_verifiable(bit: &crate::bit::Bit) -> bool {
    !(bit.hash == bit.id || bit.has_matching_user_source_artifact_identity())
}

fn verify_hash(bit: &crate::bit::Bit, hasher: &blake3::Hasher) -> flow_like_types::Result<()> {
    if !hash_is_verifiable(bit) {
        return Ok(());
    }

    let file_hash = hasher.finalize().to_hex().to_string().to_lowercase();
    let expected = bit.hash.to_lowercase();
    if file_hash != expected {
        bail!("content hash {file_hash} does not match {expected}");
    }

    Ok(())
}

async fn publish_progress(
    bit: &crate::bit::Bit,
    callback: &InterComCallback,
    downloaded: u64,
    path: &Path,
) -> flow_like_types::Result<()> {
    let event = InterComEvent::with_type(
        format!("download:{}", bit.hash),
        BitDownloadEvent {
            hash: bit.hash.to_string(),
            max: bit.size.unwrap_or(0),
            downloaded,
            path: path.to_string(),
        },
    );

    if let Err(err) = event.call(callback).await {
        tracing::warn!("Error publishing progress: {}", err);
    }

    Ok(())
}

async fn feed_hasher_with_existing(
    path: &std::path::Path,
    hasher: &mut blake3::Hasher,
) -> flow_like_types::Result<u64> {
    let mut f = async_fs::File::open(path).await?;
    let mut buf = vec![0u8; 1024 * 1024];
    let mut total = 0u64;

    loop {
        let n = f.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        total += n as u64;

        // yield occasionally to keep the runtime responsive (Windows)
        if total.is_multiple_of(8 * 1024 * 1024) {
            yield_now().await;
        }
    }

    Ok(total)
}

async fn remove_download(bit: &crate::bit::Bit, app_state: &Arc<FlowLikeState>) {
    let manager = app_state.download_manager();
    manager.lock().await.remove_download(bit);
}

pub async fn download_bit(
    bit: &crate::bit::Bit,
    app_state: Arc<FlowLikeState>,
    retries: usize,
    callback: &InterComCallback,
) -> flow_like_types::Result<Path> {
    let bit = bit.clone();
    let callback = callback.clone();

    let handle = spawn(async move {
        let _permit = global_download_semaphore()
            .acquire()
            .await
            .map_err(|_| anyhow!("Download limiter closed"))?;

        process_download_bit(&bit, app_state, retries, &callback).await
    });

    handle
        .await
        .map_err(|err| anyhow!("Download worker dropped: {}", err))?
}

/// Wait out a download of the same artifact started by someone else.
///
/// Two packs can name the same tokenizer or config file, so a request for an
/// artifact that is already in flight belongs to the first caller. The second
/// one waits for it instead of reporting a failure, and the progress events the
/// owner publishes are keyed by hash, so both packs still see them.
async fn await_concurrent_download(
    bit: &crate::bit::Bit,
    app_state: &Arc<FlowLikeState>,
    file_path: &std::path::Path,
) -> flow_like_types::Result<()> {
    const POLL_INTERVAL: Duration = Duration::from_millis(250);
    const MAX_WAIT: Duration = Duration::from_secs(60 * 60);

    tracing::debug!(
        "Waiting for an in-flight download of the artifact of bit {}",
        bit.id
    );

    let started = Instant::now();
    loop {
        if started.elapsed() > MAX_WAIT {
            bail!(
                "Timed out waiting for the in-flight download of bit {}",
                bit.id
            );
        }

        flow_like_types::tokio::time::sleep(POLL_INTERVAL).await;

        let in_flight = {
            let manager = app_state.download_manager();
            let manager = manager.lock().await;
            manager.download_exists(bit)
        };

        if !in_flight {
            break;
        }
    }

    let installed = async_fs::metadata(file_path)
        .await
        .is_ok_and(|meta| bit.size.is_none_or(|size| size == meta.len()));

    if !installed {
        bail!(
            "The concurrent download of bit {} did not produce the artifact",
            bit.id
        );
    }

    Ok(())
}

async fn process_download_bit(
    bit: &crate::bit::Bit,
    app_state: Arc<FlowLikeState>,
    retries: usize,
    callback: &InterComCallback,
) -> flow_like_types::Result<Path> {
    tracing::debug!("Processing download for: {}", bit.hash);
    let file_store = match FlowLikeState::bit_store(&app_state).await? {
        FlowLikeStore::Local(store) => store,
        _ => bail!("Only local store supported"),
    };

    let file_name = bit.file_name.clone().ok_or(anyhow!("No file name"))?;
    let store_path = Path::from(bit.hash.clone()).join(file_name);
    let file_path = file_store.path_to_filesystem(&store_path)?;
    let temp_extension = file_path
        .extension()
        .map(|ext| format!("{}.download", ext.to_string_lossy()))
        .unwrap_or_else(|| "download".to_string());
    let paths = ArtifactPaths {
        store: store_path,
        temp: file_path.with_extension(temp_extension),
        file: file_path,
    };
    let url = bit
        .download_link
        .clone()
        .ok_or(anyhow!("No download link"))?;

    // Registering under one lock keeps two callers from both believing they own
    // the artifact. A caller that loses the race must not unregister the winner.
    let client = {
        let manager = app_state.download_manager();
        let mut manager = manager.lock().await;
        if manager.download_exists(bit) {
            None
        } else {
            manager.add_download(bit)
        }
    };

    let Some(client) = client else {
        return await_concurrent_download(bit, &app_state, &paths.file)
            .await
            .map(|()| paths.store.clone());
    };

    // Every exit from here has to unregister: a leaked entry makes every later
    // attempt in this process bail out instead of downloading anything.
    let result = download_artifact(bit, &url, &paths, &client, retries, callback).await;
    remove_download(bit, &app_state).await;
    result
}

async fn download_artifact(
    bit: &crate::bit::Bit,
    url: &str,
    paths: &ArtifactPaths,
    client: &Client,
    retries: usize,
    callback: &InterComCallback,
) -> flow_like_types::Result<Path> {
    let cached_size = async_fs::metadata(&paths.file)
        .await
        .ok()
        .map(|meta| meta.len());

    let remote_size = match get_remote_size(client, url).await {
        Ok(size) => size,
        Err(err) => {
            // A complete cached copy is the only thing that may stand in for a
            // download. Without one this is a failure and has to read as one.
            let cached_is_complete = cached_size
                .is_some_and(|cached| cached > 0 && bit.size.is_none_or(|size| size == cached));

            if cached_is_complete {
                tracing::warn!(
                    "Could not reach {} for bit {}: {}. Using the cached artifact instead.",
                    url,
                    bit.id,
                    err
                );
                let _ =
                    publish_progress(bit, callback, cached_size.unwrap_or_default(), &paths.store)
                        .await;
                return Ok(paths.store.clone());
            }

            bail!(
                "Could not determine the remote size of bit {} and no cached copy exists: {}",
                bit.id,
                err
            );
        }
    };

    validate_announced_size(bit.size, remote_size).map_err(|err| {
        anyhow!(
            "Refusing to download bit {}, the response does not describe the artifact: {}",
            bit.id,
            err
        )
    })?;

    if cached_size == Some(remote_size) {
        let _ = publish_progress(bit, callback, remote_size, &paths.store).await;
        return Ok(paths.store.clone());
    }

    tracing::debug!("Downloading bit {} to {}", bit.id, paths.store);

    let mut attempt = 0usize;
    loop {
        match fetch_artifact(bit, url, paths, client, remote_size, callback).await {
            Ok(()) => break,
            Err(err) if attempt < retries => {
                attempt += 1;
                tracing::warn!(
                    "Download attempt {} for bit {} failed, retrying: {}",
                    attempt,
                    bit.id,
                    err
                );
            }
            Err(err) => return Err(err),
        }
    }

    if async_fs::try_exists(&paths.file).await.unwrap_or(false) {
        let _ = async_fs::remove_file(&paths.file).await;
    }

    async_fs::rename(&paths.temp, &paths.file)
        .await
        .map_err(|err| {
            anyhow!(
                "Could not install bit {} at {}: {}",
                bit.id,
                paths.file.display(),
                err
            )
        })?;

    let _ = publish_progress(bit, callback, remote_size, &paths.store).await;
    Ok(paths.store.clone())
}

/// A single attempt at fetching the artifact.
///
/// Returns once the temporary file holds the complete, verified artifact; the
/// caller moves it into place.
async fn fetch_artifact(
    bit: &crate::bit::Bit,
    url: &str,
    paths: &ArtifactPaths,
    client: &Client,
    remote_size: u64,
    callback: &InterComCallback,
) -> flow_like_types::Result<()> {
    let mut hasher = blake3::Hasher::new();
    let mut downloaded = 0u64;
    let mut resume = false;

    let partial_size = async_fs::metadata(&paths.temp)
        .await
        .ok()
        .map(|meta| meta.len())
        .unwrap_or(0);

    if partial_size > 0 && partial_size == remote_size {
        let mut complete = blake3::Hasher::new();
        feed_hasher_with_existing(&paths.temp, &mut complete).await?;
        if verify_hash(bit, &complete).is_ok() {
            return Ok(());
        }
        let _ = async_fs::remove_file(&paths.temp).await;
    } else if partial_size > 0 && partial_size < remote_size {
        feed_hasher_with_existing(&paths.temp, &mut hasher).await?;
        downloaded = partial_size;
        resume = true;
        tracing::debug!(
            "Resuming download to {} ({} bytes already present)",
            paths.temp.display(),
            partial_size
        );
    } else if partial_size > 0 {
        let _ = async_fs::remove_file(&paths.temp).await;
    }

    let mut headers = reqwest::header::HeaderMap::new();
    // Keep the response identical to the stored artifact so byte offsets,
    // resume ranges and the size check all describe the same stream.
    headers.insert(reqwest::header::ACCEPT_ENCODING, "identity".parse()?);
    if resume {
        headers.insert(
            reqwest::header::RANGE,
            format!("bytes={}-", downloaded).parse()?,
        );
    }

    let res = client
        .get(url)
        .headers(headers)
        .send()
        .await
        .map_err(|err| anyhow!("Request for bit {} failed: {}", bit.id, err.without_url()))?;

    if !res.status().is_success() {
        bail!(
            "Server responded with {} when downloading bit {}",
            res.status(),
            bit.id
        );
    }

    // A server that ignores the range restarts the stream, and appending that
    // to the partial file would silently concatenate two copies.
    if resume && res.status() != reqwest::StatusCode::PARTIAL_CONTENT {
        resume = false;
        downloaded = 0;
        hasher = blake3::Hasher::new();
    }

    if let Some(announced) = res.content_length() {
        let expected = remote_size.saturating_sub(downloaded);
        if announced != expected {
            bail!(
                "Server offered {} bytes of bit {}, expected {}",
                announced,
                bit.id,
                expected
            );
        }
    }

    if let Some(parent) = paths.file.parent() {
        async_fs::create_dir_all(parent).await?;
    }

    let file = OpenOptions::new()
        .write(true)
        .append(resume)
        .truncate(!resume)
        .create(true)
        .open(&paths.temp)
        .await
        .map_err(|err| anyhow!("Could not open {}: {}", paths.temp.display(), err))?;

    let mut file = BufWriter::with_capacity(1 << 20, file);
    let mut stream = res.bytes_stream();
    let mut in_buffer = 0;
    let mut since_yield = 0usize;
    let mut last_emit = Instant::now();

    while let Some(item) = stream.next().await {
        let chunk = item.map_err(|err| {
            anyhow!(
                "Transfer of bit {} failed after {} bytes: {}",
                bit.id,
                downloaded,
                err.without_url()
            )
        })?;

        // Enforce the response bound before writing, including resumed transfers
        // and chunked responses whose Content-Length cannot be trusted.
        let next_size = checked_download_size(downloaded, chunk.len(), remote_size)?;
        // Written before hashed, so the digest describes the bytes that reached
        // the disk rather than the ones that arrived from the network.
        file.write_all(&chunk)
            .await
            .map_err(|err| anyhow!("Could not write to {}: {}", paths.temp.display(), err))?;
        hasher.update(&chunk);

        in_buffer += chunk.len();
        since_yield += chunk.len();
        downloaded = next_size;

        // if buffer is bigger than 20 mb flush
        if in_buffer > 20_000_000 && file.flush().await.is_ok() {
            in_buffer = 0;
        }

        if last_emit.elapsed() >= Duration::from_millis(150) {
            let _ = publish_progress(bit, callback, downloaded, &paths.store).await;
            last_emit = Instant::now();
        }

        if since_yield >= 8 * 1024 * 1024 {
            yield_now().await;
            since_yield = 0;
        }
    }

    file.flush()
        .await
        .map_err(|err| anyhow!("Could not flush {}: {}", paths.temp.display(), err))?;
    file.get_mut()
        .sync_all()
        .await
        .map_err(|err| anyhow!("Could not sync {}: {}", paths.temp.display(), err))?;
    // Windows refuses to rename or delete a file that is still open.
    drop(file);

    let written = async_fs::metadata(&paths.temp).await?.len();
    if written != remote_size {
        bail!(
            "Transfer of bit {} ended after {} of {} bytes",
            bit.id,
            written,
            remote_size
        );
    }

    if let Err(err) = verify_hash(bit, &hasher) {
        let _ = async_fs::remove_file(&paths.temp).await;
        bail!(
            "Downloaded content of bit {} is not the artifact: {}",
            bit.id,
            err
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oversized_or_overflowing_chunks_are_rejected_before_they_can_be_written() {
        assert_eq!(checked_download_size(7, 3, 10).unwrap(), 10);
        assert!(checked_download_size(7, 4, 10).is_err());
        assert!(checked_download_size(0, 11, 10).is_err());
        assert!(checked_download_size(u64::MAX, 1, u64::MAX).is_err());
    }

    #[test]
    fn announced_size_must_match_the_declared_artifact_size() {
        assert!(validate_announced_size(Some(340_318_797), 340_318_797).is_ok());
        // A gateway block page served with a success status.
        assert!(validate_announced_size(Some(340_318_797), 20).is_err());
    }

    #[test]
    fn announced_size_without_a_declared_size_is_accepted() {
        assert!(validate_announced_size(None, 20).is_ok());
        assert!(validate_announced_size(Some(0), 20).is_ok());
    }

    #[test]
    fn content_range_yields_the_total_length() {
        assert_eq!(
            parse_total_from_content_range("bytes 0-0/340318797").unwrap(),
            340_318_797
        );
        assert!(parse_total_from_content_range("bytes 0-0/*").is_err());
        assert!(parse_total_from_content_range("bytes 0-0").is_err());
    }
}

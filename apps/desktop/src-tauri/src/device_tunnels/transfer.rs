use std::{
    collections::HashMap,
    future::Future,
    sync::{Arc, LazyLock, Mutex, PoisonError},
    time::Duration,
};

use flow_like::app::sharing::device::MAX_DEVICE_EXPORT_CHUNK;
use flow_like_device_client::TunnelDataOpen;
use serde::Deserialize;
use serde_json::Value;
use tauri::AppHandle;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    sync::watch,
};
use zeroize::Zeroize;

use crate::functions::app::device_export::{export_snapshot, local_bit_file, read_range};

/// The device granted no credit for this long: the transfer failed.
const IDLE: Duration = Duration::from_secs(60);
/// The device checks a file after its last byte; a model asset of many GiB takes a while.
const VERIFY: Duration = Duration::from_secs(15 * 60);
/// The device reads a model asset back whole to verify it: 10 MB/s, a slow disk's pace (as the
/// web client's `DataOperation.verification` allows).
const VERIFY_BYTES_PER_SECOND: u64 = 10_000_000;
const MAX_ANSWER: u64 = 64 * 1024;

/// Reads `length` bytes at an offset; runs off the async runtime.
type ReadAt = Arc<dyn Fn(u64, usize) -> anyhow::Result<Vec<u8>> + Send + Sync>;

static TRANSFERS: LazyLock<Mutex<HashMap<String, watch::Sender<bool>>>> =
    LazyLock::new(Mutex::default);

/// One file of a prepared export, sent from `offset` to its end.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ArtifactUpload {
    device_id: String,
    export_id: String,
    path: String,
    project_id: String,
    transfer_id: String,
    file_index: u32,
    offset: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ModelAssetPush {
    device_id: String,
    job_id: String,
    offset: u64,
    /// Absent: the stream carries no bytes and only opens the device's push session.
    file: Option<BitStoreFile>,
}

/// A model file in this computer's Bit store, `<hash>/<file_name>`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BitStoreFile {
    hash: String,
    file_name: String,
    size: u64,
}

fn with_transfers<T>(
    use_transfers: impl FnOnce(&mut HashMap<String, watch::Sender<bool>>) -> T,
) -> T {
    use_transfers(&mut TRANSFERS.lock().unwrap_or_else(PoisonError::into_inner))
}

fn signal<T>(
    transfer: &str,
    use_signal: impl FnOnce(&watch::Sender<bool>) -> T,
) -> Result<T, String> {
    uuid::Uuid::parse_str(transfer)
        .map_err(|_| format!("Transfer id {transfer:?} is not a UUID."))?;
    Ok(with_transfers(|transfers| {
        use_signal(
            transfers
                .entry(transfer.to_owned())
                .or_insert_with(|| watch::channel(false).0),
        )
    }))
}

/// Also before the transfer started: it then ends at once.
pub(super) fn cancel(transfer: &str) -> Result<(), String> {
    signal(transfer, |signal| {
        signal.send_replace(true);
    })
}

struct Registered(String);

impl Drop for Registered {
    fn drop(&mut self) {
        with_transfers(|transfers| transfers.remove(&self.0));
    }
}

/// Runs a transfer until it ends or is cancelled; dropping its stream resets it on the device.
async fn cancellable<T>(
    transfer: &str,
    run: impl Future<Output = Result<T, String>>,
) -> Result<T, String> {
    let mut cancelled = signal(transfer, watch::Sender::subscribe)?;
    let _registered = Registered(transfer.to_owned());
    tokio::select! {
        result = run => result,
        Ok(_) = cancelled.wait_for(|stop| *stop) => Err("the transfer was cancelled".into()),
    }
}

pub(super) async fn upload(
    app: &AppHandle,
    transfer: &str,
    upload: ArtifactUpload,
) -> Result<Value, String> {
    let ArtifactUpload {
        device_id,
        export_id,
        path,
        project_id,
        transfer_id,
        file_index,
        offset,
    } = upload;
    let failed = |error: String| format!("Uploading {path} to device {device_id} failed: {error}");
    let snapshot = export_snapshot(app, &export_id)
        .await
        .map_err(|error| failed(error.to_string()))?;
    let size = snapshot
        .files()
        .into_iter()
        .find(|file| file.path == path)
        .map(|file| file.size)
        .ok_or_else(|| failed(format!("prepared export {export_id} holds no such file")))?;
    if offset > size {
        return Err(failed(format!(
            "offset {offset} lies beyond its {size} bytes"
        )));
    }
    let source = path.clone();
    let read: ReadAt = Arc::new(move |at, length| snapshot.read_chunk(&source, at, length));
    let open = TunnelDataOpen::Artifact {
        project_id,
        transfer_id,
        file_index: Some(file_index),
        offset,
    };
    cancellable(transfer, async {
        let session = crate::device_models::session(&device_id).await?;
        let mut stream = session
            .open_data(open)
            .await
            .map_err(|error| error.to_string())?;
        write_range(&mut stream, &read, offset, size, &|_| {}).await?;
        answer(&mut stream, VERIFY).await
    })
    .await
    .map_err(failed)
}

pub(super) async fn push(
    app: &AppHandle,
    transfer: &str,
    push: ModelAssetPush,
    progress: impl Fn(u64) + Send + Sync,
) -> Result<Value, String> {
    let ModelAssetPush {
        device_id,
        job_id,
        offset,
        file,
    } = push;
    let failed = |error: String| {
        format!("Sending model asset job {job_id} to device {device_id} failed: {error}")
    };
    let source = match file {
        Some(file) => Some(bit_store_source(app, file, offset).await.map_err(failed)?),
        None => None,
    };
    let open = TunnelDataOpen::ModelAsset {
        job_id: job_id.clone(),
        offset,
    };
    let verify = verify_deadline(source.as_ref().map_or(0, |(_, size)| *size));
    cancellable(transfer, async {
        let session = crate::device_models::session(&device_id).await?;
        let mut stream = session
            .open_data(open)
            .await
            .map_err(|error| error.to_string())?;
        if let Some((read, size)) = &source {
            write_range(&mut stream, read, offset, *size, &progress).await?;
        }
        answer(&mut stream, verify).await
    })
    .await
    .map_err(failed)
}

async fn bit_store_source(
    app: &AppHandle,
    file: BitStoreFile,
    offset: u64,
) -> Result<(ReadAt, u64), String> {
    let BitStoreFile {
        hash,
        file_name,
        size,
    } = file;
    let path = local_bit_file(app, &hash, &file_name)
        .await
        .map_err(|error| error.to_string())?;
    let held = tokio::fs::metadata(&path)
        .await
        .map_err(|error| format!("{file_name} is not in this computer's Bit store: {error}"))?
        .len();
    if held != size {
        return Err(format!(
            "{file_name} in this computer's Bit store holds {held} bytes, the model asset {size}"
        ));
    }
    if offset > size {
        return Err(format!(
            "offset {offset} lies beyond the {size} bytes of {file_name}"
        ));
    }
    let read: ReadAt = Arc::new(move |at, length| read_range(&path, at, length));
    Ok((read, size))
}

/// Writes bytes `from..to`, read at most 1 MiB at a time; `sent` gets the offset reached after
/// each chunk.
async fn write_range<S: AsyncWrite + Unpin>(
    stream: &mut S,
    read: &ReadAt,
    from: u64,
    to: u64,
    sent: &(dyn Fn(u64) + Sync),
) -> Result<(), String> {
    let mut offset = from;
    while offset < to {
        let length = (to - offset).min(MAX_DEVICE_EXPORT_CHUNK as u64) as usize;
        let mut bytes = read_chunk(read, offset, length).await?;
        let written = tokio::time::timeout(IDLE, stream.write_all(&bytes)).await;
        bytes.zeroize();
        written
            .map_err(|_| {
                format!(
                    "the device took no data for {} s at byte {offset}",
                    IDLE.as_secs()
                )
            })?
            .map_err(|error| format!("sending at byte {offset} failed: {error}"))?;
        offset += length as u64;
        sent(offset);
    }
    Ok(())
}

/// Exactly `length` bytes at `offset`, read off the runtime.
async fn read_chunk(read: &ReadAt, offset: u64, length: usize) -> Result<Vec<u8>, String> {
    let reader = read.clone();
    let mut bytes = tokio::task::spawn_blocking(move || reader(offset, length))
        .await
        .map_err(|error| format!("reading at byte {offset} stopped: {error}"))?
        .map_err(|error| format!("reading at byte {offset} failed: {error:#}"))?;
    if bytes.len() != length {
        let read_length = bytes.len();
        bytes.zeroize();
        return Err(format!(
            "reading at byte {offset} gave {read_length} of {length} bytes"
        ));
    }
    Ok(bytes)
}

/// How long the device may take to answer after the last byte of a `size`-byte model asset.
fn verify_deadline(size: u64) -> Duration {
    VERIFY.max(Duration::from_secs(size.div_ceil(VERIFY_BYTES_PER_SECOND)))
}

/// Ends the stream and reads the device's JSON answer, which may take `verify` to come.
async fn answer<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    verify: Duration,
) -> Result<Value, String> {
    tokio::time::timeout(IDLE, stream.shutdown())
        .await
        .map_err(|_| format!("ending the stream took over {} s", IDLE.as_secs()))?
        .map_err(|error| format!("ending the stream failed: {error}"))?;
    let mut bytes = Vec::new();
    tokio::time::timeout(
        verify,
        (&mut *stream).take(MAX_ANSWER + 1).read_to_end(&mut bytes),
    )
    .await
    .map_err(|_| {
        format!(
            "the device did not answer within {} minutes",
            verify.as_secs() / 60
        )
    })?
    .map_err(|error| format!("reading the device's answer failed: {error}"))?;
    if bytes.len() as u64 > MAX_ANSWER {
        return Err(format!(
            "the device's answer exceeds {} KiB",
            MAX_ANSWER / 1024
        ));
    }
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("the device's answer is not JSON: {error}"))?;
    if !value.is_object() {
        return Err("the device's answer is not a JSON object".into());
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tokio::io::DuplexStream;

    fn file(size: usize) -> Vec<u8> {
        (0..size).map(|index| (index % 251) as u8).collect()
    }

    fn reader(bytes: Vec<u8>) -> ReadAt {
        Arc::new(move |at, length| {
            let at = at as usize;
            Ok(bytes[at..at + length].to_vec())
        })
    }

    /// Reads until the client ends its side, then answers `reply`.
    fn device(mut end: DuplexStream, reply: Vec<u8>) -> tokio::task::JoinHandle<Vec<u8>> {
        tokio::spawn(async move {
            let mut received = Vec::new();
            end.read_to_end(&mut received).await.unwrap();
            end.write_all(&reply).await.unwrap();
            end.shutdown().await.unwrap();
            received
        })
    }

    #[tokio::test]
    async fn a_range_streams_in_chunks_and_the_answer_follows_the_end() {
        let size = MAX_DEVICE_EXPORT_CHUNK * 2 + 17;
        let bytes = file(size);
        let (mut client, end) = tokio::io::duplex(64 * 1024);
        let device = device(end, br#"{"state":"receiving","complete":true}"#.to_vec());
        let reached = Mutex::new(Vec::new());
        let from = 5;

        write_range(
            &mut client,
            &reader(bytes.clone()),
            from,
            size as u64,
            &|offset| reached.lock().unwrap().push(offset),
        )
        .await
        .unwrap();
        let answer = answer(&mut client, VERIFY).await.unwrap();

        assert_eq!(answer, json!({"state": "receiving", "complete": true}));
        assert_eq!(device.await.unwrap(), bytes[from as usize..]);
        let chunk = MAX_DEVICE_EXPORT_CHUNK as u64;
        assert_eq!(
            reached.into_inner().unwrap(),
            [from + chunk, from + 2 * chunk, size as u64]
        );
    }

    #[tokio::test]
    async fn a_probe_sends_no_bytes_and_reads_the_answer() {
        let (mut client, end) = tokio::io::duplex(1024);
        let device = device(end, br#"{"state":"awaiting_push","bytes":42}"#.to_vec());
        assert_eq!(
            answer(&mut client, VERIFY).await.unwrap()["bytes"],
            json!(42),
            "the device says where its copy ends"
        );
        assert!(device.await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn answers_must_be_one_small_json_object() {
        for (reply, expected) in [
            (vec![b' '; MAX_ANSWER as usize + 1], "exceeds 64 KiB"),
            (b"[1, 2]".to_vec(), "not a JSON object"),
            (b"{\"state\":".to_vec(), "not JSON"),
        ] {
            let (mut client, end) = tokio::io::duplex(1024);
            let device = device(end, reply);
            let error = answer(&mut client, VERIFY).await.unwrap_err();
            assert!(error.contains(expected), "{error}");
            device.await.unwrap();
        }
    }

    #[test]
    fn a_large_model_asset_gets_as_long_to_verify_as_the_device_needs_to_read_it_back() {
        assert_eq!(verify_deadline(0), VERIFY);
        assert_eq!(verify_deadline(4 * 1024 * 1024 * 1024), VERIFY);
        assert_eq!(verify_deadline(40_000_000_000), Duration::from_secs(4_000));
        assert_eq!(
            verify_deadline(64 * 1024 * 1024 * 1024),
            Duration::from_secs(6_872)
        );
    }

    #[tokio::test]
    async fn the_answer_waits_as_long_as_the_verification_allows() {
        let (mut client, _device) = tokio::io::duplex(1024);
        let error = answer(&mut client, Duration::from_millis(20))
            .await
            .unwrap_err();
        assert!(error.contains("did not answer within"), "{error}");
    }

    #[tokio::test]
    async fn a_short_read_stops_before_sending() {
        let (mut client, _end) = tokio::io::duplex(1024);
        let short: ReadAt = Arc::new(|_, length| Ok(vec![0; length - 1]));
        let error = write_range(&mut client, &short, 0, 10, &|_| {})
            .await
            .unwrap_err();
        assert_eq!(error, "reading at byte 0 gave 9 of 10 bytes");
    }

    #[tokio::test]
    async fn a_transfer_ends_when_cancelled_also_before_it_started() {
        let early = uuid::Uuid::new_v4().to_string();
        cancel(&early).unwrap();
        let result = cancellable(&early, std::future::pending::<Result<(), String>>()).await;
        assert_eq!(result.unwrap_err(), "the transfer was cancelled");

        let running = uuid::Uuid::new_v4().to_string();
        let waiting = {
            let running = running.clone();
            tokio::spawn(async move {
                cancellable(&running, std::future::pending::<Result<(), String>>()).await
            })
        };
        tokio::time::sleep(Duration::from_millis(20)).await;
        cancel(&running).unwrap();
        assert!(waiting.await.unwrap().is_err());
        assert!(with_transfers(|transfers| {
            !transfers.contains_key(&early) && !transfers.contains_key(&running)
        }));
        assert!(cancel("not-a-transfer").is_err());
    }
}

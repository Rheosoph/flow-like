use flow_like_storage::Path;
use flow_like_storage::object_store::{
    Error as ObjectStoreError, GetOptions, ObjectMeta, ObjectStore, PutMode, PutOptions,
    PutPayload, PutResult, UpdateVersion,
};
use flow_like_types::Message;
use lz4_flex::{compress_prepend_size, decompress_size_prepended};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::instrument;

/// Outcome of a conditional read against a cached object identity.
pub enum ConditionalRead<T> {
    /// The object still carries the caller's `e_tag`; no body was transferred.
    NotModified,
    /// The object changed (or the caller had no `e_tag`); the fresh value and its metadata.
    Fresh(T, ObjectMeta),
}

/// Write a protobuf message as an lz4 block. Returns the store's [`PutResult`] so callers
/// that cache the written value can pin it to the object identity the store just assigned.
#[instrument(
    name = "compress_to_file",
    skip(store, file_path, input),
    level = "debug"
)]
pub async fn compress_to_file<T>(
    store: Arc<dyn ObjectStore>,
    file_path: Path,
    input: &T,
) -> flow_like_types::Result<PutResult>
where
    T: Message,
{
    let mut data = Vec::new();
    input.encode(&mut data)?;
    let compressed = compress_prepend_size(&data);
    let result = store.put(&file_path, PutPayload::from(compressed)).await?;
    Ok(result)
}

/// Compress and write a protobuf only when the destination does not already
/// exist. Versioned artifacts use this to preserve immutability even when two
/// publishers race.
pub async fn compress_to_file_create<T>(
    store: Arc<dyn ObjectStore>,
    file_path: Path,
    input: &T,
) -> flow_like_types::Result<()>
where
    T: Message,
{
    let mut data = Vec::new();
    input.encode(&mut data)?;
    let compressed = compress_prepend_size(&data);
    store
        .put_opts(
            &file_path,
            PutPayload::from(compressed),
            PutOptions {
                mode: PutMode::Create,
                ..Default::default()
            },
        )
        .await?;
    Ok(())
}

/// Compress and replace a protobuf only if the destination still has the
/// version observed by the caller. This prevents a delayed two-phase commit
/// from overwriting a concurrently saved floating board.
pub async fn compress_to_file_update<T>(
    store: Arc<dyn ObjectStore>,
    file_path: Path,
    input: &T,
    expected: UpdateVersion,
) -> flow_like_types::Result<()>
where
    T: Message,
{
    let mut data = Vec::new();
    input.encode(&mut data)?;
    let compressed = compress_prepend_size(&data);
    store
        .put_opts(
            &file_path,
            PutPayload::from(compressed),
            PutOptions {
                mode: PutMode::Update(expected),
                ..Default::default()
            },
        )
        .await?;
    Ok(())
}

pub async fn compress_to_file_json<T>(
    store: Arc<dyn ObjectStore>,
    file_path: Path,
    input: &T,
) -> flow_like_types::Result<()>
where
    T: Serialize + Deserialize<'static>,
{
    let data = flow_like_types::json::to_vec(input)?;
    let compressed = compress_prepend_size(&data);
    let _result = store.put(&file_path, PutPayload::from(compressed)).await?;
    Ok(())
}

/// Read from a compressed file and deserialize it into a Serde Deserializable Struct
#[instrument(name = "from_compressed", skip(store, file_path), level = "debug")]
pub async fn from_compressed<T>(
    store: Arc<dyn ObjectStore>,
    file_path: Path,
) -> flow_like_types::Result<T>
where
    T: Message + Default,
{
    Ok(from_compressed_with_meta(store, file_path).await?.0)
}

/// Same as [`from_compressed`] but also returns the [`ObjectMeta`] from the
/// underlying GET response — callers caching the deserialized result use the
/// `e_tag` / `last_modified` to validate freshness with a cheap HEAD.
#[instrument(
    name = "from_compressed_with_meta",
    skip(store, file_path),
    level = "debug"
)]
pub async fn from_compressed_with_meta<T>(
    store: Arc<dyn ObjectStore>,
    file_path: Path,
) -> flow_like_types::Result<(T, ObjectMeta)>
where
    T: Message + Default,
{
    let reader = store.get(&file_path).await?;
    let meta = reader.meta.clone();
    let bytes = reader.bytes().await?;

    let data = decompress_size_prepended(&bytes)?;
    let message = T::decode(&data[..])?;

    Ok((message, meta))
}

/// Conditional variant of [`from_compressed_with_meta`].
///
/// With `Some(e_tag)` this issues a single `If-None-Match` GET: an unchanged object costs
/// one round trip and no body, a changed object arrives in the same round trip. This is what
/// makes an ETag-validated cache free rather than a HEAD-then-GET tax — the miss path costs
/// exactly what an unconditional GET does. Passing `None` behaves like the unconditional read.
#[instrument(
    name = "from_compressed_if_changed",
    skip(store, file_path, e_tag),
    level = "debug"
)]
pub async fn from_compressed_if_changed<T>(
    store: Arc<dyn ObjectStore>,
    file_path: Path,
    e_tag: Option<&str>,
) -> flow_like_types::Result<ConditionalRead<T>>
where
    T: Message + Default,
{
    let options = GetOptions {
        if_none_match: e_tag.map(str::to_string),
        ..Default::default()
    };
    let reader = match store.get_opts(&file_path, options).await {
        Ok(reader) => reader,
        Err(ObjectStoreError::NotModified { .. }) => return Ok(ConditionalRead::NotModified),
        Err(err) => return Err(err.into()),
    };
    let meta = reader.meta.clone();
    let bytes = reader.bytes().await?;

    let data = decompress_size_prepended(&bytes)?;
    let message = T::decode(&data[..])?;

    Ok(ConditionalRead::Fresh(message, meta))
}

pub async fn from_compressed_json<T>(
    store: Arc<dyn ObjectStore>,
    file_path: Path,
) -> flow_like_types::Result<T>
where
    T: Serialize + DeserializeOwned,
{
    let reader = store.get(&file_path).await?;
    let bytes = reader.bytes().await?;
    let data = decompress_size_prepended(&bytes)?;

    let data: T = flow_like_types::json::from_slice(&data)?;
    Ok(data)
}

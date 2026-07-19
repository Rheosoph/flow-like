use flow_like_storage::Path;
use flow_like_storage::object_store::{
    ObjectMeta, ObjectStore, PutMode, PutOptions, PutPayload, UpdateVersion,
};
use flow_like_types::Message;
use lz4_flex::{compress_prepend_size, decompress_size_prepended};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::instrument;

///Write a Serde Serializable Struct to compressed file using bitcode + lz4
#[instrument(
    name = "compress_to_file",
    skip(store, file_path, input),
    level = "debug"
)]
pub async fn compress_to_file<T>(
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
    let _result = store.put(&file_path, PutPayload::from(compressed)).await?;
    Ok(())
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

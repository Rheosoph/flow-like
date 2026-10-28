//! Read-only [`ObjectStore`] decorator for shadow/replay runs.
//!
//! Every mutating operation fails loudly with a `NotSupported` error instead of
//! silently succeeding. A fabricated success would hide changes in a shadow-run diff.
//! All read operations delegate to the wrapped store unchanged.

use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::BoxStream;
use object_store::path::Path;
use object_store::{
    CopyOptions, GetOptions, GetResult, ListResult, MultipartUpload, ObjectMeta, ObjectStore,
    PutMultipartOptions, PutOptions, PutPayload, PutResult, RenameOptions, Result,
};
use std::ops::Range;
use std::sync::Arc;

/// Carries the code a regression suite reads to grade a replay that failed
/// only on refused side effects as skipped instead of regressed.
const WRITE_REFUSED: &str =
    "SHADOW_SIDE_EFFECT_REFUSED: Shadow and regression runs cannot write app storage";

#[derive(Debug)]
pub struct ReadOnlyStore {
    inner: Arc<dyn ObjectStore>,
}

impl ReadOnlyStore {
    pub fn new(inner: Arc<dyn ObjectStore>) -> Self {
        Self { inner }
    }

    fn write_denied(operation: &str, location: &str) -> object_store::Error {
        object_store::Error::NotSupported {
            source: format!(
                "{WRITE_REFUSED}: {operation} on '{location}' was blocked by the read-only store"
            )
            .into(),
        }
    }
}

impl std::fmt::Display for ReadOnlyStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ReadOnlyStore({})", self.inner)
    }
}

#[async_trait]
impl ObjectStore for ReadOnlyStore {
    async fn put_opts(
        &self,
        location: &Path,
        _payload: PutPayload,
        _opts: PutOptions,
    ) -> Result<PutResult> {
        Err(Self::write_denied("put", location.as_ref()))
    }

    async fn put_multipart_opts(
        &self,
        location: &Path,
        _opts: PutMultipartOptions,
    ) -> Result<Box<dyn MultipartUpload>> {
        Err(Self::write_denied("multipart upload", location.as_ref()))
    }

    async fn get_opts(&self, location: &Path, opts: GetOptions) -> Result<GetResult> {
        self.inner.get_opts(location, opts).await
    }

    async fn get_ranges(&self, location: &Path, ranges: &[Range<u64>]) -> Result<Vec<Bytes>> {
        self.inner.get_ranges(location, ranges).await
    }

    fn delete_stream(
        &self,
        locations: BoxStream<'static, Result<Path>>,
    ) -> BoxStream<'static, Result<Path>> {
        Box::pin(futures::StreamExt::map(locations, |location| {
            let location = location?;
            Err(Self::write_denied("delete", location.as_ref()))
        }))
    }

    fn list(&self, prefix: Option<&Path>) -> BoxStream<'static, Result<ObjectMeta>> {
        self.inner.list(prefix)
    }

    fn list_with_offset(
        &self,
        prefix: Option<&Path>,
        offset: &Path,
    ) -> BoxStream<'static, Result<ObjectMeta>> {
        self.inner.list_with_offset(prefix, offset)
    }

    async fn list_with_delimiter(&self, prefix: Option<&Path>) -> Result<ListResult> {
        self.inner.list_with_delimiter(prefix).await
    }

    async fn copy_opts(&self, from: &Path, to: &Path, _options: CopyOptions) -> Result<()> {
        Err(Self::write_denied(
            "copy",
            &format!("{from} -> {to}", from = from.as_ref(), to = to.as_ref()),
        ))
    }

    async fn rename_opts(&self, from: &Path, to: &Path, _options: RenameOptions) -> Result<()> {
        Err(Self::write_denied(
            "rename",
            &format!("{from} -> {to}", from = from.as_ref(), to = to.as_ref()),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::TryStreamExt;
    use object_store::{ObjectStoreExt, memory::InMemory};

    fn store_with_object() -> (Arc<InMemory>, ReadOnlyStore) {
        let inner = Arc::new(InMemory::new());
        let read_only = ReadOnlyStore::new(inner.clone());
        (inner, read_only)
    }

    #[tokio::test]
    async fn every_write_operation_fails_loudly() {
        let (inner, store) = store_with_object();
        let path = Path::from("apps/app-1/file.txt");
        inner
            .put(&path, PutPayload::from_static(b"live"))
            .await
            .expect("seed inner store");

        let (copy, moved) = (Path::from("copy.txt"), Path::from("moved.txt"));
        let streamed = store
            .delete_stream(Box::pin(futures::stream::iter([Ok(path.clone())])))
            .try_collect::<Vec<_>>()
            .await
            .map(drop);
        let copied = "apps/app-1/file.txt -> copy.txt";
        let renamed = "apps/app-1/file.txt -> moved.txt";
        for (operation, location, result) in [
            (
                "put",
                "apps/app-1/file.txt",
                store
                    .put(&path, PutPayload::from_static(b"x"))
                    .await
                    .map(drop),
            ),
            (
                "multipart upload",
                "apps/app-1/file.txt",
                store.put_multipart(&path).await.map(drop),
            ),
            ("delete", "apps/app-1/file.txt", store.delete(&path).await),
            ("delete", "apps/app-1/file.txt", streamed),
            ("copy", copied, store.copy(&path, &copy).await),
            ("copy", copied, store.copy_if_not_exists(&path, &copy).await),
            ("rename", renamed, store.rename(&path, &moved).await),
            (
                "rename",
                renamed,
                store.rename_if_not_exists(&path, &moved).await,
            ),
        ] {
            assert_eq!(
                result.unwrap_err().to_string(),
                format!(
                    "Operation not supported: {WRITE_REFUSED}: {operation} on '{location}' was blocked by the read-only store"
                )
            );
        }

        // The inner object survived every attempt untouched.
        let bytes = inner.get(&path).await.unwrap().bytes().await.unwrap();
        assert_eq!(bytes.as_ref(), b"live");
    }

    #[tokio::test]
    async fn reads_delegate_to_the_inner_store() {
        let (inner, store) = store_with_object();
        let path = Path::from("apps/app-1/file.txt");
        inner
            .put(&path, PutPayload::from_static(b"live"))
            .await
            .expect("seed inner store");

        let bytes = store.get(&path).await.unwrap().bytes().await.unwrap();
        assert_eq!(bytes.as_ref(), b"live");
        assert_eq!(store.head(&path).await.unwrap().size, 4);
        let listed: Vec<_> = store.list(None).try_collect().await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].location, path);
    }
}

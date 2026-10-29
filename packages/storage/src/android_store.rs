use async_trait::async_trait;
use futures::stream::BoxStream;
use lance_io::object_store::WrappingObjectStore;
use lance_object_store::{
    CopyMode, CopyOptions, GetOptions, GetResult, ListResult, MultipartUpload, ObjectMeta,
    ObjectStore, ObjectStoreExt, PutMode, PutMultipartOptions, PutOptions, PutPayload, PutResult,
    RenameOptions, RenameTargetMode, path::Path,
};
use std::sync::Arc;

/// Wraps an `ObjectStore` to avoid `hard_link()` calls that fail on Android.
///
/// Android's SELinux policy blocks the `link()` syscall, which
/// `object_store::LocalFileSystem` uses for `PutMode::Create` and
/// `copy_if_not_exists`. This wrapper intercepts those operations and
/// uses existence checks + overwrite/rename instead.
#[derive(Debug)]
pub struct AndroidSafeObjectStore {
    inner: Arc<dyn ObjectStore>,
}

impl AndroidSafeObjectStore {
    pub fn new(inner: Arc<dyn ObjectStore>) -> Self {
        Self { inner }
    }
}

impl std::fmt::Display for AndroidSafeObjectStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "AndroidSafe({})", self.inner)
    }
}

#[async_trait]
impl ObjectStore for AndroidSafeObjectStore {
    async fn put_opts(
        &self,
        location: &Path,
        payload: PutPayload,
        opts: PutOptions,
    ) -> lance_object_store::Result<PutResult> {
        eprintln!(
            "[AndroidSafe] put_opts called, mode={:?}, location={}",
            opts.mode, location
        );
        match opts.mode {
            PutMode::Create => match self.inner.head(location).await {
                Ok(_) => {
                    eprintln!("[AndroidSafe] file exists, returning AlreadyExists");
                    Err(lance_object_store::Error::AlreadyExists {
                        path: location.to_string(),
                        source: "File already exists (AndroidSafe check)".into(),
                    })
                }
                Err(lance_object_store::Error::NotFound { .. }) => {
                    eprintln!("[AndroidSafe] file not found, using Overwrite mode");
                    self.inner
                        .put_opts(
                            location,
                            payload,
                            PutOptions {
                                mode: PutMode::Overwrite,
                                ..opts
                            },
                        )
                        .await
                }
                Err(e) => {
                    eprintln!("[AndroidSafe] head error: {:?}", e);
                    Err(e)
                }
            },
            _ => self.inner.put_opts(location, payload, opts).await,
        }
    }

    async fn put_multipart_opts(
        &self,
        location: &Path,
        opts: PutMultipartOptions,
    ) -> lance_object_store::Result<Box<dyn MultipartUpload>> {
        self.inner.put_multipart_opts(location, opts).await
    }

    async fn get_opts(
        &self,
        location: &Path,
        options: GetOptions,
    ) -> lance_object_store::Result<GetResult> {
        self.inner.get_opts(location, options).await
    }

    fn delete_stream(
        &self,
        locations: BoxStream<'static, lance_object_store::Result<Path>>,
    ) -> BoxStream<'static, lance_object_store::Result<Path>> {
        self.inner.delete_stream(locations)
    }

    fn list(
        &self,
        prefix: Option<&Path>,
    ) -> BoxStream<'static, lance_object_store::Result<ObjectMeta>> {
        self.inner.list(prefix)
    }

    async fn list_with_delimiter(
        &self,
        prefix: Option<&Path>,
    ) -> lance_object_store::Result<ListResult> {
        self.inner.list_with_delimiter(prefix).await
    }

    async fn copy_opts(
        &self,
        from: &Path,
        to: &Path,
        mut options: CopyOptions,
    ) -> lance_object_store::Result<()> {
        if options.mode == CopyMode::Overwrite {
            return self.inner.copy_opts(from, to, options).await;
        }
        match self.inner.head(to).await {
            Ok(_) => Err(lance_object_store::Error::AlreadyExists {
                path: to.to_string(),
                source: "File already exists (AndroidSafe check)".into(),
            }),
            Err(lance_object_store::Error::NotFound { .. }) => {
                options.mode = CopyMode::Overwrite;
                self.inner.copy_opts(from, to, options).await
            }
            Err(e) => Err(e),
        }
    }

    async fn rename_opts(
        &self,
        from: &Path,
        to: &Path,
        mut options: RenameOptions,
    ) -> lance_object_store::Result<()> {
        if options.target_mode == RenameTargetMode::Overwrite {
            return self.inner.rename_opts(from, to, options).await;
        }
        match self.inner.head(to).await {
            Ok(_) => Err(lance_object_store::Error::AlreadyExists {
                path: to.to_string(),
                source: "File already exists (AndroidSafe check)".into(),
            }),
            Err(lance_object_store::Error::NotFound { .. }) => {
                options.target_mode = RenameTargetMode::Overwrite;
                self.inner.rename_opts(from, to, options).await
            }
            Err(e) => Err(e),
        }
    }
}

/// A [`WrappingObjectStore`] that wraps any inner store with [`AndroidSafeObjectStore`].
#[derive(Debug)]
pub struct AndroidSafeWrapper;

impl WrappingObjectStore for AndroidSafeWrapper {
    fn wrap(&self, _store_prefix: &str, original: Arc<dyn ObjectStore>) -> Arc<dyn ObjectStore> {
        Arc::new(AndroidSafeObjectStore::new(original))
    }

    /// Only conditional writes need the workaround; listings stay pushed down.
    fn wrap_paginated(
        &self,
        _store_prefix: &str,
        original: Arc<dyn lance_object_store::list::PaginatedListStore>,
    ) -> Option<Arc<dyn lance_object_store::list::PaginatedListStore>> {
        Some(original)
    }
}

/// Build [`lancedb::table::WriteOptions`] configured for Android-safe LanceDB writes.
pub fn android_write_options() -> lancedb::table::WriteOptions {
    use lance_io::object_store::ObjectStoreParams;

    let mut options = crate::lancedb_write_options::default_write_options();
    let write_params = options
        .lance_write_params
        .get_or_insert_with(Default::default);
    write_params.store_params = Some(ObjectStoreParams {
        object_store_wrapper: Some(Arc::new(AndroidSafeWrapper)),
        ..Default::default()
    });
    options
}

use flow_like_storage::object_store::ObjectMeta;
use serde::Serialize;
use std::time::Duration;

pub(super) const ARTIFACT_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// A storage read must validate existence before this key is used. Timestamps
/// alone cannot identify a rewrite; stores without an ETag or version bypass
/// the cache. Include the path and caller's scope to isolate apps and artifacts.
pub(super) fn revision_key(scope: impl Serialize, meta: &ObjectMeta) -> Option<String> {
    if !meta.e_tag.as_ref().is_some_and(|tag| !tag.is_empty())
        && !meta
            .version
            .as_ref()
            .is_some_and(|version| !version.is_empty())
    {
        return None;
    }
    let identity = (
        scope,
        meta.location.as_ref(),
        &meta.e_tag,
        &meta.version,
        meta.last_modified,
        meta.size,
    );
    serde_json::to_vec(&identity)
        .ok()
        .map(|bytes| blake3::hash(&bytes).to_hex().to_string())
}

#[cfg(test)]
pub(super) mod test_support {
    use flow_like_storage::{
        Path,
        object_store::{self, ObjectStore, ObjectStoreExt, *},
    };
    use futures::stream::BoxStream;
    use std::sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    #[derive(Debug, Default)]
    pub struct TestStore {
        pub inner: memory::InMemory,
        pub reads: AtomicUsize,
        pub after_head: Mutex<Option<(Path, PutPayload)>>,
    }

    impl std::fmt::Display for TestStore {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "artifact-test-store")
        }
    }

    #[async_trait::async_trait]
    impl ObjectStore for TestStore {
        async fn put_opts(
            &self,
            path: &Path,
            payload: PutPayload,
            opts: PutOptions,
        ) -> object_store::Result<PutResult> {
            self.inner.put_opts(path, payload, opts).await
        }

        async fn put_multipart_opts(
            &self,
            path: &Path,
            opts: PutMultipartOptions,
        ) -> object_store::Result<Box<dyn MultipartUpload>> {
            self.inner.put_multipart_opts(path, opts).await
        }

        async fn get_opts(&self, path: &Path, opts: GetOptions) -> object_store::Result<GetResult> {
            let head = opts.head;
            if !head {
                self.reads.fetch_add(1, Ordering::SeqCst);
            }
            let result = self.inner.get_opts(path, opts).await?;
            if head {
                let replacement = self.after_head.lock().unwrap().take();
                if let Some((path, payload)) = replacement {
                    self.inner.put(&path, payload).await?;
                }
            }
            Ok(result)
        }

        fn delete_stream(
            &self,
            paths: BoxStream<'static, object_store::Result<Path>>,
        ) -> BoxStream<'static, object_store::Result<Path>> {
            self.inner.delete_stream(paths)
        }

        fn list(
            &self,
            prefix: Option<&Path>,
        ) -> BoxStream<'static, object_store::Result<ObjectMeta>> {
            self.inner.list(prefix)
        }

        async fn list_with_delimiter(
            &self,
            prefix: Option<&Path>,
        ) -> object_store::Result<ListResult> {
            self.inner.list_with_delimiter(prefix).await
        }

        async fn copy_opts(
            &self,
            from: &Path,
            to: &Path,
            opts: CopyOptions,
        ) -> object_store::Result<()> {
            self.inner.copy_opts(from, to, opts).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_storage::Path;

    #[test]
    fn revision_keys_require_storage_identity_and_isolate_artifacts() {
        let mut meta = ObjectMeta {
            location: Path::from("apps/app/_board/page.page"),
            last_modified: chrono::Utc::now(),
            size: 42,
            e_tag: None,
            version: None,
        };
        assert!(revision_key(("app", "board"), &meta).is_none());
        meta.e_tag = Some("first".into());
        let first = revision_key(("app", "board"), &meta).unwrap();
        assert_ne!(first, revision_key(("other-app", "board"), &meta).unwrap());
        assert_ne!(first, revision_key(("app", "other-board"), &meta).unwrap());
        meta.e_tag = Some("rewritten".into());
        assert_ne!(first, revision_key(("app", "board"), &meta).unwrap());
        meta.e_tag = Some("first".into());
        meta.version = Some("recreated".into());
        assert_ne!(first, revision_key(("app", "board"), &meta).unwrap());
        meta.e_tag = None;
        assert!(revision_key(("app", "board"), &meta).is_some());
    }
}

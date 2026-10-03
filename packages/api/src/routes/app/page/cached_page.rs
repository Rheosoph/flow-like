use crate::{
    cache::{CacheBackendHandle, best_effort},
    routes::app::artifact_cache::{ARTIFACT_TTL, revision_key},
};
use flow_like::{
    a2ui::widget::Page, flow::board::Board, utils::compression::from_compressed_with_meta,
};
use flow_like_storage::object_store::{ObjectStore, ObjectStoreExt};
use flow_like_types::{Result, proto};
use std::sync::Arc;

const NAMESPACE: &str = "page-artifacts-v1";

/// The route has already authorized the caller and loaded the owning board.
/// Always validate the page object before using a shared entry, including for
/// published versions, so deletion and same-version restores remain visible.
pub(super) async fn load(
    cache: &CacheBackendHandle,
    store: Arc<dyn ObjectStore>,
    storage_scope: Option<&str>,
    app_id: &str,
    board: &Board,
    page_id: &str,
    version: Option<(u32, u32, u32)>,
) -> Result<Page> {
    let Some(storage_scope) = storage_scope else {
        return uncached(store, board, page_id, version).await;
    };
    let path = match version {
        Some(version) => board.versioned_page_path(version, page_id),
        None => board.page_path(page_id),
    };
    let scope = (storage_scope, app_id, &board.id, page_id, version);
    // The old loader owns legacy migration and error handling. A failed HEAD
    // must not turn an otherwise readable canonical or legacy page into a miss.
    if let Ok(meta) = store.head(&path).await {
        if let Some(key) = revision_key(scope, &meta)
            && let Some(page) = best_effort::get(cache, NAMESPACE, &key).await
        {
            return Ok(page);
        }

        if let Ok((proto, meta)) =
            from_compressed_with_meta::<proto::Page>(store.clone(), path).await
        {
            let page = Page::from(proto);
            // Use the GET identity: a writer may have changed the object since
            // HEAD, and that newer payload must never be cached under the old key.
            if let Some(key) = revision_key(scope, &meta) {
                best_effort::set(cache, NAMESPACE, &key, &page, ARTIFACT_TTL).await;
            }
            return Ok(page);
        }
    }

    uncached(store, board, page_id, version).await
}

async fn uncached(
    store: Arc<dyn ObjectStore>,
    board: &Board,
    page_id: &str,
    version: Option<(u32, u32, u32)>,
) -> Result<Page> {
    match version {
        Some(version) => {
            board
                .load_versioned_page(page_id, version, Some(store))
                .await
        }
        None => board.load_page(page_id, Some(store)).await,
    }
}

pub(super) async fn find_by_route(
    store: Arc<dyn ObjectStore>,
    board: &Board,
    route: &str,
) -> Option<Page> {
    for page_id in board.get_page_ids() {
        // A route scan may touch many pages. Keep it on direct reads so a cache
        // outage cannot add its timeout once or twice for every page inspected.
        match uncached(store.clone(), board, page_id, None).await {
            Ok(page) if page.route == route => return Some(page),
            Ok(_) => {}
            Err(error) => tracing::warn!(
                board_id = %board.id,
                page_id,
                error = %error,
                "Board lists a page whose payload is unreadable"
            ),
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::app::artifact_cache::test_support::TestStore;
    use flow_like::utils::compression::{compress_to_file, compress_to_file_json};
    use flow_like_storage::{Path, object_store::PutPayload};
    use std::sync::atomic::Ordering;

    fn board() -> Board {
        Board::new_detached(Some("board".into()), Path::from("apps/app"))
    }

    async fn write_page(store: Arc<TestStore>, path: Path, page: &Page) {
        let proto: proto::Page = page.clone().into();
        compress_to_file(store, path, &proto).await.unwrap();
    }

    #[flow_like_types::tokio::test]
    async fn separate_handles_reuse_pages_and_rewrites_or_deletion_invalidate() {
        let first = CacheBackendHandle::memory_for_test();
        let second = CacheBackendHandle::from_store_for_test(first.store().await.unwrap());
        let store = Arc::new(TestStore::default());
        let board = board();
        let path = board.page_path("page");
        let mut page = Page::new("page", "first", "/");
        write_page(store.clone(), path.clone(), &page).await;

        for cache in [&first, &second] {
            assert_eq!(
                load(
                    cache,
                    store.clone(),
                    Some("bucket"),
                    "app",
                    &board,
                    "page",
                    None
                )
                .await
                .unwrap()
                .name,
                "first"
            );
        }
        assert_eq!(
            store.reads.load(Ordering::SeqCst),
            1,
            "second replica should only HEAD"
        );

        // Storage writers can leave both the DB and payload timestamp unchanged.
        page.name = "rewritten".into();
        write_page(store.clone(), path.clone(), &page).await;
        assert_eq!(
            load(
                &second,
                store.clone(),
                Some("bucket"),
                "app",
                &board,
                "page",
                None
            )
            .await
            .unwrap()
            .name,
            "rewritten"
        );
        assert_eq!(store.reads.load(Ordering::SeqCst), 2);

        // A corrupt cache value is a miss, never an unreadable page.
        let meta = store.head(&path).await.unwrap();
        let key = revision_key(
            ("bucket", "app", &board.id, "page", None::<(u32, u32, u32)>),
            &meta,
        )
        .unwrap();
        first
            .platform()
            .await
            .unwrap()
            .set(NAMESPACE, &key, &"wrong type", ARTIFACT_TTL)
            .await
            .unwrap();
        assert_eq!(
            load(
                &second,
                store.clone(),
                Some("bucket"),
                "app",
                &board,
                "page",
                None
            )
            .await
            .unwrap()
            .name,
            "rewritten"
        );
        assert_eq!(store.reads.load(Ordering::SeqCst), 3);

        store.delete(&path).await.unwrap();
        assert!(
            load(
                &second,
                store.clone(),
                Some("bucket"),
                "app",
                &board,
                "page",
                None
            )
            .await
            .is_err()
        );
        page.name = "recreated".into();
        write_page(store.clone(), path, &page).await;
        assert_eq!(
            load(
                &second,
                store.clone(),
                Some("bucket"),
                "app",
                &board,
                "page",
                None
            )
            .await
            .unwrap()
            .name,
            "recreated"
        );
    }

    #[flow_like_types::tokio::test]
    async fn racing_rewrite_is_cached_under_get_identity() {
        let cache = CacheBackendHandle::memory_for_test();
        let store = Arc::new(TestStore::default());
        let board = board();
        let path = board.page_path("page");
        write_page(
            store.clone(),
            path.clone(),
            &Page::new("page", "before", "/"),
        )
        .await;
        let old_meta = store.head(&path).await.unwrap();
        let replacement = Path::from("replacement");
        write_page(
            store.clone(),
            replacement.clone(),
            &Page::new("page", "after", "/"),
        )
        .await;
        let bytes = store
            .inner
            .get(&replacement)
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        *store.after_head.lock().unwrap() = Some((path.clone(), PutPayload::from(bytes)));

        assert_eq!(
            load(
                &cache,
                store.clone(),
                Some("bucket"),
                "app",
                &board,
                "page",
                None
            )
            .await
            .unwrap()
            .name,
            "after"
        );
        let key = revision_key(
            ("bucket", "app", &board.id, "page", None::<(u32, u32, u32)>),
            &old_meta,
        )
        .unwrap();
        assert!(
            cache
                .platform()
                .await
                .unwrap()
                .get::<Page>(NAMESPACE, &key)
                .await
                .unwrap()
                .is_none()
        );
        let before = store.reads.load(Ordering::SeqCst);
        assert_eq!(
            load(
                &cache,
                store.clone(),
                Some("bucket"),
                "app",
                &board,
                "page",
                None
            )
            .await
            .unwrap()
            .name,
            "after"
        );
        assert_eq!(store.reads.load(Ordering::SeqCst), before);
    }

    #[flow_like_types::tokio::test]
    async fn versioned_pages_stay_bound_and_legacy_pages_still_migrate() {
        let cache = CacheBackendHandle::memory_for_test();
        let store = Arc::new(TestStore::default());
        let board = board();
        let draft = Page::new("page", "draft", "/");
        let published = Page::new("page", "published", "/");
        write_page(store.clone(), board.page_path("page"), &draft).await;
        write_page(
            store.clone(),
            board.versioned_page_path((1, 0, 0), "page"),
            &published,
        )
        .await;
        assert_eq!(
            load(
                &cache,
                store.clone(),
                Some("bucket"),
                "app",
                &board,
                "page",
                Some((1, 0, 0))
            )
            .await
            .unwrap()
            .name,
            "published"
        );
        assert!(
            load(
                &cache,
                store.clone(),
                Some("bucket"),
                "app",
                &board,
                "page",
                Some((2, 0, 0))
            )
            .await
            .is_err()
        );

        let legacy = Page::new("legacy", "legacy", "/old");
        let legacy_path = Path::from("apps/app/legacy.page");
        compress_to_file_json(store.clone(), legacy_path.clone(), &legacy)
            .await
            .unwrap();
        assert_eq!(
            load(
                &cache,
                store.clone(),
                Some("bucket"),
                "app",
                &board,
                "legacy",
                None
            )
            .await
            .unwrap()
            .name,
            "legacy"
        );
        assert!(store.head(&board.page_path("legacy")).await.is_ok());
        assert!(store.head(&legacy_path).await.is_err());
    }

    #[flow_like_types::tokio::test]
    async fn route_lookup_stops_at_first_match_and_observes_new_earlier_matches() {
        let store = Arc::new(TestStore::default());
        let mut board = board();
        board.page_ids = vec!["first".into(), "second".into(), "third".into()];
        for (id, route) in [
            ("first", "/other"),
            ("second", "/target"),
            ("third", "/target"),
        ] {
            write_page(
                store.clone(),
                board.page_path(id),
                &Page::new(id, id, route),
            )
            .await;
        }
        let lookup = || find_by_route(store.clone(), &board, "/target");
        assert_eq!(lookup().await.unwrap().id, "second");
        assert_eq!(
            store.reads.load(Ordering::SeqCst),
            2,
            "third page should never be read"
        );
        assert_eq!(lookup().await.unwrap().id, "second");
        assert_eq!(store.reads.load(Ordering::SeqCst), 4);
        write_page(
            store.clone(),
            board.page_path("first"),
            &Page::new("first", "first", "/target"),
        )
        .await;
        assert_eq!(
            lookup().await.unwrap().id,
            "first",
            "a later duplicate cannot hide an earlier route rewrite"
        );
    }
}

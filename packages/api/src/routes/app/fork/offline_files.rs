use crate::{
    error::ApiError,
    middleware::jwt::AppUser,
    permission::fork_permission::{ForkTargetKind, check_can_fork},
    state::AppState,
    utils::fork::{
        ForkPolicy,
        policy::offline_skips_content,
        transfer::{
            MAX_TRANSFER_BATCH, SignedFile, TransferSigner, is_uploadable_content_path,
            rekeyed_content_path, relative_key,
        },
    },
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like_storage::{Path as StorePath, join_object_path, object_store::ObjectStore};
use futures::{TryStreamExt, future::try_join_all};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::{IntoParams, ToSchema};

/// Objects one page may look at, skipped ones included, so a page over a
/// mostly excluded prefix still answers quickly.
const MAX_SCANNED_PER_PAGE: usize = MAX_TRANSFER_BATCH * 8;
const MAX_SEED_LEN: usize = 64;

#[derive(Clone, Debug, Deserialize, IntoParams)]
pub struct OfflineFilesQuery {
    /// The fork's `new_app_id` from `/fork/offline/begin`: the seed its
    /// widget, template and page ids were derived from.
    pub seed: String,
    /// `next` of the previous page.
    #[serde(default)]
    pub after: Option<String>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct OfflineFilesPage {
    /// One signed `GET` per file; `path` is where it lands in the fork.
    pub files: Vec<SignedFile>,
    /// Files on this page the owner's fork policy keeps out of the fork.
    pub skipped: u64,
    /// Pass as `after` for the next page; absent on the last one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

/// One page of the source app's content files for an offline fork, as
/// signed downloads. The owner's fork policy is applied here, and each file
/// already carries the path it takes in the fork.
#[utoipa::path(
    get,
    path = "/apps/{app_id}/fork/offline/files",
    tag = "forking",
    description = "List the files of an app you are copying to this device, with a download link for each.",
    params(
        ("app_id" = String, Path, description = "Source application ID"),
        OfflineFilesQuery,
    ),
    responses(
        (status = 200, description = "One page of files with signed download links", body = OfflineFilesPage),
        (status = 400, description = "The seed or cursor is malformed"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Source app not found")
    )
)]
#[tracing::instrument(
    name = "GET /apps/{app_id}/fork/offline/files",
    skip(state, user, query)
)]
pub async fn offline_fork_files(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Query(query): Query<OfflineFilesQuery>,
) -> Result<Json<OfflineFilesPage>, ApiError> {
    let src_app = check_can_fork(&user, &app_id, &state, ForkTargetKind::Offline).await?;
    validate_query(&query)?;
    let policy = ForkPolicy::from_app_row(&src_app);
    let signer = TransferSigner::content(&state).await?;
    let prefix = StorePath::from("apps").join(app_id.as_str());
    let page = scan_page(&signer.store(), &prefix, query.after.as_deref(), &policy).await?;

    let files = try_join_all(page.picked.into_iter().map(|(relative, key, size)| {
        let signer = &signer;
        let path = rekeyed_content_path(&relative, &query.seed);
        async move { signer.download(path, &key, Some(size)).await }
    }))
    .await?;
    let expires_at = signer.expires_at();
    crate::audit::storage_access::authorize_downloads(
        &state,
        &user,
        &app_id,
        "fork_content",
        &query.seed,
        None,
        files.len(),
        expires_at,
    )
    .await?;
    Ok(Json(OfflineFilesPage {
        files,
        skipped: page.skipped,
        next: page.next,
        expires_at,
    }))
}

fn validate_query(query: &OfflineFilesQuery) -> Result<(), ApiError> {
    if !is_seed(&query.seed) {
        return Err(ApiError::bad_request(format!(
            "seed must be the fork's new_app_id (1-{MAX_SEED_LEN} letters, digits, '_' or '-')"
        )));
    }
    match query.after.as_deref() {
        Some(after) if after.split('/').any(|s| s.is_empty() || s == "..") => Err(
            ApiError::bad_request(format!("{after:?} is not a cursor this endpoint returned")),
        ),
        _ => Ok(()),
    }
}

/// One page of the source listing: the files that travel (relative path,
/// key, size), how many the policy kept back, and where the next page starts.
struct ScannedPage {
    picked: Vec<(String, StorePath, u64)>,
    skipped: u64,
    next: Option<String>,
}

async fn scan_page(
    store: &Arc<dyn ObjectStore>,
    prefix: &StorePath,
    after: Option<&str>,
    policy: &ForkPolicy,
) -> Result<ScannedPage, ApiError> {
    let mut listing = match after {
        Some(after) => store.list_with_offset(Some(prefix), &join_object_path(prefix, after)),
        None => store.list(Some(prefix)),
    };
    let mut page = ScannedPage {
        picked: Vec::with_capacity(MAX_TRANSFER_BATCH),
        skipped: 0,
        next: None,
    };
    let mut scanned = 0usize;
    let mut last = None;
    while let Some(object) = listing
        .try_next()
        .await
        .map_err(|e| ApiError::internal(format!("list content of {prefix}: {e}")))?
    {
        if page.picked.len() == MAX_TRANSFER_BATCH || scanned == MAX_SCANNED_PER_PAGE {
            page.next = last;
            return Ok(page);
        }
        scanned += 1;
        let Some(relative) = relative_key(&object.location, prefix) else {
            continue;
        };
        last = Some(relative.clone());
        match classify(policy, &relative) {
            Listed::Travels => page.picked.push((relative, object.location, object.size)),
            Listed::Excluded => page.skipped += 1,
            Listed::NotContent => {}
        }
    }
    Ok(page)
}

enum Listed {
    Travels,
    /// The owner's fork policy keeps it out.
    Excluded,
    /// Not app content at all, such as a meta-shaped file in the content
    /// store of a misconfigured deployment.
    NotContent,
}

fn classify(policy: &ForkPolicy, relative: &str) -> Listed {
    if !is_uploadable_content_path(relative) {
        Listed::NotContent
    } else if offline_skips_content(policy, relative) {
        Listed::Excluded
    } else {
        Listed::Travels
    }
}

fn is_seed(seed: &str) -> bool {
    !seed.is_empty()
        && seed.len() <= MAX_SEED_LEN
        && seed
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::fork::ForkDatabaseMode;
    use flow_like_storage::object_store::{ObjectStoreExt, memory::InMemory};

    #[tokio::test]
    async fn offline_listing_enforces_exclusions_and_app_boundaries() {
        let store: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
        for key in [
            "apps/app-1/metadata/en.meta",
            "apps/app-1/media/icon.png",
            "apps/app-1/upload/private.csv",
            "apps/app-1/storage/db/customers.lance/data/part.lance",
            "apps/app-1/secret.board",
            "apps/app-10/upload/other-app.csv",
            "users/other-user/apps/app-1/upload/private.csv",
        ] {
            store
                .put(
                    &StorePath::from(key),
                    bytes::Bytes::from_static(b"test").into(),
                )
                .await
                .expect("seed content store");
        }
        let policy = ForkPolicy {
            files: false,
            databases: ForkDatabaseMode::None,
            ..ForkPolicy::default()
        };
        let page = scan_page(&store, &StorePath::from("apps/app-1"), None, &policy)
            .await
            .expect("list permitted content");

        let paths: Vec<_> = page
            .picked
            .iter()
            .map(|(path, _, _)| path.as_str())
            .collect();
        assert_eq!(paths, ["media/icon.png", "metadata/en.meta"]);
        assert_eq!(page.skipped, 2);
        assert!(page.next.is_none());
    }

    #[tokio::test]
    async fn offline_listing_rechecks_policy_on_later_pages() {
        let store: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
        for index in 0..=MAX_TRANSFER_BATCH {
            let key = StorePath::from(format!("apps/app-1/upload/{index:04}.csv"));
            store
                .put(&key, bytes::Bytes::from_static(b"test").into())
                .await
                .expect("seed file");
        }
        let prefix = StorePath::from("apps/app-1");
        let first = scan_page(&store, &prefix, None, &ForkPolicy::default())
            .await
            .expect("first page");
        assert_eq!(first.picked.len(), MAX_TRANSFER_BATCH);
        let cursor = first.next.expect("remaining content has a cursor");

        let restricted = ForkPolicy {
            files: false,
            ..ForkPolicy::default()
        };
        let next = scan_page(&store, &prefix, Some(&cursor), &restricted)
            .await
            .expect("recheck the current policy");
        assert!(next.picked.is_empty());
        assert_eq!(next.skipped, 1);
        assert!(next.next.is_none());
    }

    #[test]
    fn offline_cursor_and_seed_cannot_escape_the_selected_app() {
        for seed in ["../other", "app/other", "app*", "app?", "", "app\\other"] {
            assert!(
                validate_query(&OfflineFilesQuery {
                    seed: seed.to_string(),
                    after: None,
                })
                .is_err(),
                "{seed}"
            );
        }
        for cursor in ["../other/upload/a", "/upload/a", "upload/../a", "upload//a"] {
            assert!(
                validate_query(&OfflineFilesQuery {
                    seed: "new-app".to_string(),
                    after: Some(cursor.to_string()),
                })
                .is_err(),
                "{cursor}"
            );
        }
        assert_eq!(
            relative_key(
                &StorePath::from("apps/app-10/upload/a"),
                &StorePath::from("apps/app-1")
            ),
            None
        );
    }
}

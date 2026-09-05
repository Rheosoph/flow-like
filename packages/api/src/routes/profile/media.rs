use std::{io::Cursor, time::Duration};

use crate::{error::ApiError, state::AppState};
use flow_like_storage::{
    Path,
    object_store::{Error as StoreError, ObjectStore, PutMode, PutOptions},
};
use sea_orm::{
    ActiveModelTrait, ActiveValue, ColumnTrait, EntityTrait, IdenStatic, Iterable, QueryFilter,
    QuerySelect,
};
use serde::{Deserialize, Serialize};

const MAX_IMAGE_BYTES: u64 = 20 * 1024 * 1024;
const UPLOAD_LIFETIME_SECONDS: i64 = 3600;

#[derive(Deserialize, Serialize)]
struct PendingUpload {
    scope: String,
    expires_at: i64,
    previous_image: Option<String>,
    extension: String,
    source_revision: Option<String>,
}

pub(crate) fn normalize_image_extension(extension: &str) -> Result<String, ApiError> {
    let extension = extension
        .trim()
        .trim_start_matches('.')
        .to_ascii_lowercase();
    match extension.as_str() {
        "webp" | "png" | "jpg" | "jpeg" | "gif" | "avif" => Ok(extension),
        _ => Err(ApiError::bad_request(
            "Use a WebP, PNG, JPEG, GIF or AVIF image",
        )),
    }
}

fn upload_stem(upload_id: &str) -> Result<&str, ApiError> {
    upload_id
        .strip_suffix(".webp")
        .filter(|stem| {
            !stem.is_empty()
                && stem.len() <= 128
                && stem.bytes().all(|byte| byte.is_ascii_alphanumeric())
        })
        .ok_or_else(|| ApiError::bad_request("Invalid image upload ID"))
}

fn metadata_path(sub: &str, stem: &str) -> Path {
    Path::from("profile-media-uploads")
        .child(sub)
        .child(format!("{stem}.json"))
}

fn image_path(sub: &str, filename: &str) -> Path {
    Path::from("media")
        .child("users")
        .child(sub)
        .child(filename)
}

/// Allocate a private upload without changing the account or profile image.
/// The metadata binds completion to the authenticated user and the image field.
pub(crate) async fn prepare_upload(
    state: &AppState,
    sub: &str,
    extension: &str,
    scope: &str,
    previous_image: Option<&str>,
) -> Result<(String, String), ApiError> {
    allocate_upload(state, sub, extension, scope, previous_image, None).await
}

fn sync_slot_path(sub: &str, scope: &str) -> Path {
    let key = flow_like_storage::blake3::hash(scope.as_bytes())
        .to_hex()
        .to_string();
    Path::from("profile-media-upload-slots")
        .child(sub)
        .child(format!("{key}.json"))
}

pub(crate) async fn prepare_sync_upload(
    state: &AppState,
    sub: &str,
    extension: &str,
    scope: &str,
    previous_image: Option<&str>,
    source_revision: Option<&str>,
) -> Result<(String, String), ApiError> {
    allocate_upload(
        state,
        sub,
        extension,
        scope,
        previous_image,
        source_revision,
    )
    .await
}

async fn allocate_upload(
    state: &AppState,
    sub: &str,
    extension: &str,
    scope: &str,
    previous_image: Option<&str>,
    source_revision: Option<&str>,
) -> Result<(String, String), ApiError> {
    let extension = normalize_image_extension(extension)?;
    let stem = flow_like_types::create_id();
    let store = state.master_credentials().await?.to_store(false).await?;
    let metadata = PendingUpload {
        scope: scope.to_owned(),
        previous_image: previous_image.map(str::to_owned),
        extension: extension.clone(),
        source_revision: source_revision.map(str::to_owned),
        expires_at: chrono::Utc::now().timestamp() + UPLOAD_LIFETIME_SECONDS,
    };
    store
        .as_generic()
        .put(
            &metadata_path(sub, &stem),
            serde_json::to_vec(&metadata)?.into(),
        )
        .await?;
    if source_revision.is_some() {
        store
            .as_generic()
            .put(
                &sync_slot_path(sub, scope),
                serde_json::to_vec(&format!("{stem}.webp"))?.into(),
            )
            .await?;
    }
    let url = store
        .sign(
            "PUT",
            &image_path(sub, &format!("{stem}.{extension}")),
            Duration::from_secs(UPLOAD_LIFETIME_SECONDS as u64),
        )
        .await?;
    Ok((url.to_string(), format!("{stem}.webp")))
}

/// A failed transfer can be retried after metadata sync advances the server time.
/// Reuse only a stage for the same local revision and unchanged remote image.
pub(crate) async fn retry_sync_upload(
    state: &AppState,
    sub: &str,
    extension: &str,
    scope: &str,
    current_image: Option<&str>,
    source_revision: Option<&str>,
) -> Result<Option<(String, String)>, ApiError> {
    let Some(source_revision) = source_revision else {
        return Ok(None);
    };
    let store = state.master_credentials().await?.to_store(false).await?;
    let slot = match store.as_generic().get(&sync_slot_path(sub, scope)).await {
        Ok(slot) => slot.bytes().await?,
        Err(StoreError::NotFound { .. }) => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let upload_id: String = serde_json::from_slice(&slot)?;
    let stem = upload_stem(&upload_id)?;
    let metadata = match store.as_generic().get(&metadata_path(sub, stem)).await {
        Ok(metadata) => metadata.bytes().await?,
        Err(StoreError::NotFound { .. }) => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let metadata: PendingUpload = serde_json::from_slice(&metadata)?;
    let extension = normalize_image_extension(extension)?;
    if !can_retry_sync_upload(&metadata, scope, &extension, current_image, source_revision) {
        return Ok(None);
    }
    let remaining = metadata.expires_at - chrono::Utc::now().timestamp();
    if remaining < 60 {
        return prepare_sync_upload(
            state,
            sub,
            &extension,
            scope,
            current_image,
            Some(source_revision),
        )
        .await
        .map(Some);
    }
    let url = store
        .sign(
            "PUT",
            &image_path(sub, &format!("{stem}.{extension}")),
            Duration::from_secs(remaining as u64),
        )
        .await?;
    Ok(Some((url.to_string(), upload_id)))
}

fn can_retry_sync_upload(
    metadata: &PendingUpload,
    scope: &str,
    extension: &str,
    current_image: Option<&str>,
    source_revision: &str,
) -> bool {
    metadata.scope == scope
        && metadata.source_revision.as_deref() == Some(source_revision)
        && metadata.previous_image.as_deref() == current_image
        && metadata.extension == extension
}

/// Accept a concurrent duplicate only if every requested field is already stored.
pub(crate) fn mutation_matches<A: ActiveModelTrait>(
    expected: &A,
    current: &A,
    timestamp: <A::Entity as EntityTrait>::Column,
) -> bool {
    <A::Entity as EntityTrait>::Column::iter().all(|column| {
        if column.as_str() == timestamp.as_str() {
            return true;
        }
        match expected.get(column) {
            ActiveValue::Set(value) => current.get(column).unwrap() == value,
            _ => true,
        }
    })
}

/// Cleanup is best effort after the database commit and never turns success into failure.
pub(crate) async fn cleanup_upload(
    state: &AppState,
    sub: &str,
    upload_id: &str,
    previous_image: Option<&str>,
) {
    if let Err(error) = cleanup_stored_upload(state, sub, upload_id, previous_image).await {
        tracing::warn!("Could not clean up completed profile image upload: {error:?}");
    }
}

async fn cleanup_stored_upload(
    state: &AppState,
    sub: &str,
    upload_id: &str,
    previous_image: Option<&str>,
) -> Result<(), ApiError> {
    let stem = upload_stem(upload_id)?;
    let store = state.master_credentials().await?.to_store(false).await?;
    let metadata = store
        .as_generic()
        .get(&metadata_path(sub, stem))
        .await?
        .bytes()
        .await?;
    let metadata: PendingUpload = serde_json::from_slice(&metadata)?;
    for filename in [
        upload_id.to_owned(),
        format!("{stem}.{}", metadata.extension),
    ] {
        match store.as_generic().delete(&image_path(sub, &filename)).await {
            Ok(_) | Err(StoreError::NotFound { .. }) => {}
            Err(error) => return Err(error.into()),
        }
    }
    let Some(previous) = previous_image else {
        return Ok(());
    };
    let canonical = crate::routes::user::avatar_file_name(previous);
    if canonical == format!("{stem}-published.webp")
        || previous.contains('/')
        || previous.contains('\\')
    {
        return Ok(());
    }
    let avatar_in_use = crate::entity::user::Entity::find_by_id(sub)
        .one(&state.db)
        .await?
        .and_then(|user| user.avatar)
        .is_some_and(|avatar| crate::routes::user::avatar_file_name(&avatar) == canonical);
    let references = crate::entity::profile::Entity::find()
        .filter(crate::entity::profile::Column::UserId.eq(sub))
        .select_only()
        .columns([
            crate::entity::profile::Column::Icon,
            crate::entity::profile::Column::Thumbnail,
        ])
        .into_tuple::<(Option<String>, Option<String>)>()
        .all(&state.db)
        .await?;
    let profile_in_use = references.into_iter().any(|(icon, thumbnail)| {
        icon.into_iter()
            .chain(thumbnail)
            .any(|image| crate::routes::user::avatar_file_name(&image) == canonical)
    });
    if !avatar_in_use && !profile_in_use {
        match store
            .as_generic()
            .delete(&image_path(sub, &canonical))
            .await
        {
            Ok(_) | Err(StoreError::NotFound { .. }) => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

pub(crate) async fn finalize_upload(
    state: &AppState,
    sub: &str,
    upload_id: &str,
    scope: &str,
    current_image: Option<&str>,
) -> Result<Option<String>, ApiError> {
    let store = state.master_credentials().await?.to_store(false).await?;
    finalize_stored_upload(
        store.as_generic().as_ref(),
        sub,
        upload_id,
        scope,
        current_image,
    )
    .await
}

/// None means the asynchronous image transformer has not produced a WebP yet.
/// Publish validated bytes to a separate path that has never had a signed PUT URL.
async fn finalize_stored_upload(
    store: &dyn ObjectStore,
    sub: &str,
    upload_id: &str,
    scope: &str,
    current_image: Option<&str>,
) -> Result<Option<String>, ApiError> {
    let stem = upload_stem(upload_id)?;
    let metadata = match store.get(&metadata_path(sub, stem)).await {
        Ok(metadata) => metadata.bytes().await?,
        Err(StoreError::NotFound { .. }) => {
            return Err(ApiError::bad_request("Image upload not found"));
        }
        Err(error) => return Err(error.into()),
    };
    let metadata: PendingUpload = serde_json::from_slice(&metadata)?;
    if metadata.scope != scope {
        return Err(ApiError::bad_request(
            "Image upload belongs to a different profile or image field",
        ));
    }
    let published_id = format!("{stem}-published.webp");
    if current_image == Some(published_id.as_str()) {
        return Ok(Some(published_id));
    }
    if metadata.expires_at < chrono::Utc::now().timestamp() {
        return Err(ApiError::bad_request(
            "Image upload expired. Select the image again",
        ));
    }

    if current_image != metadata.previous_image.as_deref() {
        return Err(ApiError::conflict(
            "The image changed while this upload was pending. Select the image again",
        ));
    }
    let published_path = image_path(sub, &published_id);
    match store.head(&published_path).await {
        Ok(_) => return Ok(Some(published_id)),
        Err(StoreError::NotFound { .. }) => {}
        Err(error) => return Err(error.into()),
    }
    let image = match store.get(&image_path(sub, upload_id)).await {
        Ok(image) => image,
        Err(StoreError::NotFound { .. }) => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if image.meta.size == 0 || image.meta.size > MAX_IMAGE_BYTES {
        return Err(ApiError::bad_request(
            "Image must contain data and be smaller than 20 MB",
        ));
    }
    let bytes = image.bytes().await?;
    let validated = bytes.clone();
    flow_like_types::tokio::task::spawn_blocking(move || validate_image(&validated))
        .await
        .map_err(|error| ApiError::internal(error.to_string()))??;
    let options = PutOptions {
        mode: PutMode::Create,
        ..Default::default()
    };
    match store.put_opts(&published_path, bytes.into(), options).await {
        Ok(_) | Err(StoreError::AlreadyExists { .. }) => Ok(Some(published_id)),
        Err(error) => Err(error.into()),
    }
}

fn validate_image(bytes: &[u8]) -> Result<(), ApiError> {
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), image::ImageFormat::WebP);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16384);
    limits.max_image_height = Some(16384);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    reader
        .decode()
        .map_err(|_| ApiError::bad_request("The uploaded image could not be decoded"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_storage::object_store::memory::InMemory;

    async fn pending(store: &InMemory) {
        store
            .put(
                &metadata_path("owner", "upload"),
                serde_json::to_vec(&PendingUpload {
                    scope: "profile:workspace:icon".into(),
                    previous_image: None,
                    extension: "webp".into(),
                    source_revision: None,
                    expires_at: chrono::Utc::now().timestamp() + 3600,
                })
                .unwrap()
                .into(),
            )
            .await
            .unwrap();
    }

    #[test]
    fn rejects_paths_and_unsupported_extensions() {
        for id in [
            "../other.webp",
            "other/file.webp",
            "https://host/image.webp",
            "id.png",
            "",
        ] {
            assert!(upload_stem(id).is_err(), "{id}");
        }
        assert_eq!(normalize_image_extension(" .JPG ").unwrap(), "jpg");
        assert!(normalize_image_extension("../png").is_err());
        assert!(normalize_image_extension("svg").is_err());
    }

    #[test]
    fn sync_retry_requires_the_same_local_revision_and_unchanged_remote_image() {
        let metadata = PendingUpload {
            scope: "profile:workspace:icon".into(),
            previous_image: Some("previous.webp".into()),
            extension: "png".into(),
            source_revision: Some("local-revision".into()),
            expires_at: 0,
        };
        assert!(can_retry_sync_upload(
            &metadata,
            "profile:workspace:icon",
            "png",
            Some("previous.webp"),
            "local-revision"
        ));
        assert!(!can_retry_sync_upload(
            &metadata,
            "profile:workspace:icon",
            "png",
            Some("newer.webp"),
            "local-revision"
        ));
        assert!(!can_retry_sync_upload(
            &metadata,
            "profile:workspace:icon",
            "png",
            Some("previous.webp"),
            "older-revision"
        ));
        assert!(!can_retry_sync_upload(
            &metadata,
            "profile:other:icon",
            "png",
            Some("previous.webp"),
            "local-revision"
        ));
    }

    #[test]
    fn concurrent_completion_matches_all_requested_values() {
        use crate::entity::user;
        use sea_orm::ActiveValue::Set;
        let requested = user::ActiveModel {
            name: Set(Some("Updated name".into())),
            avatar: Set(Some("upload-published.webp".into())),
            ..Default::default()
        };
        let mut stored = requested.clone();
        assert!(mutation_matches(
            &requested,
            &stored,
            user::Column::UpdatedAt
        ));
        stored.name = Set(Some("Other name".into()));
        assert!(!mutation_matches(
            &requested,
            &stored,
            user::Column::UpdatedAt
        ));
    }

    #[flow_like_types::tokio::test]
    async fn waits_for_conversion_and_rejects_wrong_owner_scope_and_invalid_bytes() {
        let store = InMemory::new();
        pending(&store).await;
        assert!(
            finalize_stored_upload(
                &store,
                "other",
                "upload.webp",
                "profile:workspace:icon",
                None
            )
            .await
            .is_err()
        );
        assert!(
            finalize_stored_upload(&store, "owner", "upload.webp", "profile:other:icon", None)
                .await
                .is_err()
        );
        assert_eq!(
            finalize_stored_upload(
                &store,
                "owner",
                "upload.webp",
                "profile:workspace:icon",
                None
            )
            .await
            .unwrap(),
            None
        );
        store
            .put(&image_path("owner", "upload.webp"), "not an image".into())
            .await
            .unwrap();
        assert!(
            finalize_stored_upload(
                &store,
                "owner",
                "upload.webp",
                "profile:workspace:icon",
                None
            )
            .await
            .is_err()
        );
        assert!(
            store
                .head(&image_path("owner", "upload-published.webp"))
                .await
                .is_err()
        );
    }

    #[flow_like_types::tokio::test]
    async fn completion_is_idempotent_and_later_signed_uploads_cannot_replace_published_bytes() {
        let store = InMemory::new();
        pending(&store).await;
        let mut encoded = Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(1, 1)
            .write_to(&mut encoded, image::ImageFormat::WebP)
            .unwrap();
        let bytes = encoded.into_inner();
        store
            .put(&image_path("owner", "upload.webp"), bytes.clone().into())
            .await
            .unwrap();
        let expected = Some("upload-published.webp".into());
        assert_eq!(
            finalize_stored_upload(
                &store,
                "owner",
                "upload.webp",
                "profile:workspace:icon",
                None
            )
            .await
            .unwrap(),
            expected
        );
        store
            .put(&image_path("owner", "upload.webp"), "replacement".into())
            .await
            .unwrap();
        assert_eq!(
            finalize_stored_upload(
                &store,
                "owner",
                "upload.webp",
                "profile:workspace:icon",
                None
            )
            .await
            .unwrap(),
            expected
        );
        assert_eq!(
            store
                .get(&image_path("owner", "upload-published.webp"))
                .await
                .unwrap()
                .bytes()
                .await
                .unwrap()
                .as_ref(),
            bytes.as_slice()
        );
        assert!(
            finalize_stored_upload(
                &store,
                "owner",
                "upload.webp",
                "profile:workspace:icon",
                Some("newer-published.webp")
            )
            .await
            .is_err()
        );
        assert_eq!(
            finalize_stored_upload(
                &store,
                "owner",
                "upload.webp",
                "profile:workspace:icon",
                Some("upload-published.webp")
            )
            .await
            .unwrap(),
            expected
        );
    }

    #[flow_like_types::tokio::test]
    async fn expired_uploads_cannot_be_published() {
        let store = InMemory::new();
        store
            .put(
                &metadata_path("owner", "upload"),
                serde_json::to_vec(&PendingUpload {
                    scope: "avatar".into(),
                    previous_image: None,
                    extension: "webp".into(),
                    source_revision: None,
                    expires_at: chrono::Utc::now().timestamp() - 1,
                })
                .unwrap()
                .into(),
            )
            .await
            .unwrap();
        assert!(
            finalize_stored_upload(&store, "owner", "upload.webp", "avatar", None)
                .await
                .is_err()
        );
        assert_eq!(
            finalize_stored_upload(
                &store,
                "owner",
                "upload.webp",
                "avatar",
                Some("upload-published.webp")
            )
            .await
            .unwrap(),
            Some("upload-published.webp".into())
        );
    }
}

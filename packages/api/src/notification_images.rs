use std::time::Duration;

use flow_like_storage::{Path, files::store::FlowLikeStore, object_store::ObjectStoreExt};
use flow_like_types::{
    Result,
    base64::{Engine, engine::general_purpose::STANDARD},
    images::notification_thumbnail,
};

use crate::state::AppState;

const IMAGE_REFERENCE_PREFIX: &str = "notification-image:";
// A 256px RGBA PNG fits comfortably below this bound, including base64 overhead.
const MAX_INLINE_ICON_LENGTH: usize = 512 * 1024;
const IMAGE_URL_LIFETIME: Duration = Duration::from_secs(24 * 60 * 60);

pub struct PreparedNotificationIcon {
    pub stored_icon: Option<String>,
    pub push_icon: Option<String>,
}

fn image_filename(reference: &str) -> Option<&str> {
    let filename = reference.strip_prefix(IMAGE_REFERENCE_PREFIX)?;
    let hash = filename.strip_suffix(".png")?;
    (hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())).then_some(filename)
}

fn image_path(user_id: &str, filename: &str) -> Path {
    // Derive the namespace from the authenticated notification recipient. A supplied
    // reference can never select an object belonging to another user.
    Path::from("notification-images")
        .join(blake3::hash(user_id.as_bytes()).to_hex().to_string())
        .join(filename)
}

fn decode_inline_icon(icon: &str) -> Result<Vec<u8>> {
    if icon.len() > MAX_INLINE_ICON_LENGTH {
        flow_like_types::bail!("Notification icon exceeds the 512 KiB upload limit");
    }
    let (header, encoded) = icon
        .split_once(',')
        .ok_or_else(|| flow_like_types::anyhow!("Invalid notification image data URL"))?;
    if !matches!(
        header,
        "data:image/png;base64"
            | "data:image/jpeg;base64"
            | "data:image/gif;base64"
            | "data:image/webp;base64"
    ) {
        flow_like_types::bail!("Notification icon must contain a PNG, JPEG, GIF or WebP image");
    }
    notification_thumbnail(&STANDARD.decode(encoded)?)
}

async fn store_inline_icon(store: &FlowLikeStore, user_id: &str, icon: String) -> Result<String> {
    let png =
        flow_like_types::tokio::task::spawn_blocking(move || decode_inline_icon(&icon)).await??;
    let filename = format!("{}.png", blake3::hash(&png).to_hex());
    store
        .as_generic()
        .put(&image_path(user_id, &filename), png.into())
        .await?;
    Ok(format!("{IMAGE_REFERENCE_PREFIX}{filename}"))
}

async fn resolve_stored_icon(
    store: &FlowLikeStore,
    user_id: &str,
    reference: &str,
) -> Result<String> {
    let filename = image_filename(reference)
        .ok_or_else(|| flow_like_types::anyhow!("Invalid notification image reference"))?;
    let path = image_path(user_id, filename);
    store.as_generic().head(&path).await?;
    Ok(store
        .sign_cached("GET", &path, IMAGE_URL_LIFETIME)
        .await?
        .to_string())
}

/// Persist image bytes independently of the expiring URL sent to a device.
pub async fn prepare_notification_icon(
    state: &AppState,
    user_id: &str,
    icon: Option<&str>,
) -> Result<PreparedNotificationIcon> {
    let Some(icon) = icon.map(str::trim).filter(|icon| !icon.is_empty()) else {
        return Ok(PreparedNotificationIcon {
            stored_icon: None,
            push_icon: None,
        });
    };

    if icon.starts_with("data:") || icon.starts_with(IMAGE_REFERENCE_PREFIX) {
        let store = state.master_credentials().await?.to_store(false).await?;
        let reference = if icon.starts_with("data:") {
            store_inline_icon(&store, user_id, icon.to_owned()).await?
        } else {
            icon.to_owned()
        };
        let url = resolve_stored_icon(&store, user_id, &reference).await?;
        return Ok(PreparedNotificationIcon {
            stored_icon: Some(reference),
            push_icon: Some(url),
        });
    }

    // Preserve existing public image URLs and bundled UI icons. Do not fetch
    // caller-provided URLs with the API server's network privileges.
    let named_icon = icon.len() <= 128
        && icon
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
    let portable = named_icon
        || icon.starts_with("https://")
        || (icon.starts_with('/') && !icon.starts_with("//"));
    Ok(PreparedNotificationIcon {
        stored_icon: portable.then(|| icon.to_owned()),
        push_icon: icon.starts_with("https://").then(|| icon.to_owned()),
    })
}

pub async fn refresh_notification_icons(
    state: &AppState,
    notifications: &mut [crate::entity::notification::Model],
) -> Result<()> {
    if !notifications.iter().any(|item| {
        item.icon
            .as_deref()
            .is_some_and(|icon| icon.starts_with(IMAGE_REFERENCE_PREFIX))
    }) {
        return Ok(());
    }
    let store = match async { state.master_credentials().await?.to_store(false).await }.await {
        Ok(store) => store,
        Err(error) => {
            tracing::warn!(%error, "Could not access notification image storage");
            for notification in notifications {
                if notification
                    .icon
                    .as_deref()
                    .is_some_and(|icon| icon.starts_with(IMAGE_REFERENCE_PREFIX))
                {
                    notification.icon = None;
                }
            }
            return Ok(());
        }
    };
    for notification in notifications {
        let Some(reference) = notification
            .icon
            .as_deref()
            .filter(|icon| icon.starts_with(IMAGE_REFERENCE_PREFIX))
        else {
            continue;
        };
        notification.icon = match resolve_stored_icon(&store, &notification.user_id, reference)
            .await
        {
            Ok(url) => Some(url),
            Err(error) => {
                tracing::warn!(notification_id = %notification.id, %error, "Could not load notification image");
                None
            }
        };
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_storage::object_store::memory::InMemory;
    use flow_like_types::tokio;
    use std::{io::Cursor, sync::Arc};

    fn inline_icon() -> String {
        let mut png = Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(32, 16)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        format!(
            "data:image/png;base64,{}",
            STANDARD.encode(png.into_inner())
        )
    }

    #[tokio::test]
    async fn durable_icon_is_deterministic_and_resolves_from_storage() {
        let store = FlowLikeStore::Memory(Arc::new(InMemory::new()));
        let first = store_inline_icon(&store, "user-a", inline_icon())
            .await
            .unwrap();
        let second = store_inline_icon(&store, "user-a", inline_icon())
            .await
            .unwrap();
        assert_eq!(first, second);
        assert!(first.starts_with(IMAGE_REFERENCE_PREFIX));
        let resolved = resolve_stored_icon(&store, "user-a", &first).await.unwrap();
        assert!(resolved.starts_with("data:image/png;base64,"));
        assert!(resolve_stored_icon(&store, "user-b", &first).await.is_err());
    }

    #[test]
    fn rejects_malformed_references_and_unbounded_data() {
        assert!(image_filename("notification-image:../../other.png").is_none());
        assert!(image_filename("notification-image:abc.png").is_none());
        assert!(decode_inline_icon("data:text/html;base64,PHNjcmlwdD4=").is_err());
        assert!(decode_inline_icon("data:image/png;base64,bm90IHB uZw==").is_err());
        assert!(
            decode_inline_icon(&format!(
                "data:image/png;base64,{}",
                "A".repeat(MAX_INLINE_ICON_LENGTH)
            ))
            .is_err()
        );
    }
}

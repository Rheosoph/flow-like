use std::time::Duration;

use flow_like_storage::{Path, files::store::FlowLikeStore, object_store::ObjectStoreExt};
use flow_like_types::Result;
use futures_util::TryStreamExt;

use crate::state::AppState;

const IMAGE_REFERENCE_PREFIX: &str = "notification-image:";
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

fn user_directory(user_id: &str) -> Path {
    Path::from("notification-images").join(blake3::hash(user_id.as_bytes()).to_hex().to_string())
}

fn image_path(user_id: &str, filename: &str) -> Path {
    // Derive the namespace from the authenticated notification recipient. A supplied
    // reference can never select an object belonging to another user.
    user_directory(user_id).join(filename)
}

fn bot_avatar_directory(owner_id: &str, bot_id: &str) -> Path {
    user_directory(owner_id).join("bots").join(bot_id)
}

fn bot_avatar_path(owner_id: &str, bot_id: &str, look: &str) -> Path {
    bot_avatar_directory(owner_id, bot_id).join(format!("{look}.png"))
}

pub use flow_like_types::images::inline_image_thumbnail as decode_inline_icon;

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

/// Stores the picture of one look of a bot and drops the pictures of its earlier looks.
pub async fn store_bot_avatar(
    store: &FlowLikeStore,
    owner_id: &str,
    bot_id: &str,
    look: &str,
    png: Vec<u8>,
) -> Result<()> {
    let generic = store.as_generic();
    let path = bot_avatar_path(owner_id, bot_id, look);
    generic.put(&path, png.into()).await?;
    let earlier: Vec<Path> = generic
        .list(Some(&bot_avatar_directory(owner_id, bot_id)))
        .map_ok(|meta| meta.location)
        .try_filter(|location| std::future::ready(*location != path))
        .try_collect()
        .await?;
    for location in earlier {
        if let Err(error) = generic.delete(&location).await {
            tracing::warn!(%error, %location, "Could not delete the picture of a bot's earlier look");
        }
    }
    Ok(())
}

/// A link to the picture of this look of the bot; `None` until a client stored one.
pub async fn bot_avatar_url(
    store: &FlowLikeStore,
    owner_id: &str,
    bot_id: &str,
    look: &str,
) -> Result<Option<String>> {
    let path = bot_avatar_path(owner_id, bot_id, look);
    match store.as_generic().head(&path).await {
        Ok(_) => {}
        Err(flow_like_storage::object_store::Error::NotFound { .. }) => return Ok(None),
        Err(error) => return Err(error.into()),
    }
    Ok(Some(
        store
            .sign_cached("GET", &path, IMAGE_URL_LIFETIME)
            .await?
            .to_string(),
    ))
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
    use flow_like_types::{
        base64::{Engine, engine::general_purpose::STANDARD},
        images::MAX_INLINE_IMAGE_LENGTH,
        tokio,
    };
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

    #[tokio::test]
    async fn a_bot_keeps_only_the_picture_of_its_latest_look() {
        async fn stored(store: &FlowLikeStore, owner: &str, bot: &str, look: &str) -> bool {
            bot_avatar_url(store, owner, bot, look)
                .await
                .unwrap()
                .is_some()
        }
        let store = FlowLikeStore::Memory(Arc::new(InMemory::new()));
        let png = decode_inline_icon(&inline_icon()).unwrap();
        store_bot_avatar(&store, "owner", "bot_10", "look-a", png.clone())
            .await
            .unwrap();
        store_bot_avatar(&store, "owner", "bot_1", "look-a", png.clone())
            .await
            .unwrap();
        assert!(stored(&store, "owner", "bot_1", "look-a").await);

        store_bot_avatar(&store, "owner", "bot_1", "look-b", png)
            .await
            .unwrap();
        assert!(!stored(&store, "owner", "bot_1", "look-a").await);
        assert!(stored(&store, "owner", "bot_1", "look-b").await);
        assert!(stored(&store, "owner", "bot_10", "look-a").await);
        assert!(!stored(&store, "other", "bot_1", "look-b").await);
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
                "A".repeat(MAX_INLINE_IMAGE_LENGTH)
            ))
            .is_err()
        );
    }
}

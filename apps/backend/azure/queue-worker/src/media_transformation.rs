//! WebP normalisation of uploaded media, the Azure port of
//! `apps/backend/aws/media-transformer`.
//!
//! Every `BlobCreated` under `media/` in the released content container is
//! decoded, resized (square 1024, landscape 1280x720 cover-crop, portrait fit
//! 1280), encoded as lossy WebP quality 92 and written back next to the
//! original as `<basename>.webp`; the original is then deleted. `.webp` inputs
//! are ignored (that is the loop breaker for the worker's own output), videos
//! are kept untouched, and any other unsupported extension is deleted, exactly
//! as on AWS. Blob access is the workload identity through `object_store`;
//! no key or SAS is involved.

use crate::blob_events::BlobEvent;
use flow_like_storage::object_store::{
    Attribute, AttributeValue, Attributes, ObjectStore, PutOptions, PutPayload, path::Path,
};
use image::{GenericImageView, ImageReader};
use std::io::Cursor;
use std::sync::Arc;
use webp::Encoder;

const WEBP_QUALITY: f32 = 92.0;
const MEDIA_PREFIX: &str = "media/";

pub struct MediaTransformationContext {
    pub store: Arc<dyn ObjectStore>,
    pub content_container: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum MediaTransformationError {
    Permanent(&'static str),
    Retryable(&'static str),
}

fn is_supported_image_format(extension: &str) -> bool {
    matches!(
        extension.to_ascii_lowercase().as_str(),
        "jpg" | "jpeg" | "jfif" | "png" | "gif" | "bmp" | "tif" | "tiff" | "avif" | "ico"
    )
}

fn is_video_format(extension: &str) -> bool {
    matches!(
        extension.to_ascii_lowercase().as_str(),
        "mp4" | "mov" | "avi" | "mkv" | "flv" | "wmv"
    )
}

/// Same key, same directory, extension replaced by `.webp`.
fn generate_webp_key(key: &str) -> Option<String> {
    if !key.starts_with(MEDIA_PREFIX) {
        return None;
    }
    Some(match key.rfind('.') {
        Some(last_dot) => format!("{}.webp", &key[..last_dot]),
        None => format!("{key}.webp"),
    })
}

fn map_store_error(
    operation: &'static str,
    error: flow_like_storage::object_store::Error,
) -> MediaTransformationError {
    tracing::warn!(error = %error, operation, "media blob operation failed");
    MediaTransformationError::Retryable("media_blob_store_unavailable")
}

pub async fn process(
    ctx: &MediaTransformationContext,
    event: &BlobEvent,
) -> Result<(), MediaTransformationError> {
    let (container, key) = event
        .container_and_key()
        .ok_or(MediaTransformationError::Permanent("invalid_blob_subject"))?;
    if container != ctx.content_container {
        tracing::warn!(
            container,
            "blob event for a container this worker does not serve"
        );
        return Ok(());
    }
    if event.is_deletion() {
        return Ok(());
    }

    let extension = key.split('.').next_back().unwrap_or("");
    if extension.eq_ignore_ascii_case("webp") {
        return Ok(());
    }
    if !is_supported_image_format(extension) {
        if is_video_format(extension) {
            tracing::info!(key, "video upload kept as-is");
            return Ok(());
        }
        tracing::warn!(key, extension, "deleting unsupported media upload");
        return delete(ctx, key).await;
    }

    let target_key = generate_webp_key(key).ok_or(MediaTransformationError::Permanent(
        "media_key_outside_prefix",
    ))?;
    convert_and_store(ctx, key, &target_key).await?;
    delete(ctx, key).await
}

async fn delete(
    ctx: &MediaTransformationContext,
    key: &str,
) -> Result<(), MediaTransformationError> {
    match ctx.store.delete(&Path::from(key)).await {
        Ok(()) | Err(flow_like_storage::object_store::Error::NotFound { .. }) => Ok(()),
        Err(error) => Err(map_store_error("delete", error)),
    }
}

async fn convert_and_store(
    ctx: &MediaTransformationContext,
    source_key: &str,
    target_key: &str,
) -> Result<(), MediaTransformationError> {
    let target = Path::from(target_key);
    match ctx.store.head(&target).await {
        Ok(_) => {
            tracing::info!(
                target_key,
                "target WebP already exists, skipping conversion"
            );
            return Ok(());
        }
        Err(flow_like_storage::object_store::Error::NotFound { .. }) => {}
        Err(error) => return Err(map_store_error("head", error)),
    }

    let source = Path::from(source_key);
    let image_data = match ctx.store.get(&source).await {
        Ok(result) => result
            .bytes()
            .await
            .map_err(|error| map_store_error("get", error))?,
        // The original is already gone (a retry after a completed run); the
        // webp exists or nothing is left to convert either way.
        Err(flow_like_storage::object_store::Error::NotFound { .. }) => return Ok(()),
        Err(error) => return Err(map_store_error("get", error)),
    };

    let key_for_log = source_key.to_string();
    let webp_data = tokio::task::spawn_blocking(move || decode_resize_encode_webp(&image_data))
        .await
        .map_err(|_| MediaTransformationError::Retryable("media_transform_task_failed"))?
        .map_err(|error| {
            tracing::warn!(error = %error, key = %key_for_log, "image could not be transformed");
            MediaTransformationError::Permanent("media_transform_failed")
        })?;

    let attributes = Attributes::from_iter([
        (Attribute::ContentType, AttributeValue::from("image/webp")),
        (
            Attribute::CacheControl,
            AttributeValue::from("public, max-age=31536000, immutable"),
        ),
    ]);
    ctx.store
        .put_opts(
            &target,
            PutPayload::from(webp_data),
            PutOptions {
                attributes,
                ..Default::default()
            },
        )
        .await
        .map_err(|error| map_store_error("put", error))?;
    Ok(())
}

fn decode_resize_encode_webp(image_data: &[u8]) -> Result<Vec<u8>, String> {
    let reader = ImageReader::new(Cursor::new(image_data))
        .with_guessed_format()
        .map_err(|error| error.to_string())?;
    let decoded = reader.decode().map_err(|error| error.to_string())?;
    let resized = resize_image(decoded);
    encode_as_webp(resized)
}

fn resize_image(img: image::DynamicImage) -> image::DynamicImage {
    let (width, height) = img.dimensions();
    let filter = image::imageops::FilterType::CatmullRom;
    if width == height {
        img.resize(1024, 1024, filter)
    } else if width > height {
        img.resize_to_fill(1280, 720, filter)
    } else {
        img.resize(1280, 1280, filter)
    }
}

fn encode_as_webp(img: image::DynamicImage) -> Result<Vec<u8>, String> {
    let encoded = if img.color().has_alpha() {
        let rgba = img.to_rgba8();
        Encoder::from_rgba(&rgba, rgba.width(), rgba.height()).encode(WEBP_QUALITY)
    } else {
        let rgb = img.to_rgb8();
        Encoder::from_rgb(&rgb, rgb.width(), rgb.height()).encode(WEBP_QUALITY)
    };
    Ok(encoded.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webp_key_replaces_extension_inside_media_prefix() {
        assert_eq!(
            generate_webp_key("media/folder/image.JPG").as_deref(),
            Some("media/folder/image.webp")
        );
        assert_eq!(
            generate_webp_key("media/no-extension").as_deref(),
            Some("media/no-extension.webp")
        );
        assert_eq!(generate_webp_key("apps/x/image.jpg"), None);
    }

    #[test]
    fn format_gates_match_the_aws_worker() {
        assert!(is_supported_image_format("JPEG"));
        assert!(is_supported_image_format("avif"));
        assert!(!is_supported_image_format("webp"));
        assert!(is_video_format("MOV"));
        assert!(!is_video_format("svg"));
    }
}

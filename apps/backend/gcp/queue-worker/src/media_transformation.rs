//! WebP normalisation of uploaded media, the GCP port of
//! `apps/backend/aws/media-transformer`.
//!
//! Typed course assets under `media/courses/<id>/assets/<file>` are preserved;
//! the course asset API manages their formats. Other `OBJECT_FINALIZE` uploads
//! under `media/` in the released content bucket are
//! decoded, resized (square 1024, landscape 1280x720 cover-crop, portrait fit
//! 1280), encoded as lossy WebP quality 92 and written back next to the original
//! as `<basename>.webp`; the original is then deleted. `.webp` inputs are
//! ignored to avoid processing the worker's own output again. Videos are
//! kept untouched, and any other unsupported extension is deleted, exactly as on
//! AWS. Bucket access is the workload's own service account through
//! `object_store`'s metadata-server credential; no key or signed URL is involved.

use crate::gcs_events::GcsEvent;
use flow_like_storage::object_store::ObjectStoreExt;
use flow_like_storage::object_store::{
    Attribute, AttributeValue, Attributes, ObjectStore, PutOptions, PutPayload, path::Path,
};
use image::{GenericImageView, ImageReader};
use std::io::Cursor;
use std::sync::Arc;
use webp::Encoder;

const WEBP_QUALITY: f32 = 92.0;
const MEDIA_PREFIX: &str = "media/";

fn is_course_asset(key: &str) -> bool {
    let mut parts = key.split('/');
    matches!(
        (parts.next(), parts.next(), parts.next(), parts.next(), parts.next(), parts.next()),
        (Some("media"), Some("courses"), Some(course_id), Some("assets"), Some(file), None)
            if !course_id.is_empty() && !file.is_empty()
    )
}

pub struct MediaTransformationContext {
    pub store: Arc<dyn ObjectStore>,
    pub content_bucket: String,
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
    tracing::warn!(error = %error, operation, "media object operation failed");
    MediaTransformationError::Retryable("media_object_store_unavailable")
}

/// The notification carries the key exactly as stored, which is the
/// percent-encoded form `object_store` wrote; `Path::from` would encode it a
/// second time and address an object that does not exist.
fn stored_key(key: &str) -> Result<Path, MediaTransformationError> {
    Path::parse(key).map_err(|error| {
        tracing::warn!(error = %error, key, "media key is not a valid object path");
        MediaTransformationError::Permanent("invalid_media_key")
    })
}

pub async fn process(
    ctx: &MediaTransformationContext,
    event: &GcsEvent,
) -> Result<(), MediaTransformationError> {
    if event.bucket != ctx.content_bucket {
        tracing::warn!(
            bucket = %event.bucket,
            "object event for a bucket this worker does not serve"
        );
        return Ok(());
    }
    if event.is_deletion() {
        return Ok(());
    }

    let key = event.object.as_str();
    // The course asset API owns these filenames and handles image optimization explicitly.
    if is_course_asset(key) {
        return Ok(());
    }

    let extension = key.split('.').next_back().unwrap_or("");
    // The loop breaker. This worker's own output is a `.webp` under `media/`, so
    // without this arm every conversion would publish an OBJECT_FINALIZE that
    // fed the worker its own result forever.
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
    // The original is only removed once the derived object is durably stored:
    // `?` here is the whole rule. Deleting first, or deleting after a failed
    // put, would destroy the only copy of the upload.
    convert_and_store(ctx, key, &target_key).await?;
    delete(ctx, key).await
}

async fn delete(
    ctx: &MediaTransformationContext,
    key: &str,
) -> Result<(), MediaTransformationError> {
    match ctx.store.delete(&stored_key(key)?).await {
        Ok(()) | Err(flow_like_storage::object_store::Error::NotFound { .. }) => Ok(()),
        Err(error) => Err(map_store_error("delete", error)),
    }
}

async fn convert_and_store(
    ctx: &MediaTransformationContext,
    source_key: &str,
    target_key: &str,
) -> Result<(), MediaTransformationError> {
    let target = stored_key(target_key)?;
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

    let source = stored_key(source_key)?;
    let image_data = match ctx.store.get(&source).await {
        Ok(result) => result
            .bytes()
            .await
            .map_err(|error| map_store_error("get", error))?,
        // The original is already gone (a redelivery after a completed run); the
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
    fn course_asset_keys_exclude_banners_and_other_prefixes() {
        for file in ["training.csv", "cases.json", "practice.zip", "diagram.png"] {
            assert!(is_course_asset(&format!(
                "media/courses/course-1/assets/{file}"
            )));
        }
        for key in [
            "media/courses/course-1/banner.png",
            "media/courses/course-1/icon.jpg",
            "media/apps/app-1/assets/diagram.png",
            "media/courses//assets/cases.json",
            "media/courses/course-1/assets/",
            "media/courses/course-1/assets/nested/cases.json",
        ] {
            assert!(!is_course_asset(key), "{key}");
        }
    }

    fn created_event(key: &str) -> GcsEvent {
        GcsEvent {
            id: "practice-upload".into(),
            event_type: crate::gcs_events::OBJECT_FINALIZE.into(),
            bucket: "content".into(),
            object: key.into(),
            size: None,
            generation: Some(1),
            etag: None,
        }
    }

    #[tokio::test]
    async fn preserves_course_asset_bytes_and_still_transforms_banners() {
        let ctx = MediaTransformationContext {
            store: Arc::new(flow_like_storage::object_store::memory::InMemory::new()),
            content_bucket: "content".into(),
        };
        for file in ["cases.json", "practice.zip", "diagram.png"] {
            let key = format!("media/courses/course-1/assets/{file}");
            let path = Path::from(key.as_str());
            ctx.store.put(&path, "original bytes".into()).await.unwrap();
            process(&ctx, &created_event(&key)).await.unwrap();
            let bytes = ctx.store.get(&path).await.unwrap().bytes().await.unwrap();
            assert_eq!(bytes.as_ref(), b"original bytes");
            assert!(
                ctx.store
                    .head(&Path::from(generate_webp_key(&key).unwrap()))
                    .await
                    .is_err()
            );
        }

        let key = "media/courses/course-1/banner.png";
        let mut encoded = Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(2, 1)
            .write_to(&mut encoded, image::ImageFormat::Png)
            .unwrap();
        ctx.store
            .put(&Path::from(key), encoded.into_inner().into())
            .await
            .unwrap();
        process(&ctx, &created_event(key)).await.unwrap();
        assert!(ctx.store.head(&Path::from(key)).await.is_err());
        let output = ctx
            .store
            .get(&Path::from("media/courses/course-1/banner.webp"))
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        assert_eq!(
            image::guess_format(&output).unwrap(),
            image::ImageFormat::WebP
        );
    }

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

    #[test]
    fn stored_keys_are_addressed_verbatim() {
        let key = "media/apps/x/%C3%9Cbersicht (2)%231.jpg";
        assert_eq!(stored_key(key).unwrap().as_ref(), key);
        assert_ne!(Path::from(key).as_ref(), key);
        assert_eq!(
            stored_key("media/../x.jpg"),
            Err(MediaTransformationError::Permanent("invalid_media_key"))
        );
    }
}

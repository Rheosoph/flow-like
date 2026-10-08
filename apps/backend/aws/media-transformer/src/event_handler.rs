use aws_lambda_events::{
    event::s3::S3Event,
    s3::S3EventRecord,
    sqs::{SqsBatchResponse, SqsEvent},
};
use aws_sdk_s3::{primitives::ByteStream, Client as S3Client};
use image::{GenericImageView, ImageReader};
use lambda_runtime::{tracing, Error, LambdaEvent};
use std::io::Cursor;
use std::io::{BufRead, BufReader, Seek};
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use webp::Encoder;

const WEBP_QUALITY: f32 = 92.0;

fn is_course_asset(key: &str) -> bool {
    let mut parts = key.split('/');
    matches!(
        (parts.next(), parts.next(), parts.next(), parts.next(), parts.next(), parts.next()),
        (Some("media"), Some("courses"), Some(course_id), Some("assets"), Some(file), None)
            if !course_id.is_empty() && !file.is_empty()
    )
}

fn decode(key: &str) -> Result<String, Error> {
    let key = key.replace("+", " ");
    urlencoding::decode(&key)
        .map_err(|e| Error::from(format!("Failed to decode key: {}", e)))
        .map(|decoded| decoded.into_owned())
}

#[tracing::instrument(name = "SQS Function Handler", skip(event))]
pub(crate) async fn function_handler(
    event: LambdaEvent<SqsEvent>,
    s3_client: S3Client,
    bucket_name: String,
) -> Result<SqsBatchResponse, Error> {
    let mut batch_item_failures = Vec::new();

    for record in event.payload.records.iter() {
        let message_id = record.message_id.clone().unwrap_or_default();
        let body = &record.body;
        let Some(s3_event) = body.as_ref() else {
            tracing::error!("Record body is missing");
            batch_item_failures.push(aws_lambda_events::sqs::BatchItemFailure {
                item_identifier: message_id,
            });
            continue;
        };

        let s3_event: S3Event = match serde_json::from_str(s3_event) {
            Ok(event) => event,
            Err(err) => {
                tracing::error!("Failed to parse SQS message: {}", err);
                batch_item_failures.push(aws_lambda_events::sqs::BatchItemFailure {
                    item_identifier: message_id,
                });
                continue;
            }
        };

        if let Err(err) = process_s3_events(&s3_event.records, &s3_client, &bucket_name).await {
            tracing::error!("Error processing S3 event: {}", err);
            tracing::error!("Failed to process S3 event for message ID: {}", &message_id);
            batch_item_failures.push(aws_lambda_events::sqs::BatchItemFailure {
                item_identifier: message_id,
            });
        }
    }

    Ok(SqsBatchResponse {
        batch_item_failures,
    })
}

#[tracing::instrument(name = "Process S3 Events", skip(records))]
async fn process_s3_events(
    records: &[S3EventRecord],
    s3_client: &S3Client,
    bucket_name: &str,
) -> Result<(), Error> {
    let mut failed_records = 0;
    for record in records {
        if let Err(err) = process_single_record(record, s3_client, bucket_name).await {
            tracing::error!("Failed to process record: {}", err);
            failed_records += 1;
        }
    }

    if failed_records > 0 {
        return Err(Error::from(format!(
            "{} S3 record(s) failed in this SQS message",
            failed_records
        )));
    }

    Ok(())
}

#[tracing::instrument(name = "Process Single Event", skip(record, s3_client, bucket_name))]
async fn process_single_record(
    record: &S3EventRecord,
    s3_client: &S3Client,
    bucket_name: &str,
) -> Result<(), Error> {
    let object = &record.s3.object;
    let bucket = record
        .s3
        .bucket
        .name
        .as_ref()
        .ok_or_else(|| Error::from("Missing bucket name in S3 event"))?;

    let raw_key = object
        .key
        .as_ref()
        .ok_or_else(|| Error::from("Missing object key in S3 event"))?;

    let key = decode(raw_key).map_err(|e| {
        Error::from(format!(
            "Failed to decode S3 object key '{}': {}",
            raw_key, e
        ))
    })?;

    if bucket != bucket_name {
        tracing::warn!("Skipping object from different bucket: {}", bucket);
        return Ok(());
    }

    // The course asset API owns these filenames and handles image optimization explicitly.
    if is_course_asset(&key) {
        return Ok(());
    }

    let extension = key.split('.').next_back().unwrap_or("");

    if extension.eq_ignore_ascii_case("webp") {
        return Ok(());
    }

    if !is_supported_image_format(extension) {
        if is_video_format(extension) {
            tracing::info!("Skipping video file: {}", key);
            return Ok(());
        }

        tracing::info!("Deleting unsupported file type: {}", key);
        s3_client
            .delete_object()
            .bucket(bucket_name)
            .key(&key)
            .send()
            .await
            .map_err(|e| {
                Error::from(format!("Failed to delete unsupported file {}: {}", key, e))
            })?;
        return Ok(());
    }

    let converted_key = generate_webp_key(&key)?;
    convert_and_store_image(s3_client, bucket_name, &key, &converted_key).await?;

    s3_client
        .delete_object()
        .bucket(bucket_name)
        .key(&key)
        .send()
        .await
        .map_err(|e| Error::from(format!("Failed to delete original file {}: {}", key, e)))?;

    Ok(())
}

fn is_supported_image_format(extension: &str) -> bool {
    matches!(
        extension.to_ascii_lowercase().as_str(),
        "jpg" | "jpeg" | "jfif" | "png" | "gif" | "bmp" | "tif" | "tiff" | "avif" | "ico"
    )
}

fn is_video_format(extension: &str) -> bool {
    matches!(
        extension.to_lowercase().as_str(),
        "mp4" | "mov" | "avi" | "mkv" | "flv" | "wmv"
    )
}

fn generate_webp_key(key: &str) -> Result<String, Error> {
    if !key.starts_with("media/") {
        return Err(Error::from(format!(
            "Path must start with 'media/': {}",
            key
        )));
    }

    if let Some(last_dot) = key.rfind('.') {
        Ok(format!("{}.webp", &key[..last_dot]))
    } else {
        Ok(format!("{}.webp", key))
    }
}

#[tracing::instrument(name = "Convert and Store Image", skip(s3_client, bucket_name))]
async fn convert_and_store_image(
    s3_client: &S3Client,
    bucket_name: &str,
    source_key: &str,
    target_key: &str,
) -> Result<(), Error> {
    // Check if target already exists to avoid unnecessary work
    match s3_client
        .head_object()
        .bucket(bucket_name)
        .key(target_key)
        .send()
        .await
    {
        Ok(_) => {
            tracing::info!(
                "Target WebP already exists, skipping conversion: {}",
                target_key
            );
            return Ok(());
        }
        Err(_) => {
            // Target doesn't exist, proceed with conversion
        }
    }

    let response = s3_client
        .get_object()
        .bucket(bucket_name)
        .key(source_key)
        .send()
        .await
        .map_err(|e| {
            tracing::error!(
                "S3 GetObject failed for bucket='{}', key='{}': {:?}",
                bucket_name,
                source_key,
                e
            );
            Error::from(format!("Failed to download image {}: {}", source_key, e))
        })?;

    let staged = async {
        let mut image_data = tokio::fs::File::from_std(tempfile::tempfile()?);
        let mut body = response.body;
        while let Some(chunk) = body.try_next().await? {
            image_data.write_all(&chunk).await?;
        }
        image_data.flush().await?;
        image_data.rewind().await?;
        Ok::<_, Error>(image_data.into_std().await)
    }
    .await;
    let image_data: Box<dyn MediaReader> = match staged {
        Ok(file) => Box::new(BufReader::new(file)),
        Err(_) => {
            // Preserve the original in-memory path when temporary storage is unavailable.
            let bytes = s3_client
                .get_object()
                .bucket(bucket_name)
                .key(source_key)
                .send()
                .await?
                .body
                .collect()
                .await?
                .into_bytes();
            Box::new(Cursor::new(bytes))
        }
    };

    let source_key_for_transform = source_key.to_string();
    let webp_data = tokio::task::spawn_blocking(move || {
        decode_resize_encode_webp(image_data, &source_key_for_transform)
    })
    .await
    .map_err(|e| {
        Error::from(format!(
            "Image transform task failed for {}: {}",
            source_key, e
        ))
    })??;

    s3_client
        .put_object()
        .bucket(bucket_name)
        .key(target_key)
        .body(ByteStream::from(webp_data))
        .content_type("image/webp")
        .cache_control("public, max-age=31536000, immutable")
        .send()
        .await
        .map_err(|e| {
            Error::from(format!(
                "Failed to upload converted image {}: {}",
                target_key, e
            ))
        })?;

    Ok(())
}

trait MediaReader: BufRead + Seek + Send {}
impl<R: BufRead + Seek + Send> MediaReader for R {}

fn decode_resize_encode_webp<R: BufRead + Seek>(
    image_data: R,
    source_key: &str,
) -> Result<Vec<u8>, Error> {
    let img = ImageReader::new(image_data)
        .with_guessed_format()
        .map_err(|e| {
            Error::from(format!(
                "Image format detection failed for {}: {}",
                source_key, e
            ))
        })?;

    let decoded_img = img
        .decode()
        .map_err(|e| Error::from(format!("Image decoding failed for {}: {}", source_key, e)))?;

    let resized_img = resize_image(decoded_img);
    encode_as_webp(resized_img)
}

#[tracing::instrument(name = "Resize Image", skip(img))]
fn resize_image(img: image::DynamicImage) -> image::DynamicImage {
    let (width, height) = img.dimensions();
    let filter = image::imageops::FilterType::CatmullRom;

    if width == height {
        // Square image - resize to 1024x1024
        img.resize(1024, 1024, filter)
    } else if width > height {
        img.resize_to_fill(1280, 720, filter)
    } else {
        img.resize(1280, 1280, filter)
    }
}

fn encode_as_webp(img: image::DynamicImage) -> Result<Vec<u8>, Error> {
    let encoded = if img.color().has_alpha() {
        let rgba = img.to_rgba8();
        let encoder = Encoder::from_rgba(&rgba, rgba.width(), rgba.height());
        encoder.encode(WEBP_QUALITY)
    } else {
        let rgb = img.to_rgb8();
        let encoder = Encoder::from_rgb(&rgb, rgb.width(), rgb.height());
        encoder.encode(WEBP_QUALITY)
    };

    Ok(encoded.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_backed_decode_preserves_legacy_landscape_pixels() {
        let image = image::DynamicImage::ImageRgb8(image::ImageBuffer::from_fn(33, 20, |x, y| {
            image::Rgb([(x * 7) as u8, (y * 11) as u8, ((x + y) * 4) as u8])
        }));
        let expected = encode_as_webp(image.resize_to_fill(
            1280,
            720,
            image::imageops::FilterType::CatmullRom,
        ))
        .unwrap();
        let mut encoded = Cursor::new(Vec::new());
        image
            .write_to(&mut encoded, image::ImageFormat::Bmp)
            .unwrap();
        let mut file = tempfile::tempfile().unwrap();
        std::io::Write::write_all(&mut file, &encoded.into_inner()).unwrap();
        std::io::Seek::rewind(&mut file).unwrap();
        let output = decode_resize_encode_webp(BufReader::new(file), "media/test.bmp").unwrap();
        assert_eq!(output, expected);
    }

    #[test]
    fn course_assets_keep_their_original_format() {
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
        assert!(is_supported_image_format("png"));
        assert_eq!(
            generate_webp_key("media/courses/course-1/banner.png").unwrap(),
            "media/courses/course-1/banner.webp"
        );
    }

    #[tokio::test]
    async fn test_key_decoding_1() {
        let key = "media/image+%281%29.jpg";

        let decoded_key = decode(key).unwrap();

        assert_eq!(decoded_key, "media/image (1).jpg");
    }

    #[tokio::test]
    async fn test_key_decoding_2() {
        let key = "media/image%2B1.jpg";

        let decoded_key = decode(key).unwrap();

        assert_eq!(decoded_key, "media/image+1.jpg");
    }

    #[tokio::test]
    async fn test_key_decoding_3() {
        let key = "media/image+%281%29+copy%2B1.jpg";

        let decoded_key = decode(key).unwrap();

        assert_eq!(decoded_key, "media/image (1) copy+1.jpg");
    }

    #[test]
    fn test_supported_image_formats() {
        assert!(is_supported_image_format("jpg"));
        assert!(is_supported_image_format("JPG"));
        assert!(is_supported_image_format("jfif"));
        assert!(is_supported_image_format("tif"));
        assert!(!is_supported_image_format("heic"));
    }

    #[test]
    fn test_generate_webp_key() {
        assert_eq!(
            generate_webp_key("media/folder/image.JPG").unwrap(),
            "media/folder/image.webp"
        );
    }
}

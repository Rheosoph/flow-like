use flow_like_types::{
    base64::{Engine as _, engine::general_purpose::STANDARD},
    images::{MAX_NOTIFICATION_IMAGE_BYTES, notification_thumbnail},
};
use reqwest::Url;
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, SystemTime},
};

const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(8);
const CACHE_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);
const CACHE_MAX_FILES: usize = 32;
static CACHE_LOCK: Mutex<()> = Mutex::new(());

fn remote_image_url(url: &Url) -> bool {
    url.scheme() == "https"
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
}

fn local_image_path(url: &Url) -> Result<PathBuf, String> {
    if url.scheme() == "file" {
        return url
            .to_file_path()
            .map_err(|_| "Invalid image file URL".into());
    }
    let is_asset = (url.scheme() == "asset" && url.host_str() == Some("localhost"))
        || (url.scheme() == "http" && url.host_str() == Some("asset.localhost"));
    if !is_asset || !url.username().is_empty() || url.password().is_some() {
        return Err("Unsupported notification image URL".into());
    }
    let path = urlencoding::decode(url.path().trim_start_matches('/'))
        .map_err(|_| "Invalid image path")?;
    let path = PathBuf::from(path.as_ref());
    if !path.is_absolute() {
        return Err("Notification image path must be absolute".into());
    }
    Ok(path)
}

fn inline_image(source: &str) -> Result<Vec<u8>, String> {
    let (header, data) = source.split_once(',').ok_or("Invalid image data URL")?;
    let media_type = header
        .strip_prefix("data:")
        .and_then(|header| header.strip_suffix(";base64"))
        .ok_or("Notification image data must use base64")?;
    if !matches!(
        media_type,
        "image/png" | "image/jpeg" | "image/gif" | "image/webp"
    ) {
        return Err("Unsupported notification image format".into());
    }
    if data.len() > MAX_NOTIFICATION_IMAGE_BYTES.div_ceil(3) * 4 {
        return Err("Notification image exceeds 5 MiB".into());
    }
    let bytes = STANDARD.decode(data).map_err(|_| "Invalid image base64")?;
    if bytes.len() > MAX_NOTIFICATION_IMAGE_BYTES {
        return Err("Notification image exceeds 5 MiB".into());
    }
    Ok(bytes)
}

fn read_local_image(path: &Path) -> Result<Vec<u8>, String> {
    let metadata = path.metadata().map_err(|_| "Image file is unavailable")?;
    if !metadata.is_file() || metadata.len() > MAX_NOTIFICATION_IMAGE_BYTES as u64 {
        return Err("Notification image must be a regular file of at most 5 MiB".into());
    }
    let file = std::fs::File::open(path).map_err(|_| "Image file is unavailable")?;
    let mut bytes = Vec::new();
    file.take(MAX_NOTIFICATION_IMAGE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Could not read notification image")?;
    if bytes.len() > MAX_NOTIFICATION_IMAGE_BYTES {
        return Err("Notification image exceeds 5 MiB".into());
    }
    Ok(bytes)
}

async fn download_image(url: Url) -> Result<Vec<u8>, String> {
    if !remote_image_url(&url) {
        return Err("Notification images require HTTPS without URL credentials".into());
    }
    let client = reqwest::Client::builder()
        .timeout(DOWNLOAD_TIMEOUT)
        .connect_timeout(Duration::from_secs(3))
        .referer(false)
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 3 || !remote_image_url(attempt.url()) {
                attempt.error("Unsupported notification image redirect")
            } else {
                attempt.follow()
            }
        }))
        .build()
        .map_err(|_| "Could not initialize image download")?;
    let mut response = client
        .get(url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|_| "Notification image download failed")?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_NOTIFICATION_IMAGE_BYTES as u64)
    {
        return Err("Notification image exceeds 5 MiB".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "Notification image download failed")?
    {
        if bytes.len() + chunk.len() > MAX_NOTIFICATION_IMAGE_BYTES {
            return Err("Notification image exceeds 5 MiB".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn clean_cache(directory: &Path, now: SystemTime) -> Result<(), String> {
    let entries = std::fs::read_dir(directory).map_err(|_| "Could not read image cache")?;
    let mut retained = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !entry
            .file_name()
            .to_string_lossy()
            .starts_with("notification-")
            || path.extension().and_then(|ext| ext.to_str()) != Some("png")
        {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        if now.duration_since(modified).unwrap_or_default() > CACHE_MAX_AGE {
            std::fs::remove_file(path).map_err(|_| "Could not clean image cache")?;
        } else {
            retained.push((modified, path));
        }
    }
    retained.sort_by_key(|(modified, _)| *modified);
    // Reserve one slot for the new file. iOS copies attachments into its own store.
    let remove_count = retained.len().saturating_sub(CACHE_MAX_FILES - 1);
    for (_, path) in retained.into_iter().take(remove_count) {
        std::fs::remove_file(path).map_err(|_| "Could not clean image cache")?;
    }
    Ok(())
}

fn cache_image(bytes: &[u8], cache_root: &Path) -> Result<String, String> {
    let png = notification_thumbnail(bytes).map_err(|_| "Invalid notification image")?;
    let _guard = CACHE_LOCK
        .lock()
        .map_err(|_| "Image cache is unavailable")?;
    let directory = cache_root.join("notification-attachments");
    std::fs::create_dir_all(&directory).map_err(|_| "Could not create image cache")?;
    clean_cache(&directory, SystemTime::now())?;
    // Each delivery needs its own file because UserNotifications may move it.
    let path = directory.join(format!("notification-{}.png", uuid::Uuid::new_v4()));
    let mut options = std::fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path).map_err(|_| "Could not cache image")?;
    if file.write_all(&png).is_err() {
        let _ = std::fs::remove_file(&path);
        return Err("Could not cache image".into());
    }
    Url::from_file_path(path)
        .map(|url| url.to_string())
        .map_err(|_| "Invalid cached image path".into())
}

pub(super) async fn prepare(
    source: &str,
    cache_root: PathBuf,
    allow_local_file: impl Fn(&Path) -> bool,
) -> Result<String, String> {
    let source = source.trim();
    let bytes = if source.starts_with("data:") {
        inline_image(source)?
    } else {
        let url = Url::parse(source).map_err(|_| "Invalid notification image URL")?;
        if url.scheme() == "https" {
            download_image(url).await?
        } else {
            let path = local_image_path(&url)?;
            let path = tokio::fs::canonicalize(path)
                .await
                .map_err(|_| "Image file is unavailable")?;
            if !allow_local_file(&path) {
                return Err("Notification image is outside the asset scope".into());
            }
            tokio::task::spawn_blocking(move || read_local_image(&path))
                .await
                .map_err(|_| "Could not read notification image")??
        }
    };
    tokio::task::spawn_blocking(move || cache_image(&bytes, &cache_root))
        .await
        .map_err(|_| "Could not prepare notification image")?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> PathBuf {
        let directory =
            std::env::temp_dir().join(format!("notification-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    #[test]
    fn accepts_only_https_downloads_without_credentials() {
        for value in [
            "http://assets.example/image.png",
            "file:///tmp/image.png",
            "https://user:pass@assets.example/image.png",
        ] {
            assert!(!remote_image_url(&Url::parse(value).unwrap()));
        }
        assert!(remote_image_url(
            &Url::parse("https://assets.example/image?signature=secret").unwrap()
        ));
    }

    #[test]
    fn decodes_only_local_tauri_asset_hosts() {
        for value in [
            "asset://localhost/%2Ftmp%2Ftest%20image.png",
            "http://asset.localhost/%2Ftmp%2Ftest%20image.png",
            "file:///tmp/test%20image.png",
        ] {
            assert_eq!(
                local_image_path(&Url::parse(value).unwrap()).unwrap(),
                PathBuf::from("/tmp/test image.png")
            );
        }
        for value in [
            "http://localhost/private",
            "asset://other/%2Ftmp%2Fa",
            "asset://localhost/relative.png",
        ] {
            assert!(local_image_path(&Url::parse(value).unwrap()).is_err());
        }
    }

    #[test]
    fn rejects_oversized_and_non_image_inline_data_before_caching() {
        assert!(inline_image("data:text/html;base64,eA==").is_err());
        assert!(inline_image("data:image/png,plain").is_err());
        assert!(inline_image("data:image/png;base64,invalid!").is_err());
        let oversized = format!(
            "data:image/png;base64,{}",
            "A".repeat(MAX_NOTIFICATION_IMAGE_BYTES.div_ceil(3) * 4 + 1)
        );
        assert!(inline_image(&oversized).unwrap_err().contains("5 MiB"));
    }

    #[tokio::test]
    async fn refuses_local_files_outside_the_asset_scope() {
        let directory = fixture();
        let path = directory.join("image.png");
        std::fs::write(&path, b"private").unwrap();
        let source = Url::from_file_path(&path).unwrap().to_string();
        let result = prepare(&source, directory.clone(), |_| false).await;
        assert!(result.unwrap_err().contains("asset scope"));
        assert!(!directory.join("notification-attachments").exists());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn rejects_non_regular_and_oversized_local_files() {
        let directory = fixture();
        assert!(read_local_image(&directory).is_err());
        let path = directory.join("oversized.png");
        std::fs::File::create(&path)
            .unwrap()
            .set_len(MAX_NOTIFICATION_IMAGE_BYTES as u64 + 1)
            .unwrap();
        assert!(read_local_image(&path).unwrap_err().contains("5 MiB"));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn cache_evicts_expired_and_excess_files_without_touching_other_files() {
        let directory = fixture();
        for index in 0..CACHE_MAX_FILES + 5 {
            std::fs::write(
                directory.join(format!("notification-{index}.png")),
                b"image",
            )
            .unwrap();
        }
        std::fs::write(directory.join("unrelated.png"), b"keep").unwrap();
        clean_cache(&directory, SystemTime::now()).unwrap();
        assert_eq!(
            std::fs::read_dir(&directory).unwrap().count(),
            CACHE_MAX_FILES
        );
        clean_cache(
            &directory,
            SystemTime::now() + CACHE_MAX_AGE + Duration::from_secs(1),
        )
        .unwrap();
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 1);
        assert!(directory.join("unrelated.png").exists());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[tokio::test]
    async fn creates_distinct_native_png_files_and_rejects_invalid_images() {
        use std::io::Cursor;
        let directory = fixture();
        let mut png = Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(2, 2)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let source = format!(
            "data:image/png;base64,{}",
            STANDARD.encode(png.into_inner())
        );
        let first = prepare(&source, directory.clone(), |_| false)
            .await
            .unwrap();
        let second = prepare(&source, directory.clone(), |_| false)
            .await
            .unwrap();
        assert_ne!(first, second);
        let path = Url::parse(&first).unwrap().to_file_path().unwrap();
        let cached = std::fs::read(path).unwrap();
        assert_eq!(
            image::guess_format(&cached).unwrap(),
            image::ImageFormat::Png
        );
        assert!(
            prepare("data:image/png;base64,eA==", directory.clone(), |_| false)
                .await
                .is_err()
        );
        std::fs::remove_dir_all(directory).unwrap();
    }
}

use std::io::Cursor;

use base64::{Engine as _, engine::general_purpose::STANDARD};

use super::img::resize_image;
pub use flow_like_types_data_url::{
    data_url_to_base64, data_url_to_bytes, make_data_url, pathbuf_to_data_url,
};

/// Transforms the given base64 image to JPEG and optimizes it. Max Size after optimization is 1280 px in any direction.
pub async fn optimize_data_url(url: &str) -> anyhow::Result<String> {
    let data_url = make_data_url(url).await?;
    let img = image::load_from_memory(&STANDARD.decode(data_url_to_base64(&data_url)?)?)?;
    let img = resize_image(&img, 1280).await;
    let img = img.to_rgb8();
    let mut cursor = Cursor::new(Vec::new());
    img.write_to(&mut cursor, image::ImageFormat::Jpeg)?;
    let base64_encoded = STANDARD.encode(cursor.into_inner());
    let new_data_url = format!("data:image/jpeg;base64,{}", base64_encoded);
    Ok(new_data_url)
}

pub async fn image_to_data_url(
    image: &image::DynamicImage,
    format: image::ImageFormat,
) -> anyhow::Result<String> {
    let mut buffer = Cursor::new(Vec::new()); // Use Cursor to wrap the Vec<u8>
    image.write_to(&mut buffer, format)?;
    let base64 = STANDARD.encode(buffer.into_inner());
    let mime = match format {
        image::ImageFormat::Png => "image/png",
        image::ImageFormat::Jpeg => "image/jpeg",
        image::ImageFormat::Gif => "image/gif",
        image::ImageFormat::WebP => "image/webp",
        _ => "application/octet-stream",
    };
    Ok(format!("data:{};base64,{}", mime, base64))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_make_data_url() {
        let url = "https://www.gstatic.com/webp/gallery/1.webp";
        let data_url = make_data_url(url).await.unwrap();
        assert!(data_url.starts_with("data:image/webp;base64,"));
    }

    #[tokio::test]
    async fn test_optimizing_data_url() {
        let url = "https://www.gstatic.com/webp/gallery/1.webp";
        let data_url = make_data_url(url).await.unwrap();
        assert!(data_url.starts_with("data:image/webp;base64,"));
        let optimized_data_url = optimize_data_url(&data_url).await.unwrap();
        assert!(optimized_data_url.starts_with("data:image/jpeg;base64,"));
    }
}

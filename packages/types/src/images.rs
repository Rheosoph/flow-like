pub use image;
use image::GenericImageView;

pub const MAX_NOTIFICATION_IMAGE_BYTES: usize = 5 * 1024 * 1024;

/// Produce a portable notification attachment without retaining a large source image.
pub fn notification_thumbnail(bytes: &[u8]) -> anyhow::Result<Vec<u8>> {
    anyhow::ensure!(
        !bytes.is_empty() && bytes.len() <= MAX_NOTIFICATION_IMAGE_BYTES,
        "Notification images must be between 1 byte and 5 MiB"
    );
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let decoded = reader.decode()?;
    let thumbnail = if decoded.width() <= 256 && decoded.height() <= 256 {
        decoded
    } else {
        decoded.thumbnail(256, 256)
    };
    let mut output = std::io::Cursor::new(Vec::new());
    // Sixteen-bit source images can exceed the API's inline upload limit even
    // at 256px. Normalize color depth while keeping the transparency channel.
    thumbnail
        .to_rgba8()
        .write_to(&mut output, image::ImageFormat::Png)?;
    Ok(output.into_inner())
}

pub fn is_supported_image_format(extension: &str) -> bool {
    matches!(
        extension.to_lowercase().as_str(),
        "jpg" | "jpeg" | "png" | "gif" | "bmp" | "tiff" | "avif" | "heic" | "ico"
    )
}

pub fn resize_image(img: image::DynamicImage) -> image::DynamicImage {
    let (width, height) = img.dimensions();

    if width == height {
        // Square image - resize to 1024x1024
        img.resize(1024, 1024, image::imageops::FilterType::Lanczos3)
    } else if width > height {
        img.resize_to_fill(1280, 720, image::imageops::FilterType::Lanczos3)
    } else {
        img.resize(1280, 1280, image::imageops::FilterType::Lanczos3)
    }
}

pub fn encode_as_webp(img: image::DynamicImage) -> anyhow::Result<Vec<u8>> {
    let mut buffer = Vec::new();

    let encoder = webp::Encoder::from_image(&img)
        .map_err(|e| anyhow::anyhow!("Failed to create WebP encoder: {}", e))?;
    let encoded = encoder.encode_lossless();

    buffer.extend_from_slice(&encoded);
    Ok(buffer)
}

#[cfg(test)]
mod notification_tests {
    use super::*;

    #[test]
    fn notification_image_preserves_aspect_ratio_and_normalizes_to_png() {
        let source = image::DynamicImage::new_rgb8(1024, 512);
        let mut encoded = std::io::Cursor::new(Vec::new());
        source
            .write_to(&mut encoded, image::ImageFormat::Jpeg)
            .unwrap();
        let png = notification_thumbnail(encoded.get_ref()).unwrap();
        assert_eq!(image::guess_format(&png).unwrap(), image::ImageFormat::Png);
        assert_eq!(
            image::load_from_memory(&png).unwrap().dimensions(),
            (256, 128)
        );
    }

    #[test]
    fn notification_image_rejects_invalid_and_oversized_input() {
        assert!(notification_thumbnail(b"not an image").is_err());
        assert!(notification_thumbnail(&vec![0; MAX_NOTIFICATION_IMAGE_BYTES + 1]).is_err());
    }

    #[test]
    fn notification_image_rejects_excessive_dimensions_before_decoding() {
        let source = image::DynamicImage::new_rgb8(8193, 1);
        let mut encoded = std::io::Cursor::new(Vec::new());
        source
            .write_to(&mut encoded, image::ImageFormat::Png)
            .unwrap();
        assert!(notification_thumbnail(encoded.get_ref()).is_err());
    }

    #[test]
    fn notification_image_keeps_transparency_and_small_dimensions() {
        let source = image::DynamicImage::new_rgba8(24, 12);
        let mut encoded = std::io::Cursor::new(Vec::new());
        source
            .write_to(&mut encoded, image::ImageFormat::Png)
            .unwrap();
        let png = notification_thumbnail(encoded.get_ref()).unwrap();
        let decoded = image::load_from_memory(&png).unwrap();
        assert_eq!(decoded.dimensions(), (24, 12));
        assert_eq!(decoded.to_rgba8().get_pixel(0, 0).0[3], 0);
    }

    #[test]
    fn notification_image_normalizes_sixteen_bit_pixels_within_inline_upload_limit() {
        use base64::{Engine as _, engine::general_purpose::STANDARD};

        // Vary every channel so PNG compression cannot hide excess color depth.
        let mut state = 17u64;
        let pixels = (0..256 * 256 * 4)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state as u16
            })
            .collect();
        let source = image::DynamicImage::ImageRgba16(
            image::ImageBuffer::from_raw(256, 256, pixels).unwrap(),
        );
        let mut encoded = std::io::Cursor::new(Vec::new());
        source
            .write_to(&mut encoded, image::ImageFormat::Png)
            .unwrap();

        let png = notification_thumbnail(encoded.get_ref()).unwrap();
        let decoded = image::load_from_memory(&png).unwrap();
        assert_eq!(decoded.color(), image::ColorType::Rgba8);
        assert_eq!(decoded.dimensions(), (256, 256));
        assert!("data:image/png;base64,".len() + STANDARD.encode(png).len() < 512 * 1024);
    }
}

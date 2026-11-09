pub use image;
use image::GenericImageView;

pub const MAX_NOTIFICATION_IMAGE_BYTES: usize = 5 * 1024 * 1024;
/// The longest image data URL an inline upload takes; a 256 px RGBA PNG fits well below it.
pub const MAX_INLINE_IMAGE_LENGTH: usize = 512 * 1024;

/// Re-encodes a PNG, JPEG, GIF or WebP data URL as a PNG of at most 256 px; refuses anything else.
pub fn inline_image_thumbnail(data_url: &str) -> anyhow::Result<Vec<u8>> {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    anyhow::ensure!(
        data_url.len() <= MAX_INLINE_IMAGE_LENGTH,
        "The image data URL has {} bytes; inline images have at most 512 KiB",
        data_url.len()
    );
    let (header, encoded) = data_url
        .split_once(',')
        .ok_or_else(|| anyhow::anyhow!("The image is not a base64 data URL"))?;
    anyhow::ensure!(
        matches!(
            header,
            "data:image/png;base64"
                | "data:image/jpeg;base64"
                | "data:image/gif;base64"
                | "data:image/webp;base64"
        ),
        "The image must be a PNG, JPEG, GIF or WebP data URL, not '{}'",
        header.chars().take(48).collect::<String>()
    );
    notification_thumbnail(&STANDARD.decode(encoded)?)
}

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

/// The smallest per-image limit of the major vision APIs (Claude on Bedrock: 5 MB of base64).
/// An image a host inlines before it knows the provider is fitted into this.
pub const PORTABLE_INLINE_IMAGE_BYTES: usize = 3_750_000;
/// What a host inlines for all images of one message before it knows the provider, so a call
/// through the API's model proxy stays within its 6 MB request limit.
pub const PORTABLE_INLINE_IMAGES_BYTES: usize = 4_000_000;

/// Long edges an image steps down through to fit a byte limit, largest first. The first that
/// fits wins, so text in a screenshot or a photographed page stays as legible as the limit allows.
const FIT_EDGES: [u32; 5] = [4096, 3072, 2560, 2048, 1568];
const FIT_JPEG_QUALITY: u8 = 90;
/// For an image that fits no edge at [`FIT_JPEG_QUALITY`].
const FIT_FALLBACK_JPEG_QUALITY: u8 = 75;

/// An image [`fit_image_within`] re-encoded.
#[derive(Debug)]
pub struct FittedImage {
    pub bytes: Vec<u8>,
    pub media_type: &'static str,
}

/// Re-encodes an image of more than `max_bytes` to fit them, turned upright as its EXIF says.
/// It keeps the full size when re-encoding alone fits and otherwise steps the long edge down
/// through [`FIT_EDGES`]: an image with transparency stays PNG, any other becomes JPEG. `None`
/// when the image already fits; an error when it does not decode or fits at no step.
pub fn fit_image_within(bytes: &[u8], max_bytes: usize) -> anyhow::Result<Option<FittedImage>> {
    fit_within_edges(bytes, max_bytes, &FIT_EDGES)
}

fn fit_within_edges(
    bytes: &[u8],
    max_bytes: usize,
    edges: &[u32],
) -> anyhow::Result<Option<FittedImage>> {
    if bytes.len() <= max_bytes {
        return Ok(None);
    }
    let image = decode_upright(bytes)?;
    let mut edge = image.width().max(image.height());
    let smallest = loop {
        let scaled = scaled_to(&image, edge);
        let fitted = encode_fitted(&scaled)?;
        if fitted.bytes.len() <= max_bytes {
            return Ok(Some(fitted));
        }
        match next_edge(edges, edge, fitted.bytes.len(), max_bytes) {
            Some(next) => edge = next,
            None => break scaled,
        }
    };
    let bytes = encode_jpeg(&smallest, FIT_FALLBACK_JPEG_QUALITY)?;
    anyhow::ensure!(
        bytes.len() <= max_bytes,
        "The image is still {} bytes as a {}x{} JPEG of quality {FIT_FALLBACK_JPEG_QUALITY}, over the {max_bytes}-byte limit",
        bytes.len(),
        smallest.width(),
        smallest.height()
    );
    Ok(Some(FittedImage {
        bytes,
        media_type: "image/jpeg",
    }))
}

/// The next edge below `edge` worth encoding. The encoded size grows about with the pixel count,
/// so edges that `encoded` bytes at `edge` say cannot fit are skipped, down to the last one.
fn next_edge(edges: &[u32], edge: u32, encoded: usize, max_bytes: usize) -> Option<u32> {
    let reachable = edge as f64 * (max_bytes as f64 / encoded as f64).sqrt() * 1.1;
    let below: Vec<u32> = edges.iter().copied().filter(|step| *step < edge).collect();
    below
        .iter()
        .copied()
        .find(|step| *step as f64 <= reachable)
        .or_else(|| below.last().copied())
}

fn decode_upright(bytes: &[u8]) -> anyhow::Result<image::DynamicImage> {
    use image::ImageDecoder;
    let mut decoder = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()?
        .into_decoder()?;
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut image = image::DynamicImage::from_decoder(decoder)?;
    image.apply_orientation(orientation);
    Ok(image)
}

fn scaled_to(image: &image::DynamicImage, edge: u32) -> image::DynamicImage {
    if image.width().max(image.height()) <= edge {
        return image.clone();
    }
    image.resize(edge, edge, image::imageops::FilterType::CatmullRom)
}

fn encode_fitted(image: &image::DynamicImage) -> anyhow::Result<FittedImage> {
    if image.color().has_alpha() {
        let mut output = std::io::Cursor::new(Vec::new());
        image
            .to_rgba8()
            .write_to(&mut output, image::ImageFormat::Png)?;
        return Ok(FittedImage {
            bytes: output.into_inner(),
            media_type: "image/png",
        });
    }
    Ok(FittedImage {
        bytes: encode_jpeg(image, FIT_JPEG_QUALITY)?,
        media_type: "image/jpeg",
    })
}

/// JPEG of `image`, with any transparency laid over white.
fn encode_jpeg(image: &image::DynamicImage, quality: u8) -> anyhow::Result<Vec<u8>> {
    let rgb = if image.color().has_alpha() {
        let rgba = image.to_rgba8();
        image::RgbImage::from_fn(rgba.width(), rgba.height(), |x, y| {
            let [r, g, b, a] = rgba.get_pixel(x, y).0;
            let over_white =
                |channel: u8| ((channel as u16 * a as u16 + 255 * (255 - a as u16)) / 255) as u8;
            image::Rgb([over_white(r), over_white(g), over_white(b)])
        })
    } else {
        image.to_rgb8()
    };
    let mut output = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut output, quality).encode_image(&rgb)?;
    Ok(output)
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

#[cfg(test)]
mod fit_tests {
    use super::*;

    /// A photo-like image: smooth gradients with fine detail, so JPEG neither collapses nor
    /// explodes it.
    fn photo(width: u32, height: u32) -> image::DynamicImage {
        image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(width, height, |x, y| {
            let detail = ((x * 7 + y * 13) % 17) as u8;
            image::Rgb([
                (x * 255 / width) as u8 ^ detail,
                (y * 255 / height) as u8,
                ((x + y) % 256) as u8 ^ (detail << 2),
            ])
        }))
    }

    fn png(image: &image::DynamicImage) -> Vec<u8> {
        let mut output = std::io::Cursor::new(Vec::new());
        image
            .write_to(&mut output, image::ImageFormat::Png)
            .unwrap();
        output.into_inner()
    }

    const EDGES: [u32; 3] = [400, 300, 200];

    #[test]
    fn an_image_within_the_limit_is_left_as_it_is() {
        let bytes = png(&photo(64, 48));
        assert!(fit_image_within(&bytes, bytes.len()).unwrap().is_none());
    }

    #[test]
    fn a_photo_keeps_its_size_when_reencoding_fits() {
        let bytes = png(&photo(300, 200));
        let fitted = fit_within_edges(&bytes, bytes.len() - 1, &EDGES)
            .unwrap()
            .unwrap();
        assert_eq!(fitted.media_type, "image/jpeg");
        let decoded = image::load_from_memory(&fitted.bytes).unwrap();
        assert_eq!(decoded.dimensions(), (300, 200));
    }

    #[test]
    fn a_photo_steps_down_its_long_edge_until_it_fits() {
        let source = photo(600, 300);
        let limit = encode_jpeg(&source, FIT_JPEG_QUALITY).unwrap().len() / 3;
        let fitted = fit_within_edges(&png(&source), limit, &EDGES)
            .unwrap()
            .unwrap();
        assert!(fitted.bytes.len() <= limit);
        let decoded = image::load_from_memory(&fitted.bytes).unwrap();
        assert!(EDGES.contains(&decoded.width()), "{}", decoded.width());
        assert_eq!(decoded.width(), decoded.height() * 2);
    }

    #[test]
    fn transparency_stays_png_while_it_fits() {
        let source =
            image::DynamicImage::ImageRgba8(image::RgbaImage::from_fn(600, 400, |x, y| {
                image::Rgba([
                    (x % 256) as u8,
                    (y % 256) as u8,
                    ((x * y) % 256) as u8,
                    (x % 200) as u8,
                ])
            }));
        let bytes = png(&source);
        let fitted = fit_within_edges(&bytes, bytes.len() * 3 / 4, &EDGES)
            .unwrap()
            .unwrap();
        assert_eq!(fitted.media_type, "image/png");
        assert!(fitted.bytes.len() <= bytes.len() * 3 / 4);
    }

    #[test]
    fn skipped_edges_are_those_the_size_rules_out() {
        assert_eq!(next_edge(&EDGES, 600, 100, 90), Some(400));
        assert_eq!(next_edge(&EDGES, 600, 900, 100), Some(200));
        assert_eq!(next_edge(&EDGES, 600, 9_000, 100), Some(200));
        assert_eq!(next_edge(&EDGES, 200, 900, 100), None);
    }

    #[test]
    fn what_cannot_be_decoded_or_fitted_fails() {
        assert!(fit_image_within(&[0; 64], 32).is_err());
        assert!(fit_within_edges(&png(&photo(600, 450)), 1_000, &EDGES).is_err());
    }
}

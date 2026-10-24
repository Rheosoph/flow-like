use super::file_probe::{FilePrefix, PROBE_PREFIX_BYTES};
use flow_like_types::{
    JsonSchema, anyhow,
    image::{ImageDecoder, ImageError, ImageFormat, ImageReader},
    json::{Deserialize, Serialize},
};
use std::{fmt, io::Cursor};

const PREFIX_GROWTH: u64 = 4;

/// What an image file's header says, read without decoding a single pixel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ImageHeader {
    /// Width in pixels; the upright width when the EXIF orientation was applied.
    pub width: u32,
    /// Height in pixels; the upright height when the EXIF orientation was applied.
    pub height: u32,
    /// Lowercase format name such as `jpeg`, `png`, `webp` or `tiff`.
    pub format: String,
    /// EXIF orientation tag, 1–8; 1 when the file carries none.
    pub orientation: u8,
}

#[derive(Debug)]
pub enum ImageHeaderError {
    NotAnImage,
    Unsupported {
        format: ImageFormat,
        error: ImageError,
    },
    /// The header could not be parsed from these bytes; more of the file may complete it.
    Incomplete {
        format: ImageFormat,
        error: ImageError,
    },
}

impl fmt::Display for ImageHeaderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAnImage => write!(
                f,
                "its leading bytes match no supported image format (PNG, JPEG, GIF, WebP, TIFF, BMP, ICO, QOI, PNM, HDR, OpenEXR, DDS, farbfeld)"
            ),
            Self::Unsupported { format, error } => {
                write!(f, "{} images cannot be read: {error}", format_name(*format))
            }
            Self::Incomplete { format, error } => {
                write!(f, "the {} header is invalid: {error}", format_name(*format))
            }
        }
    }
}

impl std::error::Error for ImageHeaderError {}

fn format_name(format: ImageFormat) -> String {
    format!("{format:?}").to_ascii_lowercase()
}

/// Reads dimensions, format and EXIF orientation from the leading `bytes` of an image file.
/// With `apply_exif_orientation`, width and height are swapped for orientations 5–8 so they
/// match the upright image `Read Image` produces.
pub fn parse_image_header(
    bytes: &[u8],
    apply_exif_orientation: bool,
) -> Result<ImageHeader, ImageHeaderError> {
    let reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| ImageHeaderError::NotAnImage)?;
    let format = reader.format().ok_or(ImageHeaderError::NotAnImage)?;
    let classify = |error: ImageError| match error {
        ImageError::Decoding(_) | ImageError::IoError(_) => {
            ImageHeaderError::Incomplete { format, error }
        }
        error => ImageHeaderError::Unsupported { format, error },
    };

    let mut decoder = reader.into_decoder().map_err(classify)?;
    let (width, height) = decoder.dimensions();
    let orientation = decoder.orientation().map_err(classify)?.to_exif();
    let (width, height) = if apply_exif_orientation && (5..=8).contains(&orientation) {
        (height, width)
    } else {
        (width, height)
    };

    Ok(ImageHeader {
        width,
        height,
        format: format_name(format),
        orientation,
    })
}

/// Parses the image header of `file`, fetching 64 KiB first and four times as much on every
/// retry until the header is complete — EXIF and other APP segments can push a JPEG's frame
/// header far past the first read. A non-image fails after the first read.
pub async fn read_image_header(
    file: &mut FilePrefix,
    apply_exif_orientation: bool,
) -> flow_like_types::Result<ImageHeader> {
    let mut len = PROBE_PREFIX_BYTES;
    loop {
        file.fill_to(len).await?;
        match parse_image_header(file.bytes(), apply_exif_orientation) {
            Ok(header) => return Ok(header),
            Err(ImageHeaderError::Incomplete { .. }) if !file.is_complete() => {
                len = len.saturating_mul(PREFIX_GROWTH);
            }
            Err(error) => {
                return Err(anyhow!(
                    "'{}' is not a readable image after reading {} of its {} bytes: {error}",
                    file.location(),
                    file.bytes().len(),
                    file.meta().size
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::file_probe::tests::stored;
    use flow_like_types::image::{
        ExtendedColorType, ImageEncoder, Rgb, RgbImage,
        codecs::{jpeg::JpegEncoder, png::PngEncoder},
    };

    /// A little-endian TIFF block whose only entry is Orientation 6: the stored frame
    /// has to be turned 90 degrees clockwise to be upright.
    const ROTATE_90_EXIF: [u8; 26] = [
        0x49, 0x49, 0x2a, 0x00, 0x08, 0x00, 0x00, 0x00, 0x01, 0x00, 0x12, 0x01, 0x03, 0x00, 0x01,
        0x00, 0x00, 0x00, 0x06, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];

    fn frame() -> RgbImage {
        RgbImage::from_pixel(32, 16, Rgb([200, 40, 40]))
    }

    fn png() -> Vec<u8> {
        let mut bytes = Vec::new();
        PngEncoder::new(&mut bytes)
            .write_image(frame().as_raw(), 32, 16, ExtendedColorType::Rgb8)
            .unwrap();
        bytes
    }

    fn rotated_jpeg() -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut encoder = JpegEncoder::new(&mut bytes);
        encoder.set_exif_metadata(ROTATE_90_EXIF.to_vec()).unwrap();
        encoder
            .write_image(frame().as_raw(), 32, 16, ExtendedColorType::Rgb8)
            .unwrap();
        bytes
    }

    /// Inserts comment segments right after SOI so the frame header lands past `padding`
    /// bytes, then appends `trailing` bytes after EOI — data a header read must never touch.
    fn padded_jpeg(padding: usize, trailing: usize) -> Vec<u8> {
        let jpeg = rotated_jpeg();
        let mut bytes = jpeg[..2].to_vec();
        let mut left = padding;
        while left > 0 {
            let chunk = left.min(60_000);
            bytes.extend_from_slice(&[0xFF, 0xFE]);
            bytes.extend_from_slice(&((chunk + 2) as u16).to_be_bytes());
            bytes.extend(std::iter::repeat_n(b'x', chunk));
            left -= chunk;
        }
        bytes.extend_from_slice(&jpeg[2..]);
        bytes.extend(std::iter::repeat_n(0, trailing));
        bytes
    }

    #[test]
    fn png_header_reports_size_and_format() {
        let header = parse_image_header(&png(), true).unwrap();
        assert_eq!(
            header,
            ImageHeader {
                width: 32,
                height: 16,
                format: "png".into(),
                orientation: 1,
            }
        );
    }

    #[test]
    fn jpeg_exif_orientation_swaps_axes_only_when_applied() {
        let bytes = rotated_jpeg();
        let upright = parse_image_header(&bytes, true).unwrap();
        assert_eq!((upright.width, upright.height), (16, 32));
        assert_eq!(upright.orientation, 6);
        assert_eq!(upright.format, "jpeg");
        let stored = parse_image_header(&bytes, false).unwrap();
        assert_eq!((stored.width, stored.height), (32, 16));
        assert_eq!(stored.orientation, 6);
    }

    #[test]
    fn a_header_cut_short_asks_for_more_bytes() {
        let bytes = padded_jpeg(100_000, 0);
        assert!(matches!(
            parse_image_header(&bytes[..64 * 1024], true),
            Err(ImageHeaderError::Incomplete { .. })
        ));
        assert!(matches!(
            parse_image_header(b"plain text, not pixels", true),
            Err(ImageHeaderError::NotAnImage)
        ));
    }

    #[tokio::test]
    async fn the_prefix_grows_until_the_frame_header_is_reached() {
        let bytes = padded_jpeg(100_000, 2 * 1024 * 1024);
        let size = bytes.len();
        let mut file = stored(bytes).await;
        let header = read_image_header(&mut file, true).await.unwrap();
        assert_eq!(
            (header.width, header.height, header.orientation),
            (16, 32, 6)
        );
        assert_eq!(file.bytes().len(), 256 * 1024);
        assert!(file.bytes().len() < size);
    }

    #[tokio::test]
    async fn a_small_image_is_read_in_one_request() {
        let bytes = png();
        let size = bytes.len();
        let mut file = stored(bytes).await;
        read_image_header(&mut file, true).await.unwrap();
        assert_eq!(file.bytes().len(), size);
    }

    #[tokio::test]
    async fn a_non_image_fails_after_the_first_read() {
        let mut file = stored(vec![b'a'; 1024 * 1024]).await;
        let error = read_image_header(&mut file, true).await.unwrap_err();
        assert!(
            error.to_string().contains("no supported image format"),
            "{error}"
        );
        assert_eq!(file.bytes().len() as u64, PROBE_PREFIX_BYTES);
    }
}

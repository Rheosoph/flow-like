use crate::computer::ocr::{OcrLine, OcrOptions, OcrWord, PixelBox, match_language};
use ::windows::{
    Globalization::Language,
    Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap},
    Media::Ocr::OcrEngine,
    Storage::Streams::DataWriter,
    core::HSTRING,
};
use flow_like_types::{Result, anyhow};

pub async fn recognize(image: image::RgbaImage, options: OcrOptions) -> Result<Vec<OcrLine>> {
    tokio::task::spawn_blocking(move || recognize_blocking(image, &options)).await?
}

const LANGUAGE_HINT: &str = "add the language with its optical character recognition feature in Settings > Time & language > Language & region, or run `Add-WindowsCapability -Online -Name \"Language.OCR~~~<tag>~0.0.1.0\"` in an elevated PowerShell";

fn engine(languages: &[String]) -> Result<OcrEngine> {
    if languages.is_empty() {
        return OcrEngine::TryCreateFromUserProfileLanguages().map_err(|e| {
            anyhow!(
                "Windows OCR has no recognizer for the user's languages ({}); {}",
                e.message(),
                LANGUAGE_HINT
            )
        });
    }
    let available: Vec<Language> = OcrEngine::AvailableRecognizerLanguages()
        .map(|languages| languages.into_iter().collect())
        .unwrap_or_default();
    let tags: Vec<String> = available
        .iter()
        .map(|language| {
            language
                .LanguageTag()
                .map(|tag| tag.to_string_lossy())
                .unwrap_or_default()
        })
        .collect();
    for requested in languages {
        let Some(tag) = match_language(requested, &tags) else {
            continue;
        };
        let language = Language::CreateLanguage(&HSTRING::from(tag.as_str()))
            .map_err(|e| anyhow!("Windows rejected language tag {}: {}", tag, e.message()))?;
        return OcrEngine::TryCreateFromLanguage(&language)
            .map_err(|e| anyhow!("Windows OCR cannot use {}: {}", tag, e.message()));
    }
    Err(anyhow!(
        "Windows OCR has no recognizer for {} (installed: {}); {}",
        languages.join(", "),
        if tags.is_empty() {
            "none".to_string()
        } else {
            tags.join(", ")
        },
        LANGUAGE_HINT
    ))
}

fn recognize_blocking(image: image::RgbaImage, options: &OcrOptions) -> Result<Vec<OcrLine>> {
    let engine = engine(&options.languages)?;
    let (width, height) = image.dimensions();
    let limit = OcrEngine::MaxImageDimension().unwrap_or(2600).max(1);
    let scale = (limit as f64 / width.max(height) as f64).min(1.0);
    let image = if scale < 1.0 {
        image::imageops::resize(
            &image,
            ((width as f64 * scale).floor() as u32).max(1),
            ((height as f64 * scale).floor() as u32).max(1),
            image::imageops::FilterType::Triangle,
        )
    } else {
        image
    };
    let (scaled_width, scaled_height) = image.dimensions();
    let mut bgra = image.into_raw();
    for pixel in bgra.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let writer = DataWriter::new()?;
    writer.WriteBytes(&bgra)?;
    let buffer = writer.DetachBuffer()?;
    let bitmap = SoftwareBitmap::CreateCopyFromBuffer(
        &buffer,
        BitmapPixelFormat::Bgra8,
        scaled_width as i32,
        scaled_height as i32,
    )?;
    let result = engine
        .RecognizeAsync(&bitmap)?
        .get()
        .map_err(|e| anyhow!("Windows OCR failed: {}", e.message()))?;

    let factor = 1.0 / scale;
    let mut lines = Vec::new();
    for line in result.Lines()? {
        let mut words = Vec::new();
        for word in line.Words()? {
            let rect = word.BoundingRect()?;
            let Some(bbox_px) = to_pixels(
                rect.X as f64 * factor,
                rect.Y as f64 * factor,
                rect.Width as f64 * factor,
                rect.Height as f64 * factor,
                width,
                height,
            ) else {
                continue;
            };
            words.push(OcrWord {
                text: word.Text()?.to_string_lossy(),
                confidence: None,
                bbox_px,
                bbox: None,
            });
        }
        let Some(first) = words.first() else {
            continue;
        };
        let bbox_px = words[1..]
            .iter()
            .fold(first.bbox_px, |acc, w| acc.union(&w.bbox_px));
        lines.push(OcrLine {
            text: line.Text()?.to_string_lossy(),
            confidence: None,
            bbox_px,
            bbox: None,
            center: None,
            words,
        });
    }
    Ok(lines)
}

fn to_pixels(x: f64, y: f64, w: f64, h: f64, width: u32, height: u32) -> Option<PixelBox> {
    if ![x, y, w, h].iter().all(|v| v.is_finite()) {
        return None;
    }
    let x0 = x.floor().clamp(0.0, width as f64);
    let y0 = y.floor().clamp(0.0, height as f64);
    let x1 = (x + w).ceil().clamp(0.0, width as f64);
    let y1 = (y + h).ceil().clamp(0.0, height as f64);
    (x1 > x0 && y1 > y0).then(|| PixelBox {
        x: x0 as u32,
        y: y0 as u32,
        width: (x1 - x0) as u32,
        height: (y1 - y0) as u32,
    })
}

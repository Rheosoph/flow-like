use crate::computer::ocr::{
    OcrLine, OcrOptions, OcrWord, PixelBox, finite_confidence, match_language,
    normalized_bottom_left_to_pixels, proportional_word_box, word_spans,
};
use flow_like_types::{Result, anyhow};
use objc2::{AnyThread, msg_send, rc::Retained, runtime::AnyObject, sel};
use objc2_core_foundation::{CFData, CFRetained};
use objc2_core_graphics::{
    CGBitmapInfo, CGColorRenderingIntent, CGColorSpace, CGDataProvider, CGImage, CGImageAlphaInfo,
};
use objc2_foundation::{NSArray, NSDictionary, NSError, NSObjectProtocol, NSRange, NSString};
use objc2_vision::{
    VNImageRequestHandler, VNRecognizeTextRequest, VNRecognizedText, VNRectangleObservation,
    VNRequest, VNRequestTextRecognitionLevel,
};

pub async fn recognize(image: image::RgbaImage, options: OcrOptions) -> Result<Vec<OcrLine>> {
    tokio::task::spawn_blocking(move || {
        let picture = cg_image(&image)?;
        objc2::rc::autoreleasepool(|_| recognize_image(&picture, image.dimensions(), &options))
    })
    .await?
}

/// Wraps the RGBA buffer as a CGImage; no per-pixel conversion or encoding.
fn cg_image(image: &image::RgbaImage) -> Result<CFRetained<CGImage>> {
    let (width, height) = (image.width() as usize, image.height() as usize);
    let failed = || anyhow!("Cannot hand a {}x{} image to Apple Vision", width, height);
    let data = CFData::from_bytes(image.as_raw());
    let provider = CGDataProvider::with_cf_data(Some(&data)).ok_or_else(failed)?;
    let space = CGColorSpace::new_device_rgb().ok_or_else(failed)?;
    unsafe {
        CGImage::new(
            width,
            height,
            8,
            32,
            width * 4,
            Some(&space),
            CGBitmapInfo(CGImageAlphaInfo::NoneSkipLast.0),
            Some(&provider),
            std::ptr::null(),
            false,
            CGColorRenderingIntent::RenderingIntentDefault,
        )
    }
    .ok_or_else(failed)
}

fn configure_languages(request: &VNRecognizeTextRequest, languages: &[String]) -> Result<()> {
    if languages.is_empty() {
        if request.respondsToSelector(sel!(setAutomaticallyDetectsLanguage:)) {
            request.setAutomaticallyDetectsLanguage(true);
        }
        return Ok(());
    }
    let supported: Vec<String> = unsafe { request.supportedRecognitionLanguagesAndReturnError() }
        .map(|tags| tags.iter().map(|tag| tag.to_string()).collect())
        .unwrap_or_default();
    let mut tags = Vec::with_capacity(languages.len());
    for language in languages {
        if supported.is_empty() {
            tags.push(language.clone());
            continue;
        }
        let tag = match_language(language, &supported).ok_or_else(|| {
            anyhow!(
                "Apple Vision cannot recognize language '{}'; supported: {}",
                language,
                supported.join(", ")
            )
        })?;
        if !tags.contains(&tag) {
            tags.push(tag);
        }
    }
    let tags: Vec<Retained<NSString>> = tags.iter().map(|tag| NSString::from_str(tag)).collect();
    request.setRecognitionLanguages(&NSArray::from_retained_slice(&tags));
    Ok(())
}

fn recognize_image(
    picture: &CGImage,
    size: (u32, u32),
    options: &OcrOptions,
) -> Result<Vec<OcrLine>> {
    let (width, height) = size;
    let handler_options = NSDictionary::<NSString, AnyObject>::new();
    let handler = unsafe {
        VNImageRequestHandler::initWithCGImage_options(
            VNImageRequestHandler::alloc(),
            picture,
            &handler_options,
        )
    };
    let request = VNRecognizeTextRequest::new();
    request.setRecognitionLevel(VNRequestTextRecognitionLevel::Accurate);
    request.setUsesLanguageCorrection(options.language_correction);
    configure_languages(&request, &options.languages)?;
    let base: &VNRequest = &request;
    handler
        .performRequests_error(&NSArray::from_slice(&[base]))
        .map_err(|error| {
            anyhow!(
                "Apple Vision text recognition failed: {}",
                error.localizedDescription()
            )
        })?;

    let Some(observations) = request.results() else {
        return Ok(Vec::new());
    };
    let mut lines = Vec::new();
    for observation in observations.iter() {
        let Some(candidate) = observation.topCandidates(1).firstObject() else {
            continue;
        };
        let rect = unsafe { observation.boundingBox() };
        let Some(bbox_px) = normalized_bottom_left_to_pixels(
            rect.origin.x,
            rect.origin.y,
            rect.size.width,
            rect.size.height,
            width,
            height,
        ) else {
            continue;
        };
        let text = candidate.string().to_string();
        let confidence = finite_confidence(candidate.confidence() as f64);
        let words = words(&candidate, &text, confidence, &bbox_px, size);
        lines.push(OcrLine {
            text,
            confidence,
            bbox_px,
            bbox: None,
            center: None,
            words,
        });
    }
    Ok(lines)
}

/// Word boxes from Vision's per-range bounding boxes, or a proportional slice of the line box
/// when Vision cannot box a range.
fn words(
    candidate: &VNRecognizedText,
    text: &str,
    confidence: Option<f64>,
    line: &PixelBox,
    size: (u32, u32),
) -> Vec<OcrWord> {
    let total_chars = text.chars().count();
    word_spans(text)
        .into_iter()
        .map(|(word, utf16_start, utf16_len, char_start, char_len)| {
            let range = NSRange::new(utf16_start, utf16_len);
            let observation: std::result::Result<
                Retained<VNRectangleObservation>,
                Retained<NSError>,
            > = unsafe { msg_send![candidate, boundingBoxForRange: range, error: _] };
            let bbox_px = observation
                .ok()
                .and_then(|observation| {
                    let rect = unsafe { observation.boundingBox() };
                    normalized_bottom_left_to_pixels(
                        rect.origin.x,
                        rect.origin.y,
                        rect.size.width,
                        rect.size.height,
                        size.0,
                        size.1,
                    )
                })
                .unwrap_or_else(|| proportional_word_box(line, char_start, char_len, total_chars));
            OcrWord {
                text: word.to_string(),
                confidence,
                bbox_px,
                bbox: None,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Renders digits with the set-of-marks bitmap font and reads them back through Vision.
    /// Run manually: `cargo test -p flow-like-catalog-automation --features execute --lib
    /// vision_reads_rendered_digits -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn vision_reads_rendered_digits() {
        use crate::computer::capture_state::draw_number;
        let mut canvas = image::RgbaImage::from_pixel(1200, 400, image::Rgba([255, 255, 255, 255]));
        draw_number(&mut canvas, 40, 60, 2468, 8, [0, 0, 0]);
        draw_number(&mut canvas, 520, 60, 1357, 8, [0, 0, 0]);
        draw_number(&mut canvas, 40, 260, 90, 6, [0, 0, 0]);
        let started = std::time::Instant::now();
        let lines = recognize(canvas, OcrOptions::default()).await.unwrap();
        println!("{:?} {lines:#?}", started.elapsed());
        let first = lines
            .iter()
            .find(|line| line.text.replace(' ', "").contains("2468"))
            .expect("2468 recognized");
        assert!(first.bbox_px.y.abs_diff(60) < 20 && first.bbox_px.x.abs_diff(40) < 20);
        assert!(lines.iter().any(|line| line.text.contains("90")));
        let bottom = lines.iter().find(|line| line.text.contains("90")).unwrap();
        assert!(bottom.bbox_px.y > first.bbox_px.y + 100);
    }
}

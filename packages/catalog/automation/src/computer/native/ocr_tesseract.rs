use crate::computer::ocr::{OcrLine, OcrWord, PixelBox};

/// Parses `tesseract … tsv` output into lines of words. Coordinates are divided by `scale`
/// (the factor the image was enlarged by before recognition) and clamped to the original image.
pub(crate) fn parse_tsv(tsv: &str, scale: f64, width: u32, height: u32) -> Vec<OcrLine> {
    let mut lines: Vec<((u32, u32, u32, u32), OcrLine)> = Vec::new();
    for row in tsv.lines().skip(1) {
        let columns: Vec<&str> = row.split('\t').collect();
        if columns.len() < 12 || columns[0] != "5" {
            continue;
        }
        let number = |index: usize| columns[index].trim().parse::<f64>().ok();
        let text = columns[11..].join("\t").trim().to_string();
        let (Some(conf), Some(left), Some(top), Some(w), Some(h)) =
            (number(10), number(6), number(7), number(8), number(9))
        else {
            continue;
        };
        if text.is_empty() || conf < 0.0 || w <= 0.0 || h <= 0.0 {
            continue;
        }
        let key = |index: usize| columns[index].trim().parse::<u32>().unwrap_or(0);
        let key = (key(1), key(2), key(3), key(4));
        let Some(bbox_px) = scaled_box(left, top, w, h, scale, width, height) else {
            continue;
        };
        let word = OcrWord {
            text,
            confidence: Some((conf / 100.0).clamp(0.0, 1.0)),
            bbox_px,
            bbox: None,
        };
        match lines.last_mut() {
            Some((last, line)) if *last == key => line.words.push(word),
            _ => lines.push((
                key,
                OcrLine {
                    text: String::new(),
                    confidence: None,
                    bbox_px,
                    bbox: None,
                    center: None,
                    words: vec![word],
                },
            )),
        }
    }
    lines
        .into_iter()
        .map(|(_, mut line)| {
            line.text = line
                .words
                .iter()
                .map(|w| w.text.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            line.bbox_px = line.words[1..]
                .iter()
                .fold(line.words[0].bbox_px, |acc, w| acc.union(&w.bbox_px));
            let total: f64 = line.words.iter().filter_map(|w| w.confidence).sum();
            line.confidence = Some(total / line.words.len() as f64);
            line
        })
        .collect()
}

fn scaled_box(
    left: f64,
    top: f64,
    width: f64,
    height: f64,
    scale: f64,
    image_width: u32,
    image_height: u32,
) -> Option<PixelBox> {
    let x0 = (left / scale).floor().clamp(0.0, image_width as f64);
    let y0 = (top / scale).floor().clamp(0.0, image_height as f64);
    let x1 = ((left + width) / scale)
        .ceil()
        .clamp(0.0, image_width as f64);
    let y1 = ((top + height) / scale)
        .ceil()
        .clamp(0.0, image_height as f64);
    (x1 > x0 && y1 > y0).then(|| PixelBox {
        x: x0 as u32,
        y: y0 as u32,
        width: (x1 - x0) as u32,
        height: (y1 - y0) as u32,
    })
}

/// Tesseract reads screen-sized text poorly; small captures are enlarged 2x first.
pub(crate) fn upscale_factor(width: u32, height: u32) -> u32 {
    if width.max(height) <= 2000 { 2 } else { 1 }
}

#[cfg(target_os = "linux")]
pub async fn recognize(
    image: image::RgbaImage,
    options: crate::computer::ocr::OcrOptions,
) -> flow_like_types::Result<Vec<OcrLine>> {
    use flow_like_types::anyhow;
    use tokio::io::AsyncWriteExt;

    let (width, height) = image.dimensions();
    let factor = upscale_factor(width, height);
    let png = tokio::task::spawn_blocking(move || -> flow_like_types::Result<Vec<u8>> {
        let image = if factor > 1 {
            image::imageops::resize(
                &image,
                width * factor,
                height * factor,
                image::imageops::FilterType::CatmullRom,
            )
        } else {
            image
        };
        let mut png = Vec::new();
        image::DynamicImage::ImageRgba8(image)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .map_err(|e| anyhow!("Cannot encode image for Tesseract: {}", e))?;
        Ok(png)
    })
    .await??;

    let languages: Vec<String> = options
        .languages
        .iter()
        .map(|language| crate::computer::ocr::tesseract_language(language))
        .collect();
    let mut command = tokio::process::Command::new("tesseract");
    command.args(["stdin", "stdout", "--psm", "11"]);
    if !languages.is_empty() {
        command.args(["-l", &languages.join("+")]);
    }
    command
        .arg("tsv")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let mut child = command.spawn().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            anyhow!(
                "Text recognition on Linux needs Tesseract: install it (e.g. `sudo apt install tesseract-ocr`, `sudo dnf install tesseract` or `sudo pacman -S tesseract tesseract-data-eng`) and make sure `tesseract` is on PATH"
            )
        } else {
            anyhow!("Cannot start tesseract: {}", error)
        }
    })?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("Cannot write the image to tesseract"))?;
    let writer = tokio::spawn(async move {
        let result = stdin.write_all(&png).await;
        drop(stdin);
        result
    });
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(120),
        child.wait_with_output(),
    )
    .await
    .map_err(|_| anyhow!("Tesseract did not finish within 120 seconds"))??;
    writer
        .await?
        .map_err(|e| anyhow!("Cannot write the image to tesseract: {}", e))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let hint = if stderr.contains("Failed loading language")
            || stderr.contains("Error opening data file")
        {
            format!(
                "; install the language data (e.g. `sudo apt install {}`)",
                languages
                    .iter()
                    .map(|language| format!("tesseract-ocr-{}", language.replace('_', "-")))
                    .collect::<Vec<_>>()
                    .join(" ")
            )
        } else {
            String::new()
        };
        return Err(anyhow!(
            "Tesseract failed ({}): {}{}",
            output.status,
            stderr.trim(),
            hint
        ));
    }
    Ok(parse_tsv(
        &String::from_utf8_lossy(&output.stdout),
        factor as f64,
        width,
        height,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TSV: &str = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext
1\t1\t0\t0\t0\t0\t0\t0\t800\t600\t-1\t
2\t1\t1\t0\t0\t0\t20\t40\t300\t30\t-1\t
4\t1\t1\t1\t1\t0\t20\t40\t300\t30\t-1\t
5\t1\t1\t1\t1\t1\t20\t40\t100\t30\t96.5\tSave
5\t1\t1\t1\t1\t2\t140\t42\t180\t28\t88.0\tchanges?
5\t1\t1\t1\t1\t3\t330\t40\t10\t30\t-1\t
5\t1\t2\t1\t1\t1\t400\t500\t90\t24\t71\tCancel
5\t1\t2\t1\t1\t2\t500\t500\t0\t24\t71\tbroken
";

    #[test]
    fn tsv_groups_words_into_lines_and_scales_back() {
        let lines = parse_tsv(TSV, 2.0, 400, 300);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "Save changes?");
        assert_eq!(
            lines[0].bbox_px,
            PixelBox {
                x: 10,
                y: 20,
                width: 150,
                height: 15
            }
        );
        assert_eq!(lines[0].words[1].bbox_px.x, 70);
        let confidence = lines[0].confidence.unwrap();
        assert!((confidence - 0.9225).abs() < 1e-9);
        assert_eq!(lines[1].text, "Cancel");
        assert_eq!(
            lines[1].bbox_px,
            PixelBox {
                x: 200,
                y: 250,
                width: 45,
                height: 12
            }
        );
        assert!(parse_tsv("", 1.0, 10, 10).is_empty());
        assert!(parse_tsv("header\n5\t1\tbad", 1.0, 10, 10).is_empty());
    }

    #[test]
    fn small_captures_are_enlarged() {
        assert_eq!(upscale_factor(1440, 900), 2);
        assert_eq!(upscale_factor(2880, 1800), 1);
    }
}

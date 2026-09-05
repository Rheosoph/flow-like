//! Data URL transport and byte conversion without image decoders or Flow runtime types.

use base64::{Engine as _, engine::general_purpose::STANDARD};

/// If you input a valid Data URL, it will return the same URL.
/// Otherwise it will try to download the image and return a Data URL.
pub async fn make_data_url(url: &str) -> anyhow::Result<String> {
    if url.starts_with("data:") {
        return Ok(url.to_string());
    }

    let user_agent = "flow-like/0.1 (info@great-co.de)";
    let response = reqwest::Client::new()
        .get(url)
        .header(reqwest::header::USER_AGENT, user_agent)
        .send()
        .await?;

    let status = response.status();
    if !status.is_success() {
        return Err(anyhow::anyhow!("Failed to download image: {}", status));
    }
    let headers = response.headers().clone();
    let mut content_type = headers
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| anyhow::anyhow!("Missing content type"))?;

    if !content_type.starts_with("image/") {
        // Now we check if the url path ends with an image extension
        let path = url.split('/').next_back().unwrap_or("");
        let path = path.split('?').next().unwrap_or("");
        let extension = path.split('.').next_back().unwrap_or("");

        content_type = match extension {
            "jpg" | "jpeg" => "image/jpeg",
            "png" => "image/png",
            "gif" => "image/gif",
            "webp" => "image/webp",
            "bmp" => "image/bmp",
            "ico" => "image/x-icon",
            "svg" => "image/svg+xml",
            _ => return Err(anyhow::anyhow!("Invalid content type")),
        };
    }

    let bytes = response.bytes().await?;

    // Create a Data URL
    let base64_encoded = STANDARD.encode(&bytes);
    Ok(format!("data:{};base64,{}", content_type, base64_encoded))
}

pub async fn data_url_to_bytes(url: &str) -> anyhow::Result<Vec<u8>> {
    let base64_data = data_url_to_base64(url)?;
    let bytes = STANDARD.decode(base64_data)?;
    Ok(bytes)
}

pub fn data_url_to_base64(url: &str) -> anyhow::Result<&str> {
    url.split(',')
        .next_back()
        .ok_or_else(|| anyhow::anyhow!("Invalid Data URL"))
}

pub async fn pathbuf_to_data_url(path: &std::path::PathBuf) -> anyhow::Result<String> {
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    let base64 = tokio::fs::read(path).await?;
    let base64 = STANDARD.encode(&base64);
    let data_url = format!("data:{};base64,{}", mime, base64);
    Ok(data_url)
}

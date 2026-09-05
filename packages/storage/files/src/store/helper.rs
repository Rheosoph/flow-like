// ---------- HTTP(S) ----------

use anyhow::{Result, anyhow};
use base64::{Engine, engine::general_purpose};
use bytes::Bytes;
use mime_guess::mime;
use object_store::{ObjectStore, PutPayload, path::Path};
use reqwest::Url;
use std::sync::Arc;

pub async fn put_http(parsed: Url, store: Arc<dyn ObjectStore>) -> Result<(Path, usize)> {
    let client = reqwest::Client::new();
    let resp = client.get(parsed.clone()).send().await?;
    let status = resp.status();
    if !status.is_success() {
        return Err(anyhow!("HTTP error {status} for {parsed}"));
    }

    // Pull headers we may need
    let headers = resp.headers().clone();
    // Buffer the body (switch to multipart upload if you expect huge files)
    let body = resp.bytes().await?;

    // Filename candidates (Content-Disposition first — it's explicitly set by the uploader)
    let mut name = filename_from_content_disposition(&headers)
        .or_else(|| filename_from_query(&parsed))
        .or_else(|| filename_from_url_path(&parsed))
        .unwrap_or_else(|| hash_name(&body)); // stable name if none present

    // Extension candidates
    let mut ext = ext_from_name(&name)
        .or_else(|| ext_from_content_type(headers.get(reqwest::header::CONTENT_TYPE)))
        .or_else(|| sniff_ext(&body));

    // Ensure extension present
    if ext.is_none() {
        ext = Some("bin".to_string());
    }
    if ext.is_some() && !name.contains('.') {
        name.push('.');
        name.push_str(ext.as_deref().unwrap());
    }

    let len = body.len();
    let payload = PutPayload::from_bytes(body);
    let path = Path::from(sanitize_for_path(&name));
    store.put(&path, payload).await?;

    Ok((path, len))
}

// ---------- data: URL ----------

pub async fn put_data_url(url: &str, store: Arc<dyn ObjectStore>) -> Result<(Path, usize)> {
    // data:[<mediatype>][;base64],<data>
    let (meta, data_part) = url
        .split_once(',')
        .ok_or_else(|| anyhow!("Invalid data URL"))?;

    let is_base64 = meta.contains(";base64");
    let mime = meta
        .strip_prefix("data:")
        .unwrap_or_default()
        .split(';')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("application/octet-stream");

    let bytes = if is_base64 {
        let decoded = general_purpose::STANDARD
            .decode(data_part)
            .map_err(|e| anyhow!("Base64 decode error: {e}"))?;
        Bytes::from(decoded)
    } else {
        // Percent-decoded (raw) data
        Bytes::from(percent_decode(data_part))
    };

    // Name & extension
    let mut name = hash_name(&bytes); // stable, avoids collisions
    let mut ext = ext_from_mime_str(mime).or_else(|| sniff_ext(&bytes));
    if ext.is_none() {
        ext = Some("bin".to_string());
    }
    if let Some(e) = ext {
        name.push('.');
        name.push_str(&e);
    }

    let len = bytes.len();
    let path = Path::from(sanitize_for_path(&name));
    let payload = PutPayload::from_bytes(bytes);
    store.put(&path, payload).await?;

    Ok((path, len))
}

// ---------- helpers ----------

fn filename_from_url_path(u: &Url) -> Option<String> {
    u.path_segments()?.next_back().and_then(|seg| {
        if seg.is_empty() {
            None
        } else {
            Some(seg.to_string())
        }
    })
}

fn filename_from_query(u: &Url) -> Option<String> {
    // Handle ?filename=foo.png style
    let mut name = None;
    for (k, v) in u.query_pairs() {
        if k.eq_ignore_ascii_case("filename") && !v.is_empty() {
            name = Some(v.to_string());
            break;
        }
    }
    name
}

fn percent_decode_to_string(s: &str) -> String {
    // tolerant: if decoded bytes aren't valid UTF-8, fall back lossy
    percent_encoding::percent_decode_str(s)
        .decode_utf8_lossy()
        .into_owned()
}

fn filename_from_content_disposition(headers: &reqwest::header::HeaderMap) -> Option<String> {
    use reqwest::header::CONTENT_DISPOSITION;
    let val = headers.get(CONTENT_DISPOSITION)?.to_str().ok()?.to_string();

    // Try RFC 5987 filename*=, then legacy filename=
    // e.g., attachment; filename="name.png"; filename*=UTF-8''name.png
    if let Some(idx) = val.to_lowercase().find("filename*=") {
        let (_, rest) = val.split_at(idx + "filename*=".len());
        if let Some((_, enc)) = rest.split_once("''") {
            let end = enc.find(';').unwrap_or(enc.len());
            return Some(percent_decode_to_string(&enc[..end]));
        }
    }
    if let Some(idx) = val.to_lowercase().find("filename=") {
        let (_, rest) = val.split_at(idx + "filename=".len());
        let end = rest.find(';').unwrap_or(rest.len());
        let raw = rest[..end].trim().trim_matches('"');
        if !raw.is_empty() {
            return Some(raw.to_string());
        }
    }
    None
}

fn ext_from_name(name: &str) -> Option<String> {
    std::path::Path::new(name)
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_lowercase())
}

fn ext_from_content_type(h: Option<&reqwest::header::HeaderValue>) -> Option<String> {
    let s = h.and_then(|v| v.to_str().ok())?;
    ext_from_mime_str(s)
}

fn ext_from_mime_str(s: &str) -> Option<String> {
    s.parse::<mime::Mime>().ok().and_then(|m| {
        mime_guess::get_mime_extensions(&m).and_then(|arr| arr.first().map(|e| e.to_string()))
    })
}

fn sniff_ext(bytes: &Bytes) -> Option<String> {
    infer::get(bytes).map(|t| t.extension().to_string())
}

fn hash_name(bytes: &Bytes) -> String {
    let h = blake3::hash(bytes);
    format!("blob-{}", &h.to_hex()[..16])
}

fn percent_decode(s: &str) -> Vec<u8> {
    // tolerate bad percent encodings by falling back to the raw bytes
    percent_encoding::percent_decode_str(s)
        .decode_utf8()
        .map(|cow| cow.into_owned().into_bytes())
        .unwrap_or_else(|_| s.as_bytes().to_vec())
}

fn sanitize_for_path(name: &str) -> String {
    // Keep path-object safe: no slashes, no backslashes; conservative allowed set
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '-'
            }
        })
        .collect()
}

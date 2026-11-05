//! iOS rejects executables whose `__TEXT` segment exceeds 500 MB, and Tauri
//! embeds the frontend there. WASM and formats Brotli cannot shrink (archives,
//! fonts, images, media) ship as signed bundle resources instead.

pub const RESOURCE_DIR: &str = "frontend-wasm";

fn is_compressed_wasm(name: &str) -> bool {
    name.ends_with(".wasm.gz")
        || name.rsplit_once(".wasm.gz.").is_some_and(|(_, part)| {
            !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit())
        })
}

/// The served MIME type of a frontend file that iOS ships as a bundle
/// resource, or `None` when Tauri embeds it in the executable.
pub fn bundled_mime_type(path: &str) -> Option<&'static str> {
    let name = path.rsplit('/').next().unwrap_or(path);
    if is_compressed_wasm(name) {
        // The viewer decompresses these files after fetching and joining chunks.
        return Some("application/octet-stream");
    }
    let (_, extension) = name.rsplit_once('.')?;
    Some(match extension {
        "wasm" => "application/wasm",
        "gz" => "application/gzip",
        "zip" => "application/zip",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "avif" => "image/avif",
        "gif" => "image/gif",
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "webp" => "image/webp",
        "hdr" => "image/vnd.radiance",
        "mp3" => "audio/mpeg",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::bundled_mime_type;

    #[test]
    fn classifies_bundled_payloads_by_file_name() {
        for (path, mime_type) in [
            ("viewer/engine.wasm", "application/wasm"),
            ("viewer/engine.wasm.gz", "application/octet-stream"),
            ("viewer/engine.wasm.gz.003", "application/octet-stream"),
            ("licenses/sources/krilla-0.8.2.tar.gz", "application/gzip"),
            ("_next/static/media/inter.woff2", "font/woff2"),
            ("hdri/studio_small_03_1k.hdr", "image/vnd.radiance"),
            ("placeholder.png", "image/png"),
        ] {
            assert_eq!(bundled_mime_type(path), Some(mime_type), "{path}");
        }
        for path in [
            "index.html",
            "_next/static/chunks/app.js",
            "use/__next._full.txt",
            "viewer/engine.wasm.map",
            "viewer/engine.wasm.js",
            "viewer/engine.wasm.gz.manifest.json",
            "viewer/engine.wasm.gz.",
            "logo.svg",
            "LICENSE",
            ".DS_Store",
        ] {
            assert_eq!(bundled_mime_type(path), None, "{path}");
        }
    }
}

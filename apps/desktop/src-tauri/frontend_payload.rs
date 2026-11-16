//! iOS rejects executables whose `__TEXT` segment exceeds 500 MB, and Tauri
//! embeds the frontend there. WASM and formats Brotli cannot shrink (archives,
//! fonts, images, media) ship as signed bundle resources instead.

pub const RESOURCE_DIR: &str = "frontend-wasm";

/// Android delivers editor runtime chunks in the install-time editors module.
/// Other WASM, runtime manifests, and license archives stay in the base app.
pub fn android_editor_mime_type(path: &str) -> Option<&'static str> {
    let path = path.trim_start_matches('/');
    let chunk = if let Some(chunk) = path.strip_prefix("wordcraft/wordcraft.wasm.gz.") {
        chunk
    } else {
        let (editor, filename) = path.strip_prefix("craft/")?.split_once('/')?;
        if !matches!(
            editor,
            "cadcraft"
                | "deckcraft"
                | "designcraft"
                | "filmcraft"
                | "gridcraft"
                | "pdfcraft"
                | "photocraft"
                | "solidcraft"
                | "soundcraft"
                | "vectorcraft"
        ) {
            return None;
        }
        filename.strip_prefix("craft.wasm.gz.")?
    };
    (chunk.len() == 3 && chunk.bytes().all(|byte| byte.is_ascii_digit()))
        .then_some("application/octet-stream")
}

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
    use super::{android_editor_mime_type, bundled_mime_type};

    #[test]
    fn android_moves_only_editor_runtime_chunks() {
        for path in [
            "wordcraft/wordcraft.wasm.gz.000",
            "/craft/pdfcraft/craft.wasm.gz.002",
            "craft/vectorcraft/craft.wasm.gz.003",
        ] {
            assert_eq!(
                android_editor_mime_type(path),
                Some("application/octet-stream")
            );
        }
        for path in [
            "craft/pdfcraft/runtime.json",
            "craft/pdfcraft/craft.wasm",
            "craft/pdfcraft/craft.wasm.gz",
            "craft/pdfcraft/craft.wasm.gz.001.map",
            "craft/pdfcraft/craft.wasm.gz.01",
            "craft/pdfcraft/craft.wasm.gz.000/extra",
            "craft/licenses/sources/krilla.tar.gz",
            "craft/pdfcraft/licenses/sources/krilla.tar.gz",
            "craft/unknown/craft.wasm.gz.000",
            "wordcraft/licenses/wordcraft.wasm.gz.000",
            "board_canvas.wasm",
            "ort-wasm-simd-threaded.wasm",
        ] {
            assert_eq!(android_editor_mime_type(path), None, "{path}");
        }
    }

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

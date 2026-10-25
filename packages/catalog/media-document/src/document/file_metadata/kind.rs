use flow_like_catalog_core::{looks_like_text, mime_type_for, sniff_file_type};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Image,
    Pdf,
    Office,
    Audio,
    Video,
    Archive,
    Text,
    Other,
}

impl FileKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Pdf => "pdf",
            Self::Office => "office",
            Self::Audio => "audio",
            Self::Video => "video",
            Self::Archive => "archive",
            Self::Text => "text",
            Self::Other => "other",
        }
    }
}

const OOXML_PREFIX: &str = "application/vnd.openxmlformats-officedocument.";

/// Local file headers store part names uncompressed, so a package's main part names it.
const OOXML_MAIN_PARTS: [(&[u8], &str); 3] = [
    (
        b"word/document.xml",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    ),
    (
        b"xl/workbook.xml",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    ),
    (
        b"ppt/presentation.xml",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation",
    ),
];

/// The media type of a file from its leading bytes, falling back to its extension, and the
/// kind that type belongs to. A ZIP container is narrowed to the Office type it carries.
pub fn detect_file_type(prefix: &[u8], file_name: &str) -> (&'static str, FileKind) {
    let by_extension = mime_type_for(file_name);
    let mime_type = match sniff_file_type(prefix) {
        Some(("application/zip", _)) => zip_mime_type(prefix, by_extension),
        Some((mime_type, _)) => mime_type,
        None if looks_like_text(prefix) && !can_be_text(by_extension) => "text/plain",
        None => by_extension,
    };
    (mime_type, kind_of(mime_type))
}

pub fn is_ooxml(mime_type: &str) -> bool {
    mime_type.starts_with(OOXML_PREFIX)
}

fn zip_mime_type(prefix: &[u8], by_extension: &'static str) -> &'static str {
    if is_office(by_extension) {
        return by_extension;
    }
    OOXML_MAIN_PARTS
        .iter()
        .find(|(part, _)| prefix.windows(part.len()).any(|window| window == *part))
        .map_or("application/zip", |(_, mime_type)| mime_type)
}

fn is_office(mime_type: &str) -> bool {
    matches!(
        mime_type,
        "application/msword"
            | "application/vnd.ms-excel"
            | "application/vnd.ms-powerpoint"
            | "application/rtf"
    ) || is_ooxml(mime_type)
        || mime_type.starts_with("application/vnd.oasis.opendocument.")
}

fn is_textual(mime_type: &str) -> bool {
    mime_type.starts_with("text/")
        || matches!(
            mime_type,
            "application/json" | "application/xml" | "message/rfc822"
        )
}

fn can_be_text(mime_type: &str) -> bool {
    is_textual(mime_type) || matches!(mime_type, "image/svg+xml" | "application/rtf")
}

fn kind_of(mime_type: &str) -> FileKind {
    match mime_type {
        "application/pdf" => FileKind::Pdf,
        "application/zip"
        | "application/gzip"
        | "application/x-tar"
        | "application/vnd.rar"
        | "application/x-7z-compressed" => FileKind::Archive,
        _ if is_office(mime_type) => FileKind::Office,
        _ if mime_type.starts_with("image/") => FileKind::Image,
        _ if mime_type.starts_with("audio/") => FileKind::Audio,
        _ if mime_type.starts_with("video/") => FileKind::Video,
        _ if is_textual(mime_type) => FileKind::Text,
        _ => FileKind::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOCX: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.document";

    #[test]
    fn magic_bytes_win_over_a_wrong_extension() {
        assert_eq!(
            detect_file_type(b"\x89PNG\r\n\x1a\n rest", "photo.jpg"),
            ("image/png", FileKind::Image)
        );
        assert_eq!(
            detect_file_type(b"%PDF-1.7\n", "scan"),
            ("application/pdf", FileKind::Pdf)
        );
    }

    #[test]
    fn a_zip_is_narrowed_to_the_office_type_it_carries() {
        let local_header = b"PK\x03\x04\x14\x00\x00\x00\x08\x00word/document.xml";
        assert_eq!(
            detect_file_type(local_header, "upload"),
            (DOCX, FileKind::Office)
        );
        assert_eq!(
            detect_file_type(b"PK\x03\x04\x14\x00", "report.docx"),
            (DOCX, FileKind::Office)
        );
        assert_eq!(
            detect_file_type(b"PK\x03\x04\x14\x00", "bundle.zip"),
            ("application/zip", FileKind::Archive)
        );
    }

    #[test]
    fn unsigned_content_falls_back_to_the_extension() {
        assert_eq!(
            detect_file_type(b"a,b\n1,2\n", "table.csv"),
            ("text/csv", FileKind::Text)
        );
        assert_eq!(
            detect_file_type(b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>", "icon.svg"),
            ("image/svg+xml", FileKind::Image)
        );
        assert_eq!(
            detect_file_type(b"just words", "notes.png"),
            ("text/plain", FileKind::Text)
        );
        assert_eq!(
            detect_file_type(b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1\x00", "legacy.doc"),
            ("application/msword", FileKind::Office)
        );
        assert_eq!(
            detect_file_type(b"\x00\x01\x02", "blob"),
            ("application/octet-stream", FileKind::Other)
        );
    }

    #[test]
    fn audio_and_video_come_from_their_containers() {
        assert_eq!(detect_file_type(b"ID3\x04\x00", "song").1, FileKind::Audio);
        assert_eq!(
            detect_file_type(b"\x00\x00\x00\x18ftypisom\x00\x00", "clip").1,
            FileKind::Video
        );
    }
}

use flow_like_types::anyhow;
use lopdf::Document;
use serde::Serialize;

#[derive(Debug, Default, PartialEq, Serialize)]
pub struct PdfDetails {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pdf_version: Option<String>,
    pub encrypted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keywords: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creator: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub producer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creation_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modification_date: Option<String>,
}

/// Page count, version and Info dictionary of a PDF, read through lopdf's metadata loader,
/// which resolves only the trailer, Info and page-tree objects instead of the whole document.
/// A PDF that needs a password still reports its version and that it is encrypted.
pub fn pdf_details(pdf: &[u8]) -> flow_like_types::Result<PdfDetails> {
    let encrypted = trailer_has_encrypt(pdf);
    match Document::load_metadata_mem(pdf) {
        Ok(meta) => Ok(PdfDetails {
            page_count: Some(meta.page_count),
            pdf_version: Some(meta.version),
            encrypted,
            title: present(meta.title),
            author: present(meta.author),
            subject: present(meta.subject),
            keywords: present(meta.keywords),
            creator: present(meta.creator),
            producer: present(meta.producer),
            creation_date: present(meta.creation_date).map(|date| pdf_date_to_iso(&date)),
            modification_date: present(meta.modification_date).map(|date| pdf_date_to_iso(&date)),
        }),
        Err(lopdf::Error::InvalidPassword | lopdf::Error::Decryption(_)) => Ok(locked_details(pdf)),
        Err(lopdf::Error::Unimplemented(message)) if message.contains("encrypted") => {
            Ok(locked_details(pdf))
        }
        Err(_) if encrypted => Ok(locked_details(pdf)),
        Err(error) => Err(anyhow!("Reading the PDF metadata failed: {error}")),
    }
}

fn locked_details(pdf: &[u8]) -> PdfDetails {
    PdfDetails {
        pdf_version: header_version(pdf),
        encrypted: true,
        ..PdfDetails::default()
    }
}

fn present(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Offsets in a PDF count from its `%PDF-` header, which may follow some leading junk.
fn from_header(pdf: &[u8]) -> &[u8] {
    let window = &pdf[..pdf.len().min(1024)];
    find(window, b"%PDF-").map_or(pdf, |start| &pdf[start..])
}

fn header_version(pdf: &[u8]) -> Option<String> {
    let version: String = from_header(pdf)
        .strip_prefix(b"%PDF-")?
        .iter()
        .take_while(|byte| byte.is_ascii_digit() || **byte == b'.')
        .map(|byte| *byte as char)
        .collect();
    (!version.is_empty()).then_some(version)
}

/// The dictionary of the trailer `startxref` points to: after `trailer` for a classic
/// cross-reference table, or the cross-reference stream's own dictionary.
fn latest_trailer(pdf: &[u8]) -> Option<&[u8]> {
    let pdf = from_header(pdf);
    let tail = pdf.len().saturating_sub(1024);
    let marker = pdf[tail..]
        .windows(b"startxref".len())
        .rposition(|window| window == b"startxref")?
        + tail;
    let digits: String = pdf[marker + b"startxref".len()..]
        .iter()
        .skip_while(|byte| byte.is_ascii_whitespace())
        .take_while(|byte| byte.is_ascii_digit())
        .map(|byte| *byte as char)
        .collect();
    let section = pdf.get(digits.parse::<usize>().ok()?..)?;
    if section.starts_with(b"xref") {
        let trailer = &section[find(section, b"trailer")?..];
        let end = find(trailer, b"startxref").unwrap_or(trailer.len());
        Some(&trailer[..end])
    } else {
        let end = find(section, b"stream").unwrap_or(section.len());
        Some(&section[..end])
    }
}

fn trailer_has_encrypt(pdf: &[u8]) -> bool {
    const KEY: &[u8] = b"/Encrypt";
    let Some(trailer) = latest_trailer(pdf) else {
        return false;
    };
    trailer
        .windows(KEY.len())
        .enumerate()
        .any(|(index, window)| {
            window == KEY
                && trailer
                    .get(index + KEY.len())
                    .is_none_or(|next| !next.is_ascii_alphanumeric())
        })
}

/// `D:YYYYMMDDHHmmSSOHH'mm'` (every part after the year optional) as ISO 8601; anything that
/// does not follow the PDF date format is returned unchanged.
fn pdf_date_to_iso(raw: &str) -> String {
    let body = raw.strip_prefix("D:").unwrap_or(raw);
    let digits_end = body
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(body.len());
    let (digits, zone) = body.split_at(digits_end);
    if digits.len() < 4 || digits.len() > 14 || digits.len() % 2 != 0 {
        return raw.to_string();
    }
    let part = |range: std::ops::Range<usize>, default: &'static str| {
        digits.get(range).unwrap_or(default).to_string()
    };
    let zone = match zone.chars().next() {
        None => String::new(),
        Some('Z') => "Z".to_string(),
        Some(sign @ ('+' | '-')) => {
            let offset: String = zone[1..].chars().filter(char::is_ascii_digit).collect();
            match offset.len() {
                2 => format!("{sign}{offset}:00"),
                4 => format!("{sign}{}:{}", &offset[..2], &offset[2..]),
                _ => return raw.to_string(),
            }
        }
        Some(_) => return raw.to_string(),
    };
    format!(
        "{}-{}-{}T{}:{}:{}{zone}",
        part(0..4, "0000"),
        part(4..6, "01"),
        part(6..8, "01"),
        part(8..10, "00"),
        part(10..12, "00"),
        part(12..14, "00"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::{
        Object, Stream, StringFormat, dictionary,
        encryption::{EncryptionState, EncryptionVersion, Permissions},
    };

    fn two_page_pdf(title: &str) -> Document {
        let mut doc = Document::with_version("1.7");
        let pages_id = doc.new_object_id();
        let mut kids = Vec::new();
        for _ in 0..2 {
            let content_id = doc.add_object(Stream::new(dictionary! {}, b"BT ET".to_vec()));
            kids.push(Object::Reference(doc.add_object(dictionary! {
                "Type" => "Page",
                "Parent" => pages_id,
                "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
                "Contents" => content_id,
            })));
        }
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => kids,
                "Count" => 2,
            }),
        );
        let catalog_id = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        let info_id = doc.add_object(dictionary! {
            "Title" => Object::String(title.as_bytes().to_vec(), StringFormat::Literal),
            "Producer" => Object::String(b"flow-like test".to_vec(), StringFormat::Literal),
            "CreationDate" => Object::String(b"D:20240131120000+01'00'".to_vec(), StringFormat::Literal),
        });
        doc.trailer.set("Root", catalog_id);
        doc.trailer.set("Info", info_id);
        doc.trailer.set(
            "ID",
            Object::Array(vec![
                Object::String(b"0123456789abcdef".to_vec(), StringFormat::Hexadecimal),
                Object::String(b"0123456789abcdef".to_vec(), StringFormat::Hexadecimal),
            ]),
        );
        doc
    }

    fn saved(mut doc: Document) -> Vec<u8> {
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).unwrap();
        bytes
    }

    fn encrypted(user_password: &str) -> Vec<u8> {
        let mut doc = two_page_pdf("Locked");
        let state = EncryptionState::try_from(EncryptionVersion::V1 {
            document: &doc,
            owner_password: "owner",
            user_password,
            permissions: Permissions::default(),
        })
        .unwrap();
        doc.encrypt(&state).unwrap();
        saved(doc)
    }

    #[test]
    fn reads_pages_version_and_info_fields_that_exist() {
        let details = pdf_details(&saved(two_page_pdf("Quarterly Report"))).unwrap();
        assert_eq!(
            details,
            PdfDetails {
                page_count: Some(2),
                pdf_version: Some("1.7".into()),
                encrypted: false,
                title: Some("Quarterly Report".into()),
                producer: Some("flow-like test".into()),
                creation_date: Some("2024-01-31T12:00:00+01:00".into()),
                ..PdfDetails::default()
            }
        );
        let json = flow_like_types::json::to_value(&details).unwrap();
        assert!(json.get("author").is_none());
    }

    #[test]
    fn an_owner_password_still_shows_the_metadata_and_marks_it_encrypted() {
        let details = pdf_details(&encrypted("")).unwrap();
        assert!(details.encrypted);
        assert_eq!(details.page_count, Some(2));
    }

    #[test]
    fn a_user_password_leaves_only_version_and_encryption() {
        let details = pdf_details(&encrypted("secret")).unwrap();
        assert!(details.encrypted);
        assert_eq!(details.pdf_version.as_deref(), Some("1.7"));
        assert_eq!(details.title, None);
    }

    #[test]
    fn a_broken_pdf_is_an_error() {
        assert!(pdf_details(b"%PDF-1.4\nnot really a pdf").is_err());
    }

    #[test]
    fn pdf_dates_become_iso_8601() {
        assert_eq!(
            pdf_date_to_iso("D:20240131120000+01'00'"),
            "2024-01-31T12:00:00+01:00"
        );
        assert_eq!(pdf_date_to_iso("D:20240131120000Z"), "2024-01-31T12:00:00Z");
        assert_eq!(pdf_date_to_iso("D:2024"), "2024-01-01T00:00:00");
        assert_eq!(pdf_date_to_iso("yesterday"), "yesterday");
    }
}

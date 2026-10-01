use super::{Delivery, Envelope, S3Source, max_bytes};
use crate::{error::ApiError, state::AppState};
use axum::http::StatusCode;
use base64::Engine;
use flow_like::credentials::StoreType;
use flow_like_catalog_core::{
    FlowPath, InboundEmail, InboundEmailAddress, InboundEmailAttachment, InboundEmailHeader,
    MailMessageRef, MailSession,
};
use flow_like_storage::{files::store::FlowLikeStore, object_store::ObjectStoreExt};
use flow_like_types::dispatch::REQUEST_FILES_STORE_REF;
use mail_parser::{
    Encoding, Message, MessageParser, MessagePart, MessagePartId, MimeHeaders,
    decoders::{base64::base64_decode, quoted_printable::quoted_printable_decode},
};
use std::{borrow::Cow, collections::HashSet};

const MAX_ATTACHMENT_NAME: usize = 100;
const MAX_EXTENSION: usize = 16;
const MAX_ATTACHMENTS: usize = 100;

pub(super) async fn store(state: &crate::state::State) -> Result<FlowLikeStore, ApiError> {
    Ok(state
        .master_credentials()
        .await?
        .to_store_type(StoreType::Tmp)
        .await?)
}

/// A message above the size limit never becomes deliverable, so it is not retried.
pub(super) fn too_large() -> ApiError {
    ApiError::coded(
        StatusCode::PAYLOAD_TOO_LARGE,
        "MAIL_TOO_LARGE",
        "Message exceeds the mail size limit",
    )
}

pub(super) fn is_too_large(error: &ApiError) -> bool {
    error.status() == StatusCode::PAYLOAD_TOO_LARGE
}

pub(super) async fn read(state: &AppState, envelope: &Envelope) -> Result<Vec<u8>, ApiError> {
    let bytes = if let Some(path) = &envelope.raw_object {
        let encrypted = store(state.as_ref())
            .await?
            .as_generic()
            .get(&path.clone().into())
            .await?
            .bytes()
            .await?;
        let encrypted = std::str::from_utf8(&encrypted)
            .map_err(|_| ApiError::internal("Invalid encrypted mail object"))?;
        let raw = crate::utils::crypto::decrypt_secret(encrypted, &state.encryption_key)
            .ok_or_else(|| ApiError::internal("Cannot decrypt mail object"))?;
        base64::engine::general_purpose::STANDARD
            .decode(raw)
            .map_err(|_| ApiError::internal("Invalid stored MIME encoding"))?
    } else if let Some(source) = &envelope.s3 {
        read_s3(source, max_bytes(state)).await?
    } else {
        return Err(ApiError::internal("Missing mail object"));
    };
    if bytes.is_empty() {
        return Err(ApiError::bad_request("The received message is empty"));
    }
    if bytes.len() > max_bytes(state) {
        return Err(too_large());
    }
    Ok(bytes)
}

#[cfg(feature = "ses")]
async fn s3_client() -> &'static aws_sdk_s3::Client {
    static CLIENT: flow_like_types::tokio::sync::OnceCell<aws_sdk_s3::Client> =
        flow_like_types::tokio::sync::OnceCell::const_new();
    CLIENT
        .get_or_init(|| async {
            aws_sdk_s3::Client::new(
                &aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await,
            )
        })
        .await
}

#[cfg(feature = "ses")]
fn s3_error(operation: &str, source: &S3Source, error: impl std::error::Error) -> ApiError {
    ApiError::internal(format!(
        "Cannot {operation} SES receipt object {}/{}: {}",
        source.bucket,
        source.key,
        aws_sdk_s3::error::DisplayErrorContext(error)
    ))
}

#[cfg(feature = "ses")]
async fn read_s3(source: &S3Source, max_bytes: usize) -> Result<Vec<u8>, ApiError> {
    use flow_like_types::tokio::io::AsyncReadExt;
    let client = s3_client().await;
    let exceeds =
        |size: Option<i64>| size.is_some_and(|size| size < 0 || size as usize > max_bytes);
    let head = client
        .head_object()
        .bucket(&source.bucket)
        .key(&source.key)
        .set_version_id(source.version_id.clone())
        .send()
        .await
        .map_err(|error| s3_error("inspect", source, error))?;
    if exceeds(head.content_length()) {
        return Err(too_large());
    }
    let object = client
        .get_object()
        .bucket(&source.bucket)
        .key(&source.key)
        .set_version_id(source.version_id.clone())
        .send()
        .await
        .map_err(|error| s3_error("read", source, error))?;
    if exceeds(object.content_length()) {
        return Err(too_large());
    }
    let mut bytes = Vec::new();
    object
        .body
        .into_async_read()
        .take((max_bytes + 1) as u64)
        .read_to_end(&mut bytes)
        .await?;
    Ok(bytes)
}
#[cfg(not(feature = "ses"))]
async fn read_s3(_: &S3Source, _: usize) -> Result<Vec<u8>, ApiError> {
    Err(ApiError::bad_request(
        "This server was built without SES support",
    ))
}

/// Deletes the current version; a versioned bucket keeps a noncurrent copy for its lifecycle.
#[cfg(feature = "ses")]
pub(super) async fn delete_s3(source: &S3Source) -> Result<(), ApiError> {
    s3_client()
        .await
        .delete_object()
        .bucket(&source.bucket)
        .key(&source.key)
        .send()
        .await
        .map_err(|error| s3_error("delete", source, error))?;
    Ok(())
}
#[cfg(not(feature = "ses"))]
pub(super) async fn delete_s3(_: &S3Source) -> Result<(), ApiError> {
    Ok(())
}

fn flow_path(path: &str) -> FlowPath {
    FlowPath::new(path.to_owned(), REQUEST_FILES_STORE_REF.to_owned(), None)
}

fn addresses(value: Option<&mail_parser::Address<'_>>) -> Vec<InboundEmailAddress> {
    value
        .into_iter()
        .flat_map(|a| a.iter())
        .take(100)
        .map(|address| InboundEmailAddress {
            name: address.name().map(|name| preview(name, 256).to_owned()),
            email: address
                .address()
                .map(|email| preview(email, 320).to_owned()),
        })
        .collect()
}

fn preview(text: &str, limit: usize) -> &str {
    let mut end = text.len().min(limit);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

fn mailer_daemon(address: &str) -> bool {
    address
        .split('@')
        .next()
        .is_some_and(|local| local.eq_ignore_ascii_case("mailer-daemon"))
}

fn automated_header(name: &str, value: &str) -> bool {
    let token = value
        .trim()
        .split(|c: char| c == ';' || c == '(' || c.is_whitespace())
        .next()
        .unwrap_or_default();
    match name.to_ascii_lowercase().as_str() {
        "auto-submitted" => !token.eq_ignore_ascii_case("no"),
        "precedence" => ["bulk", "junk", "list"]
            .iter()
            .any(|value| token.eq_ignore_ascii_case(value)),
        "list-id" | "x-autoreply" | "x-autorespond" => true,
        _ => false,
    }
}

fn mime_automated(mail: &Message<'_>) -> bool {
    let report = mail.content_type().is_some_and(|content_type| {
        let (kind, subtype) = (
            content_type.ctype(),
            content_type.subtype().unwrap_or_default(),
        );
        (kind.eq_ignore_ascii_case("multipart") && subtype.eq_ignore_ascii_case("report"))
            || (kind.eq_ignore_ascii_case("message")
                && ["delivery-status", "disposition-notification"]
                    .iter()
                    .any(|value| subtype.eq_ignore_ascii_case(value)))
    });
    let daemon = mail
        .from()
        .and_then(|from| from.first())
        .and_then(|from| from.address())
        .is_some_and(mailer_daemon);
    report
        || daemon
        || mail
            .headers_raw()
            .any(|(name, value)| automated_header(name, value))
}

fn envelope_automated(envelope_from: &str) -> bool {
    let sender = envelope_from
        .trim()
        .trim_start_matches('<')
        .trim_end_matches('>')
        .trim();
    sender.is_empty() || mailer_daemon(sender)
}

/// Bounces, receipts, auto-replies and list traffic (RFC 3834, RFC 3464, RFC 8098, RFC 2919).
pub(crate) fn is_automated(envelope_from: &str, mail: &Message<'_>) -> bool {
    envelope_automated(envelope_from) || mime_automated(mail)
}

fn sanitize_file_name(raw: &str) -> String {
    let base = raw.rsplit(['/', '\\']).next().unwrap_or_default();
    let mut name = String::with_capacity(base.len());
    for c in base.chars() {
        let c = if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
            c
        } else {
            '_'
        };
        if !(c == '_' && name.ends_with('_')) {
            name.push(c);
        }
    }
    name.trim_matches(['.', '_']).to_owned()
}

fn extension(name: &str) -> Option<(&str, &str)> {
    name.rsplit_once('.').filter(|(stem, extension)| {
        !stem.is_empty()
            && (1..=MAX_EXTENSION).contains(&extension.len())
            && extension.bytes().all(|b| b.is_ascii_alphanumeric())
    })
}

fn extension_for(content_type: &str) -> &'static str {
    match content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "application/pdf" => "pdf",
        "text/plain" => "txt",
        "text/html" => "html",
        "text/csv" => "csv",
        "text/calendar" => "ics",
        "application/json" => "json",
        "application/xml" | "text/xml" => "xml",
        "application/zip" => "zip",
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/svg+xml" => "svg",
        "message/rfc822" => "eml",
        "application/msword" => "doc",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => "docx",
        "application/vnd.ms-excel" => "xls",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => "xlsx",
        "application/vnd.ms-powerpoint" => "ppt",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation" => "pptx",
        _ => "bin",
    }
}

/// Object name for an attachment: its sanitized filename when that keeps an extension.
fn attachment_name(index: usize, filename: Option<&str>, content_type: &str) -> String {
    filename
        .map(sanitize_file_name)
        .and_then(|name| {
            let (stem, ext) = extension(&name)?;
            if name.len() <= MAX_ATTACHMENT_NAME {
                return Some(name.clone());
            }
            let stem = stem[..MAX_ATTACHMENT_NAME - ext.len() - 1].trim_end_matches(['.', '_']);
            (!stem.is_empty()).then(|| format!("{stem}.{ext}"))
        })
        .unwrap_or_else(|| format!("attachment-{index}.{}", extension_for(content_type)))
}

type Objects = Vec<(String, Vec<u8>)>;

/// Named inline text parts that mail-parser files as body text are attachments the sender added.
fn promoted_text_parts(mail: &Message<'_>) -> Vec<MessagePartId> {
    let mut listed: HashSet<MessagePartId> = mail.attachments.iter().copied().collect();
    mail.text_body
        .iter()
        .chain(&mail.html_body)
        .copied()
        .filter(|id| {
            mail.part(*id).is_some_and(|part| {
                part.is_text()
                    && part
                        .content_disposition()
                        .is_some_and(|disposition| disposition.has_attribute("filename"))
            }) && listed.insert(*id)
        })
        .collect()
}

/// Every body part in order; the first one stays body text even when it is also a file.
fn joined_body<'x>(
    ids: &[MessagePartId],
    promoted: &HashSet<MessagePartId>,
    body: impl Fn(usize) -> Option<Cow<'x, str>>,
) -> Option<String> {
    let parts = ids
        .iter()
        .enumerate()
        .filter(|&(position, id)| position == 0 || !promoted.contains(id))
        .filter_map(|(position, _)| body(position))
        .collect::<Vec<_>>();
    (!parts.is_empty()).then(|| parts.join("\n"))
}

fn utf8_compatible(charset: Option<&str>) -> bool {
    charset.is_none_or(|charset| {
        ["utf-8", "utf8", "us-ascii", "ascii"]
            .iter()
            .any(|name| charset.eq_ignore_ascii_case(name))
    })
}

/// mail-parser transcodes text parts to UTF-8, replacing undecodable bytes. Text in a
/// UTF-8 compatible or undeclared charset is stored as the sender's transfer-decoded bytes.
fn attachment_bytes(raw: &[u8], part: &MessagePart<'_>, charset: Option<&str>) -> Vec<u8> {
    if !part.is_text() || !utf8_compatible(charset) {
        return part.contents().to_vec();
    }
    raw.get(part.raw_body_offset() as usize..part.raw_end_offset() as usize)
        .and_then(|encoded| match part.encoding {
            Encoding::Base64 => base64_decode(encoded),
            Encoding::QuotedPrintable => quoted_printable_decode(encoded),
            Encoding::None => Some(encoded.to_vec()),
        })
        .unwrap_or_else(|| part.contents().to_vec())
}

fn content_id(part: &MessagePart<'_>) -> Option<String> {
    part.content_id()
        .map(|id| id.trim().trim_start_matches('<').trim_end_matches('>'))
        .filter(|id| !id.is_empty())
        .map(|id| preview(id, 256).to_owned())
}

fn disposition(part: &MessagePart<'_>) -> Option<String> {
    part.content_disposition().and_then(|disposition| {
        if disposition.is_attachment() {
            Some("attachment".to_owned())
        } else {
            disposition.is_inline().then(|| "inline".to_owned())
        }
    })
}

/// `lowercase_html` must already be ASCII lower-case.
fn embedded(content_id: Option<&str>, lowercase_html: &str) -> bool {
    content_id
        .is_some_and(|id| lowercase_html.contains(&format!("cid:{}", id.to_ascii_lowercase())))
}

fn attachment(
    raw: &[u8],
    prefix: &str,
    index: usize,
    part: &MessagePart<'_>,
    lowercase_html: &str,
) -> (InboundEmailAttachment, (String, Vec<u8>)) {
    let content_type = part
        .content_type()
        .map(|t| format!("{}/{}", t.ctype(), t.subtype().unwrap_or("octet-stream")))
        .unwrap_or_else(|| "application/octet-stream".into());
    let charset = part
        .content_type()
        .and_then(|t| t.attribute("charset"))
        .map(str::trim)
        .filter(|charset| !charset.is_empty());
    // Each index owns its folder; the sanitized name only makes the file recognizable.
    let path = format!(
        "{prefix}/attachments/{index}/{}",
        attachment_name(index, part.attachment_name(), &content_type)
    );
    let bytes = attachment_bytes(raw, part, charset);
    let content_id = content_id(part);
    let attachment = InboundEmailAttachment {
        filename: part
            .attachment_name()
            .map(|name| preview(name, 256).to_owned()),
        content_type: preview(&content_type, 256).to_owned(),
        size: bytes.len() as u64,
        path: flow_path(&path),
        embedded: embedded(content_id.as_deref(), lowercase_html),
        content_id,
        disposition: disposition(part),
        charset: charset.map(|charset| preview(charset, 256).to_owned()),
    };
    (attachment, (path, bytes))
}

fn parse(raw: &[u8], prefix: &str) -> Result<(InboundEmail, Objects), ApiError> {
    let mail = MessageParser::default()
        .parse(raw)
        .ok_or_else(|| ApiError::bad_request("Invalid MIME message"))?;
    let promoted = promoted_text_parts(&mail);
    let promoted_ids = promoted.iter().copied().collect::<HashSet<_>>();
    let text = joined_body(&mail.text_body, &promoted_ids, |i| mail.body_text(i));
    let html = joined_body(&mail.html_body, &promoted_ids, |i| mail.body_html(i));
    let lowercase_html = html
        .as_deref()
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let parts = mail
        .attachments
        .iter()
        .chain(&promoted)
        .filter_map(|id| mail.part(*id))
        .collect::<Vec<_>>();
    // Only the first attachments become files; the raw message keeps every part.
    let omitted_attachments =
        u32::try_from(parts.len().saturating_sub(MAX_ATTACHMENTS)).unwrap_or(u32::MAX);
    let raw_path = format!("{prefix}/raw.eml");
    let mut objects = vec![(raw_path.clone(), raw.to_vec())];
    let mut attachments = Vec::new();
    for (index, part) in parts.into_iter().take(MAX_ATTACHMENTS).enumerate() {
        let (attachment, object) = attachment(raw, prefix, index, part, &lowercase_html);
        attachments.push(attachment);
        objects.push(object);
    }
    let from = addresses(mail.from());
    let sender = addresses(mail.sender())
        .into_iter()
        .next()
        .or_else(|| from.first().cloned());
    // prepare fills the transport identity before this value can be dispatched.
    let mut email = InboundEmail {
        id: String::new(),
        delivery_id: String::new(),
        envelope_from: String::new(),
        recipient: String::new(),
        session: None,
        reference: None,
        sender,
        from,
        to: addresses(mail.to()),
        cc: addresses(mail.cc()),
        reply_to: addresses(mail.reply_to()),
        subject: mail.subject().map(|s| preview(s, 4096).to_owned()),
        message_id: mail.message_id().map(|id| preview(id, 1024).to_owned()),
        text: None,
        html: None,
        text_truncated: false,
        html_truncated: false,
        text_path: None,
        html_path: None,
        attachments,
        omitted_attachments,
        raw_path: flow_path(&raw_path),
        headers: Vec::new(),
        received_at: None,
        expires_at: None,
        authentication: None,
        automated: mime_automated(&mail),
    };
    for (kind, body) in [("txt", text), ("html", html)] {
        if let Some(body) = body {
            let path = format!("{prefix}/body.{kind}");
            let value = Some(preview(&body, 16 * 1024).to_owned());
            let truncated = body.len() > 16 * 1024;
            let file = Some(flow_path(&path));
            if kind == "txt" {
                email.text = value;
                email.text_truncated = truncated;
                email.text_path = file;
            } else {
                email.html = value;
                email.html_truncated = truncated;
                email.html_path = file;
            }
            objects.push((path, body.into_bytes()));
        }
    }
    let mut budget = 32 * 1024;
    email.headers = mail
        .headers_raw()
        .take(100)
        .filter_map(|(name, value)| {
            let name = preview(name, 128);
            let value = preview(value, budget.min(2048));
            budget = budget.saturating_sub(name.len() + value.len());
            (budget > 0).then(|| InboundEmailHeader {
                name: name.to_owned(),
                value: value.to_owned(),
            })
        })
        .collect();
    Ok((email, objects))
}

pub(super) async fn prepare(
    state: &AppState,
    row: &Delivery,
    sink: &crate::entity::event_sink::Model,
    envelope: &Envelope,
    raw: &[u8],
    expires_at: i64,
) -> Result<(InboundEmail, Objects), ApiError> {
    let actor = super::sink_owner(state, sink)
        .await?
        .unwrap_or_else(|| format!("sink:{}", sink.id));
    let user = crate::credentials::storage_path_segment(&actor, "user");
    let app = crate::credentials::storage_path_segment(&row.app_id, "app");
    let prefix = format!(
        "tmp/user/{user}/apps/{app}/runs/{}/request/mail",
        row.run_id
    );
    let (mut email, objects) = parse(raw, &prefix)?;
    email.id = row.id.clone();
    email.delivery_id = envelope.delivery_id.clone();
    email.envelope_from = envelope.envelope_from.clone();
    email.recipient = envelope.recipient.clone();
    email.automated |= envelope_automated(&envelope.envelope_from);
    let session = MailSession {
        app_id: row.app_id.clone(),
        event_id: row.event_id.clone(),
    };
    email.reference = Some(MailMessageRef {
        session: session.clone(),
        delivery_id: row.id.clone(),
    });
    email.session = Some(session);
    email.received_at =
        chrono::DateTime::from_timestamp_millis(row.received_at).map(|d| d.to_rfc3339());
    email.expires_at = chrono::DateTime::from_timestamp_millis(expires_at).map(|d| d.to_rfc3339());
    email.authentication = envelope.authentication.clone();
    Ok((email, objects))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mime_attachments_keep_bytes_without_trusting_filenames() {
        let raw=b"From: Sender <sender@example.com>\r\nTo: displayed@example.com\r\nSubject: Invoice\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=x\r\n\r\n--x\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nHello\r\n--x\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=\"../../invoice.bin\"\r\nContent-Transfer-Encoding: base64\r\n\r\nAAECAw==\r\n--x--\r\n";
        let (email, objects) = parse(raw, "tmp/test").unwrap();
        assert_eq!(email.subject.as_deref(), Some("Invoice"));
        assert_eq!(
            email.attachments[0].path.path,
            "tmp/test/attachments/0/invoice.bin"
        );
        assert_eq!(
            email.attachments[0].filename.as_deref(),
            Some("../../invoice.bin")
        );
        assert_eq!(
            objects
                .iter()
                .find(|(p, _)| p.ends_with("attachments/0/invoice.bin"))
                .unwrap()
                .1,
            vec![0, 1, 2, 3]
        );
        assert!(
            email.recipient.is_empty(),
            "Routing comes from the SMTP envelope, never MIME To"
        );
        assert!(!email.automated);
    }

    #[test]
    fn attachment_names_are_safe_recognizable_and_always_have_an_extension() {
        for (filename, content_type, expected) in [
            (
                Some("Quarterly Report (final).pdf"),
                "application/pdf",
                "Quarterly_Report_final_.pdf",
            ),
            (
                Some("Übersicht 2026.xlsx"),
                "application/octet-stream",
                "bersicht_2026.xlsx",
            ),
            (Some("C:\\Users\\me\\scan.png"), "image/png", "scan.png"),
            (Some(".htaccess"), "text/plain", "attachment-3.txt"),
            (Some("README"), "text/plain", "attachment-3.txt"),
            (Some("../.."), "image/jpeg", "attachment-3.jpg"),
            (Some("evil.p df"), "application/pdf", "attachment-3.pdf"),
            (None, "application/x-unknown", "attachment-3.bin"),
            (None, "text/calendar; method=REQUEST", "attachment-3.ics"),
        ] {
            assert_eq!(
                attachment_name(3, filename, content_type),
                expected,
                "{filename:?}"
            );
        }
        let long = format!("{}.pdf", "a".repeat(300));
        let name = attachment_name(0, Some(&long), "application/pdf");
        assert_eq!(name.len(), MAX_ATTACHMENT_NAME);
        assert!(name.ends_with("a.pdf"));
        for name in [
            attachment_name(0, Some("a/../../b.txt"), "text/plain"),
            attachment_name(0, Some("\u{0}..\u{202e}fdp.exe"), "text/plain"),
        ] {
            assert!(!name.starts_with('.'));
            assert!(
                name.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)),
                "{name}"
            );
        }
    }

    #[test]
    fn long_unicode_bodies_have_a_bounded_preview_and_complete_file() {
        let body = "日".repeat(10_000);
        let raw = format!("Content-Type: text/plain; charset=utf-8\r\n\r\n{body}");
        let (email, objects) = parse(raw.as_bytes(), "tmp/test").unwrap();
        assert!(email.text.as_ref().unwrap().len() <= 16 * 1024);
        assert!(email.text_truncated);
        assert_eq!(email.text_path.as_ref().unwrap().path, "tmp/test/body.txt");
        assert_eq!(
            objects
                .iter()
                .find(|(p, _)| p.ends_with("body.txt"))
                .unwrap()
                .1,
            body.as_bytes()
        );
    }

    #[test]
    fn html_bodies_are_stored_as_html_files() {
        let raw = b"Content-Type: text/html; charset=utf-8\r\n\r\n<p>Hello</p>";
        let (email, _) = parse(raw, "tmp/test").unwrap();
        assert_eq!(email.html_path.as_ref().unwrap().path, "tmp/test/body.html");
    }

    #[test]
    fn mime_sender_and_author_do_not_supply_transport_identity() {
        let raw = b"From: Author <author@example.com>\r\nSender: Agent <agent@example.com>\r\nReply-To: support@example.com\r\nTo: displayed@example.com\r\nReturn-Path: <claimed-envelope@example.com>\r\nSubject: Order\r\n\r\nBody";
        let (email, _) = parse(raw, "tmp/test").unwrap();
        let sender = email.sender.as_ref().unwrap();
        assert_eq!(sender.name.as_deref(), Some("Agent"));
        assert_eq!(sender.email.as_deref(), Some("agent@example.com"));
        assert_eq!(email.from[0].email.as_deref(), Some("author@example.com"));
        assert_eq!(
            email.reply_to[0].email.as_deref(),
            Some("support@example.com")
        );
        assert_eq!(email.to[0].email.as_deref(), Some("displayed@example.com"));
        assert!(email.envelope_from.is_empty());
        assert!(email.recipient.is_empty());
        assert!(email.id.is_empty() && email.delivery_id.is_empty());
        assert!(email.session.is_none() && email.reference.is_none());

        let (email, _) = parse(
            b"From: First <first@example.com>, Second <second@example.com>\r\n\r\nBody",
            "tmp/test",
        )
        .unwrap();
        assert_eq!(email.from.len(), 2);
        assert_eq!(
            email.sender.as_ref().unwrap().email.as_deref(),
            Some("first@example.com")
        );
    }

    #[test]
    fn absent_body_and_attachments_store_only_the_original_message() {
        let raw = b"From: author@example.com\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=empty\r\n\r\n--empty--\r\n";
        let (email, objects) = parse(raw, "tmp/test").unwrap();
        assert!(email.text.is_none() && email.html.is_none());
        assert!(email.text_path.is_none() && email.html_path.is_none());
        assert!(!email.text_truncated && !email.html_truncated);
        assert!(email.attachments.is_empty());
        assert_eq!(objects, vec![("tmp/test/raw.eml".into(), raw.to_vec())]);
        assert_eq!(email.raw_path.store_ref, REQUEST_FILES_STORE_REF);
    }

    #[test]
    fn typed_payload_round_trips_content_and_file_references() {
        let raw = b"From: author@example.com\r\nSubject: Report\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=x\r\n\r\n--x\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nReport attached\r\n--x\r\nContent-Type: application/pdf\r\nContent-Disposition: attachment; filename=report.pdf\r\nContent-Transfer-Encoding: base64\r\n\r\nAAECAw==\r\n--x--\r\n";
        let (email, objects) = parse(raw, "tmp/test").unwrap();
        let value = serde_json::to_value(&email).unwrap();
        let restored: InboundEmail = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(&restored).unwrap(), value);
        assert_eq!(restored.subject.as_deref(), Some("Report"));
        assert_eq!(
            restored.attachments[0].filename.as_deref(),
            Some("report.pdf")
        );
        assert_eq!(
            restored.attachments[0].path.path,
            "tmp/test/attachments/0/report.pdf"
        );
        assert_eq!(restored.attachments[0].content_type, "application/pdf");
        assert_eq!(restored.attachments[0].size, 4);
        for serialized in [
            &value["raw_path"],
            &value["text_path"],
            &value["attachments"][0]["path"],
        ] {
            let path: FlowPath = serde_json::from_value(serialized.clone()).unwrap();
            assert_eq!(path.store_ref, REQUEST_FILES_STORE_REF);
            assert!(path.cache_store_ref.is_none());
            assert!(objects.iter().any(|(key, _)| key == &path.path));
        }
    }

    fn stored<'a>(objects: &'a Objects, attachment: &InboundEmailAttachment) -> &'a [u8] {
        &objects
            .iter()
            .find(|(path, _)| path == &attachment.path.path)
            .unwrap()
            .1
    }

    fn body<'a>(objects: &'a Objects, kind: &str) -> &'a str {
        let suffix = format!("body.{kind}");
        std::str::from_utf8(
            &objects
                .iter()
                .find(|(p, _)| p.ends_with(&suffix))
                .unwrap()
                .1,
        )
        .unwrap()
    }

    #[test]
    fn named_inline_text_parts_become_attachments_instead_of_vanishing() {
        let raw = b"From: a@example.com\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=x\r\n\r\n--x\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nHello\r\n--x\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Disposition: inline; filename=notes.txt\r\n\r\nMeeting notes\r\n--x--\r\n";
        let (email, objects) = parse(raw, "tmp/test").unwrap();
        assert_eq!(email.attachments.len(), 1);
        let notes = &email.attachments[0];
        assert_eq!(notes.filename.as_deref(), Some("notes.txt"));
        assert_eq!(notes.path.path, "tmp/test/attachments/0/notes.txt");
        assert_eq!(notes.disposition.as_deref(), Some("inline"));
        assert_eq!(notes.charset.as_deref(), Some("utf-8"));
        assert!(!notes.embedded);
        assert_eq!(stored(&objects, notes), b"Meeting notes");
        assert_eq!(email.text.as_deref(), Some("Hello"));
        assert_eq!(body(&objects, "txt"), "Hello");
        assert_eq!(email.omitted_attachments, 0);
    }

    #[test]
    fn a_single_named_text_part_is_both_body_and_attachment() {
        let raw = b"From: a@example.com\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Disposition: inline; filename=notes.txt\r\n\r\nOnly part";
        let (email, _) = parse(raw, "tmp/test").unwrap();
        assert_eq!(email.text.as_deref(), Some("Only part"));
        assert_eq!(email.attachments.len(), 1);
        assert_eq!(email.attachments[0].filename.as_deref(), Some("notes.txt"));
    }

    #[test]
    fn body_text_after_an_inline_attachment_is_kept() {
        let raw = b"From: a@example.com\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=x\r\n\r\n--x\r\nContent-Type: text/plain\r\n\r\nBefore\r\n--x\r\nContent-Type: application/pdf; name=scan.pdf\r\nContent-Disposition: inline; filename=scan.pdf\r\nContent-Transfer-Encoding: base64\r\n\r\nJVBERi0=\r\n--x\r\nContent-Type: text/plain\r\n\r\nAfter\r\n--x--\r\n";
        let (email, objects) = parse(raw, "tmp/test").unwrap();
        assert_eq!(email.attachments.len(), 1);
        assert_eq!(email.attachments[0].filename.as_deref(), Some("scan.pdf"));
        assert_eq!(stored(&objects, &email.attachments[0]), b"%PDF-");
        let text = email.text.unwrap();
        assert!(text.contains("Before") && text.contains("After"), "{text}");
        let file = body(&objects, "txt");
        assert!(file.contains("Before") && file.contains("After"), "{file}");
    }

    #[test]
    fn apple_layouts_keep_every_html_fragment_and_inline_files_are_not_embedded() {
        let raw = b"From: a@example.com\r\nMIME-Version: 1.0\r\nContent-Type: multipart/alternative; boundary=a\r\n\r\n--a\r\nContent-Type: text/plain; charset=us-ascii\r\n\r\nBefore\r\n\r\nAfter\r\n--a\r\nContent-Type: multipart/mixed; boundary=m\r\n\r\n--m\r\nContent-Type: text/html; charset=us-ascii\r\n\r\n<html><body><div>Before</div></body></html>\r\n--m\r\nContent-Type: application/pdf; name=\"contract.pdf\"\r\nContent-Disposition: inline; filename=\"contract.pdf\"\r\nContent-Id: <5A1B2C3D-apple@example.com>\r\nContent-Transfer-Encoding: base64\r\n\r\nJVBERi0=\r\n--m\r\nContent-Type: text/html; charset=us-ascii\r\n\r\n<html><body><div>After</div></body></html>\r\n--m--\r\n--a--\r\n";
        let (email, objects) = parse(raw, "tmp/test").unwrap();
        let html = body(&objects, "html");
        assert!(
            html.contains("<div>Before</div>") && html.contains("<div>After</div>"),
            "{html}"
        );
        assert_eq!(email.attachments.len(), 1);
        let pdf = &email.attachments[0];
        assert_eq!(pdf.filename.as_deref(), Some("contract.pdf"));
        assert_eq!(pdf.disposition.as_deref(), Some("inline"));
        assert_eq!(
            pdf.content_id.as_deref(),
            Some("5A1B2C3D-apple@example.com")
        );
        assert!(!pdf.embedded);
    }

    #[test]
    fn text_attachments_without_a_foreign_charset_keep_their_original_bytes() {
        let raw = b"From: a@example.com\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=x\r\n\r\n--x\r\nContent-Type: text/plain\r\n\r\nSee files\r\n--x\r\nContent-Type: text/plain\r\nContent-Disposition: attachment; filename=raw.txt\r\nContent-Transfer-Encoding: base64\r\n\r\nR3Lk32U=\r\n--x\r\nContent-Type: text/csv; charset=utf-8\r\nContent-Disposition: attachment; filename=list.csv\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\na;b=\r\n;c=E4\r\n--x\r\nContent-Type: text/plain; charset=iso-8859-1\r\nContent-Disposition: attachment; filename=latin.txt\r\nContent-Transfer-Encoding: base64\r\n\r\nR3Lk32U=\r\n--x--\r\n";
        let (email, objects) = parse(raw, "tmp/test").unwrap();
        assert_eq!(email.attachments.len(), 3);
        let undeclared = &email.attachments[0];
        assert_eq!(stored(&objects, undeclared), b"Gr\xe4\xdfe");
        assert_eq!(undeclared.size, 5);
        assert!(undeclared.charset.is_none());
        let utf8 = &email.attachments[1];
        assert_eq!(stored(&objects, utf8), b"a;b;c\xe4");
        assert_eq!(utf8.size, 6);
        let latin = &email.attachments[2];
        assert_eq!(stored(&objects, latin), "Gr\u{e4}\u{df}e".as_bytes());
        assert_eq!(latin.size, 7);
        assert_eq!(latin.charset.as_deref(), Some("iso-8859-1"));
    }

    #[test]
    fn outlook_signature_images_are_embedded_but_real_attachments_are_not() {
        let raw = b"From: a@example.com\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=m\r\n\r\n--m\r\nContent-Type: multipart/related; boundary=r; type=\"text/html\"\r\n\r\n--r\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<html><body><p>Invoice attached</p><img src=\"cid:Image001.png@01DB1234.56789AB0\"></body></html>\r\n--r\r\nContent-Type: image/png; name=\"image001.png\"\r\nContent-Description: image001.png\r\nContent-Disposition: inline; filename=\"image001.png\"\r\nContent-ID: <image001.png@01DB1234.56789AB0>\r\nContent-Transfer-Encoding: base64\r\n\r\niVBORw0KGgo=\r\n--r--\r\n\r\n--m\r\nContent-Type: application/pdf; name=\"invoice.pdf\"\r\nContent-Disposition: attachment; filename=\"invoice.pdf\"\r\nContent-Transfer-Encoding: base64\r\n\r\nJVBERi0=\r\n--m--\r\n";
        let (email, objects) = parse(raw, "tmp/test").unwrap();
        assert_eq!(email.attachments.len(), 2);
        let logo = &email.attachments[0];
        assert_eq!(logo.filename.as_deref(), Some("image001.png"));
        assert_eq!(
            logo.content_id.as_deref(),
            Some("image001.png@01DB1234.56789AB0")
        );
        assert_eq!(logo.disposition.as_deref(), Some("inline"));
        assert!(logo.embedded);
        assert_eq!(stored(&objects, logo), b"\x89PNG\r\n\x1a\n");
        let invoice = &email.attachments[1];
        assert_eq!(invoice.filename.as_deref(), Some("invoice.pdf"));
        assert_eq!(invoice.disposition.as_deref(), Some("attachment"));
        assert!(invoice.content_id.is_none());
        assert!(!invoice.embedded);
    }

    #[test]
    fn more_than_one_hundred_attachments_are_truncated_instead_of_refused() {
        let mut raw = String::from(
            "From: a@example.com\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=x\r\n\r\n--x\r\nContent-Type: text/plain\r\n\r\nMany files\r\n",
        );
        for index in 0..=MAX_ATTACHMENTS {
            raw.push_str(&format!(
                "--x\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=file-{index}.bin\r\n\r\n{index}\r\n"
            ));
        }
        raw.push_str("--x--\r\n");
        let (email, objects) = parse(raw.as_bytes(), "tmp/test").unwrap();
        assert_eq!(email.attachments.len(), MAX_ATTACHMENTS);
        assert_eq!(email.omitted_attachments, 1);
        assert_eq!(
            email.attachments[MAX_ATTACHMENTS - 1].filename.as_deref(),
            Some("file-99.bin")
        );
        assert!(!objects.iter().any(|(path, _)| path.contains("file-100")));
        assert_eq!(email.text.as_deref(), Some("Many files"));
    }

    #[test]
    fn payloads_without_attachment_details_still_deserialize() {
        let file =
            |path: &str| serde_json::json!({"path": path, "store_ref": REQUEST_FILES_STORE_REF});
        let legacy: InboundEmail = serde_json::from_value(serde_json::json!({
            "id": "mail-1", "delivery_id": "provider-1",
            "envelope_from": "bounce@example.com", "recipient": "orders@example.com",
            "attachments": [{"filename": "invoice.pdf", "content_type": "application/pdf",
                "size": 4, "path": file("tmp/mail/attachments/0/invoice.pdf")}],
            "raw_path": file("tmp/mail/raw.eml")
        }))
        .unwrap();
        assert_eq!(legacy.omitted_attachments, 0);
        let attachment = &legacy.attachments[0];
        assert!(attachment.content_id.is_none() && attachment.disposition.is_none());
        assert!(attachment.charset.is_none() && !attachment.embedded);
    }

    fn message(raw: &[u8]) -> Message<'_> {
        MessageParser::default().parse(raw).unwrap()
    }

    #[test]
    fn automated_mail_is_recognized_from_envelope_headers_and_reports() {
        let human: &[u8] =
            b"From: Person <person@example.com>\r\nAuto-Submitted: no\r\nSubject: Hi\r\n\r\nBody";
        assert!(!is_automated("person@example.com", &message(human)));
        for sender in ["", "<>", " <> ", "MAILER-DAEMON@example.com"] {
            assert!(is_automated(sender, &message(human)), "{sender:?}");
        }
        for raw in [
            &b"From: a@example.com\r\nAuto-Submitted: auto-replied\r\n\r\nAway"[..],
            b"From: a@example.com\r\nAuto-Submitted: auto-generated (vacation)\r\n\r\nAway",
            b"From: a@example.com\r\nPrecedence: bulk\r\n\r\nNews",
            b"From: a@example.com\r\nprecedence: List\r\n\r\nNews",
            b"From: a@example.com\r\nList-Id: <news.example.com>\r\n\r\nNews",
            b"From: a@example.com\r\nX-Autoreply: yes\r\n\r\nAway",
            b"From: a@example.com\r\nX-Autorespond: yes\r\n\r\nAway",
            b"From: Mail Delivery <MAILER-DAEMON@example.com>\r\n\r\nBounce",
            b"From: a@example.com\r\nContent-Type: multipart/report; report-type=delivery-status; boundary=x\r\n\r\n--x--\r\n",
            b"From: a@example.com\r\nContent-Type: message/disposition-notification\r\n\r\nRead",
        ] {
            assert!(
                is_automated("sender@example.com", &message(raw)),
                "{}",
                String::from_utf8_lossy(raw)
            );
        }
        let (email, _) = parse(
            b"From: a@example.com\r\nAuto-Submitted: auto-replied\r\n\r\nAway",
            "tmp/test",
        )
        .unwrap();
        assert!(email.automated);
        assert!(!is_automated(
            "person@example.com",
            &message(b"From: a@example.com\r\nPrecedence: first-class\r\n\r\nBody")
        ));
    }
}

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
use mail_parser::{Message, MessageParser, MimeHeaders};

const MAX_ATTACHMENT_NAME: usize = 100;
const MAX_EXTENSION: usize = 16;

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

fn parse(raw: &[u8], prefix: &str) -> Result<(InboundEmail, Objects), ApiError> {
    let mail = MessageParser::default()
        .parse(raw)
        .ok_or_else(|| ApiError::bad_request("Invalid MIME message"))?;
    if mail.attachment_count() > 100 {
        return Err(ApiError::bad_request("Email has more than 100 attachments"));
    }
    let raw_path = format!("{prefix}/raw.eml");
    let mut objects = vec![(raw_path.clone(), raw.to_vec())];
    let mut attachments = Vec::new();
    for (index, part) in mail.attachments().enumerate() {
        let content_type = part
            .content_type()
            .map(|t| format!("{}/{}", t.ctype(), t.subtype().unwrap_or("octet-stream")))
            .unwrap_or_else(|| "application/octet-stream".into());
        // Each index owns its folder; the sanitized name only makes the file recognizable.
        let path = format!(
            "{prefix}/attachments/{index}/{}",
            attachment_name(index, part.attachment_name(), &content_type)
        );
        let bytes = part.contents().to_vec();
        attachments.push(InboundEmailAttachment {
            filename: part
                .attachment_name()
                .map(|name| preview(name, 256).to_owned()),
            content_type: preview(&content_type, 256).to_owned(),
            size: bytes.len() as u64,
            path: flow_path(&path),
        });
        objects.push((path, bytes));
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
        raw_path: flow_path(&raw_path),
        headers: Vec::new(),
        received_at: None,
        expires_at: None,
        authentication: None,
        automated: mime_automated(&mail),
    };
    for (kind, body) in [("txt", mail.body_text(0)), ("html", mail.body_html(0))] {
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
            objects.push((path, body.as_bytes().to_vec()));
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

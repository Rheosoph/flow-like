#[cfg(feature = "execute")]
use async_smtp::{Envelope, SendableEmail};
#[cfg(feature = "execute")]
use chrono::Offset;
#[cfg(feature = "execute")]
use flow_like::flow::execution::LogLevel;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::FlowPath;
#[cfg(feature = "execute")]
use flow_like_types::anyhow;
use flow_like_types::{async_trait, json::json};

use crate::mail::smtp::SmtpConnection;
#[cfg(feature = "execute")]
use crate::mail::{generate_mail_footer_html, generate_mail_footer_plain};

#[crate::register_node]
#[derive(Default)]
pub struct SmtpSendMailNode;

impl SmtpSendMailNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SmtpSendMailNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "email_smtp_send",
            "Send Mail",
            "Sends an email via SMTP using a cached connection",
            "Email/SMTP",
        );
        node.set_flowscript_name("smtp", "send");
        node.set_receiver("connection");
        node.add_icon("/flow/icons/mail.svg");

        node.add_input_pin("exec_in", "In", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "connection",
            "Connection",
            "SMTP connection handle",
            VariableType::Struct,
        )
        .set_schema::<SmtpConnection>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "from",
            "From",
            "From header (single address)",
            VariableType::String,
        );
        node.add_input_pin("to", "To", "Comma-separated list", VariableType::String);
        node.add_input_pin("cc", "Cc", "Comma-separated list", VariableType::String)
            .set_default_value(Some(json!("")));
        node.add_input_pin("bcc", "Bcc", "Comma-separated list", VariableType::String)
            .set_default_value(Some(json!("")));
        node.add_input_pin("subject", "Subject", "Subject line", VariableType::String)
            .set_default_value(Some(json!("(no subject)")));
        node.add_input_pin(
            "body_text",
            "Body (text)",
            "Plaintext body",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "body_html",
            "Body (HTML)",
            "Optional HTML body",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        // Attachments
        node.add_input_pin(
            "attachments",
            "Attachments",
            "Files to attach",
            VariableType::Struct,
        )
        .set_value_type(flow_like::flow::pin::ValueType::Array)
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build())
        .set_default_value(Some(json!([])));

        // Options
        node.add_input_pin(
            "include_bcc_header",
            "Include Bcc Header",
            "If true, the Bcc header is included in the message; otherwise it's omitted (recipients still receive).",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        // Outputs
        node.add_output_pin(
            "exec_out",
            "Out",
            "Execution output",
            VariableType::Execution,
        );
        node.add_output_pin(
            "message_id",
            "Message-ID",
            "The generated Message-ID",
            VariableType::String,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let connection: SmtpConnection = context.evaluate_pin("connection").await?;
        let from: String = context.evaluate_pin("from").await?;
        let to: String = context.evaluate_pin("to").await?;
        let cc: String = context.evaluate_pin("cc").await?;
        let bcc: String = context.evaluate_pin("bcc").await?;
        let subject: String = context.evaluate_pin("subject").await?;
        let body_text: String = context.evaluate_pin("body_text").await?;
        let body_html: String = context.evaluate_pin("body_html").await?;
        let include_bcc_header: bool = context.evaluate_pin("include_bcc_header").await?;

        let in_attachments = context.evaluate_pin::<Vec<FlowPath>>("attachments").await?;
        let mut attachments = Vec::new();
        for flow_path in in_attachments {
            let content: Vec<u8> = flow_path.get(context, false).await?;
            let filename = flow_path
                .path
                .split('/')
                .next_back()
                .unwrap_or("attachment")
                .to_string();
            attachments.push((filename, content));
        }

        let mail_from = parse_first_address(&from)
            .ok_or_else(|| anyhow!("'From' must contain a valid email address"))?;
        let mut rcpts = Vec::<String>::new();
        rcpts.extend(parse_address_list(&to));
        rcpts.extend(parse_address_list(&cc));
        rcpts.extend(parse_address_list(&bcc));

        if rcpts.is_empty() {
            return Err(anyhow!(
                "No recipients provided (fill at least one of To, Cc, or Bcc)"
            ));
        }

        let message_id = generate_message_id(&from);
        let message = build_rfc5322_message_send(
            &from,
            &to,
            &cc,
            if include_bcc_header { &bcc } else { "" },
            &subject,
            &body_text,
            &body_html,
            &message_id,
            &attachments,
        );

        let session = connection.to_session(context).await?;
        let mut transport = session.lock().await;

        let mail_from_addr = mail_from
            .parse()
            .map_err(|e| anyhow!("Invalid from address '{}': {}", mail_from, e))?;

        let rcpt_addrs: Result<Vec<_>, _> = rcpts
            .iter()
            .map(|addr| {
                addr.parse()
                    .map_err(|e| anyhow!("Invalid recipient address '{}': {}", addr, e))
            })
            .collect();
        let rcpt_addrs = rcpt_addrs?;

        let envelope = Envelope::new(Some(mail_from_addr), rcpt_addrs)
            .map_err(|e| anyhow!("Failed to create envelope: {}", e))?;
        let sendable_mail = SendableEmail::new(envelope, message.as_bytes().to_vec());

        let _accepted = transport
            .send(sendable_mail)
            .await
            .map_err(|e| anyhow!("SMTP send failed: {}", e))?;

        context.log_message(
            &format!(
                "SMTP sent '{}' (Message-ID: {}) to {} recipient(s) with {} attachment(s)",
                subject,
                message_id,
                rcpts.len(),
                attachments.len()
            ),
            LogLevel::Debug,
        );

        context
            .set_pin_value("message_id", json!(message_id))
            .await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Web functionality requires the 'execute' feature"
        ))
    }
}

#[cfg(feature = "execute")]
#[allow(clippy::too_many_arguments)]
pub fn build_rfc5322_message_send(
    from: &str,
    to: &str,
    cc: &str,
    bcc_header: &str,
    subject: &str,
    body_text: &str,
    body_html: &str,
    message_id: &str,
    attachments: &[(String, Vec<u8>)],
) -> String {
    let crlf = "\r\n";
    let mut headers = Vec::new();

    if !from.is_empty() {
        headers.push(format!("From: {}", from));
    }
    if !to.is_empty() {
        headers.push(format!("To: {}", to));
    }
    if !cc.is_empty() {
        headers.push(format!("Cc: {}", cc));
    }
    if !bcc_header.is_empty() {
        headers.push(format!("Bcc: {}", bcc_header));
    }
    headers.push(format!("Subject: {}", subject));
    headers.push(format!("Message-ID: {}", message_id));
    headers.push(format!("Date: {}", rfc2822_now()));
    headers.push("MIME-Version: 1.0".to_string());

    let mut message = String::new();

    if attachments.is_empty() {
        if body_html.trim().is_empty() {
            headers.push("Content-Type: text/plain; charset=utf-8".to_string());
            headers.push("Content-Transfer-Encoding: 8bit".to_string());
            message.push_str(&headers.join(crlf));
            message.push_str(crlf);
            message.push_str(crlf);
            message.push_str(body_text);
            message.push_str(crlf);
            message.push_str(crlf);
            message.push_str(&generate_mail_footer_plain());
            message.push_str(crlf);
        } else {
            let alt_boundary = "----=_FlowLikeBoundary_mpart_alternative_001";
            headers.push(format!(
                "Content-Type: multipart/alternative; boundary=\"{}\"",
                alt_boundary
            ));

            message.push_str(&headers.join(crlf));
            message.push_str(crlf);
            message.push_str(crlf);

            message.push_str(&format!("--{}{}", alt_boundary, crlf));
            message.push_str("Content-Type: text/plain; charset=utf-8");
            message.push_str(crlf);
            message.push_str("Content-Transfer-Encoding: 8bit");
            message.push_str(crlf);
            message.push_str(crlf);
            message.push_str(body_text);
            message.push_str(crlf);
            message.push_str(&generate_mail_footer_plain());
            message.push_str(crlf);

            message.push_str(&format!("--{}{}", alt_boundary, crlf));
            message.push_str("Content-Type: text/html; charset=utf-8");
            message.push_str(crlf);
            message.push_str("Content-Transfer-Encoding: 8bit");
            message.push_str(crlf);
            message.push_str(crlf);
            message.push_str(body_html);
            message.push_str(&generate_mail_footer_html());
            message.push_str(crlf);

            message.push_str(&format!("--{}--{}", alt_boundary, crlf));
        }
    } else {
        let mixed_boundary = "----=_FlowLikeBoundary_mpart_mixed_001";
        headers.push(format!(
            "Content-Type: multipart/mixed; boundary=\"{}\"",
            mixed_boundary
        ));
        message.push_str(&headers.join(crlf));
        message.push_str(crlf);
        message.push_str(crlf);

        message.push_str(&format!("--{}{}", mixed_boundary, crlf));

        if body_html.trim().is_empty() {
            message.push_str("Content-Type: text/plain; charset=utf-8");
            message.push_str(crlf);
            message.push_str("Content-Transfer-Encoding: 8bit");
            message.push_str(crlf);
            message.push_str(crlf);
            message.push_str(body_text);
            message.push_str(crlf);
            message.push_str(crlf);
            message.push_str(&generate_mail_footer_plain());
            message.push_str(crlf);
        } else {
            let alt_boundary = "----=_FlowLikeBoundary_mpart_alternative_001";
            message.push_str(&format!(
                "Content-Type: multipart/alternative; boundary=\"{}\"",
                alt_boundary
            ));
            message.push_str(crlf);
            message.push_str(crlf);

            message.push_str(&format!("--{}{}", alt_boundary, crlf));
            message.push_str("Content-Type: text/plain; charset=utf-8");
            message.push_str(crlf);
            message.push_str("Content-Transfer-Encoding: 8bit");
            message.push_str(crlf);
            message.push_str(crlf);
            message.push_str(body_text);
            message.push_str(crlf);

            message.push_str(&format!("--{}{}", alt_boundary, crlf));
            message.push_str("Content-Type: text/html; charset=utf-8");
            message.push_str(crlf);
            message.push_str("Content-Transfer-Encoding: 8bit");
            message.push_str(crlf);
            message.push_str(crlf);
            message.push_str(body_html);
            message.push_str(&generate_mail_footer_html());
            message.push_str(crlf);

            message.push_str(&format!("--{}--{}", alt_boundary, crlf));
        }

        for (filename, content) in attachments {
            message.push_str(&format!("--{}{}", mixed_boundary, crlf));

            let mime_type = detect_mime_type(filename, content);
            message.push_str(&format!("Content-Type: {}", mime_type));
            message.push_str(crlf);
            message.push_str("Content-Transfer-Encoding: base64");
            message.push_str(crlf);
            message.push_str(&format!(
                "Content-Disposition: attachment; filename=\"{}\"",
                sanitize_filename(filename)
            ));
            message.push_str(crlf);
            message.push_str(crlf);

            let encoded = base64_encode(content);
            for chunk in encoded.as_bytes().chunks(76) {
                message.push_str(&String::from_utf8_lossy(chunk));
                message.push_str(crlf);
            }
        }

        message.push_str(&format!("--{}--{}", mixed_boundary, crlf));
    }

    message
}

#[cfg(feature = "execute")]
fn detect_mime_type(filename: &str, _content: &[u8]) -> String {
    let extension = filename.split('.').next_back().unwrap_or("").to_lowercase();
    match extension.as_str() {
        "pdf" => "application/pdf",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "txt" => "text/plain",
        "html" | "htm" => "text/html",
        "csv" => "text/csv",
        "json" => "application/json",
        "xml" => "application/xml",
        "zip" => "application/zip",
        "tar" => "application/x-tar",
        "gz" => "application/gzip",
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "mp4" => "video/mp4",
        "avi" => "video/x-msvideo",
        "mov" => "video/quicktime",
        _ => "application/octet-stream",
    }
    .to_string()
}

#[cfg(feature = "execute")]
fn sanitize_filename(filename: &str) -> String {
    filename.replace(['"', '\\', '\r', '\n'], "_")
}

#[cfg(feature = "execute")]
fn parse_first_address(input: &str) -> Option<String> {
    let mut list = parse_address_list(input);
    if list.is_empty() {
        None
    } else {
        Some(list.remove(0))
    }
}

#[cfg(feature = "execute")]
fn parse_address_list(input: &str) -> Vec<String> {
    input
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|token| {
            // Prefer address inside angle brackets if present
            if let (Some(start), Some(end)) = (token.find('<'), token.find('>')) {
                token[start + 1..end].trim().to_string()
            } else {
                token.to_string()
            }
        })
        .collect()
}

#[cfg(feature = "execute")]
pub fn generate_message_id(from: &str) -> String {
    use std::time::SystemTime;
    let domain = from.split('@').nth(1).unwrap_or("flow-like.local");
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    format!(
        "<flowlike-{}-{}@{}>",
        now.as_secs(),
        now.subsec_nanos(),
        domain
    )
}

#[cfg(feature = "execute")]
fn rfc2822_now() -> String {
    use chrono::{DateTime, FixedOffset, Local};
    // Format like: Tue, 01 Jul 2003 10:52:37 +0200
    let now_local: DateTime<Local> = Local::now();
    let offset: FixedOffset = now_local.offset().fix();
    now_local.with_timezone(&offset).to_rfc2822()
}

#[cfg(feature = "execute")]
fn base64_encode(data: &[u8]) -> String {
    use base64::{Engine as _, engine::general_purpose};
    general_purpose::STANDARD.encode(data)
}

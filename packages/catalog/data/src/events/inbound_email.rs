use flow_like::flow::{
    execution::{ExecutionEnvironment, context::ExecutionContext},
    node::{Node, NodeLogic, NodeScores},
    pin::ValueType,
    variable::VariableType,
};
use flow_like_catalog_core::{
    FlowPath, InboundEmail, InboundEmailAddress, InboundEmailAttachment, MailMessageRef,
    MailSession,
};
use flow_like_types::{
    Value, anyhow, async_trait,
    json::{self, json},
};

#[crate::register_node]
#[derive(Default)]
pub struct InboundEmailEventNode;

impl InboundEmailEventNode {
    pub fn new() -> Self {
        Self
    }
}

fn email_from_payload(payload: Option<&Value>) -> flow_like_types::Result<InboundEmail> {
    let value = payload
        .and_then(|payload| payload.get("email"))
        .filter(|email| email.is_object())
        .cloned()
        .ok_or_else(|| anyhow!("Inbound email payload must contain an email object"))?;
    let mut email: InboundEmail = json::from_value(value)
        .map_err(|error| anyhow!("Invalid inbound email payload: {error}"))?;
    if email.sender.is_none() {
        email.sender = email.from.first().cloned();
    }
    Ok(email)
}

fn resolve_mail_references(
    email: &mut InboundEmail,
    app_id: &str,
    event_id: &str,
) -> flow_like_types::Result<()> {
    if app_id.is_empty() || event_id.is_empty() || email.id.is_empty() {
        return Err(anyhow!(
            "Inbound email requires app, Event and delivery identifiers"
        ));
    }
    let session = MailSession {
        app_id: app_id.to_owned(),
        event_id: event_id.to_owned(),
    };
    let reference = MailMessageRef {
        session: session.clone(),
        delivery_id: email.id.clone(),
    };
    if email
        .session
        .as_ref()
        .is_some_and(|supplied| supplied != &session)
        || email
            .reference
            .as_ref()
            .is_some_and(|supplied| supplied != &reference)
    {
        return Err(anyhow!(
            "Inbound email references do not match this Event and delivery"
        ));
    }
    email.session = Some(session);
    email.reference = Some(reference);
    Ok(())
}

fn email_outputs(email: &InboundEmail) -> flow_like_types::Result<[(&'static str, Value); 22]> {
    let session = email
        .session
        .as_ref()
        .ok_or_else(|| anyhow!("Missing mail session"))?;
    let reference = email
        .reference
        .as_ref()
        .ok_or_else(|| anyhow!("Missing mail message reference"))?;
    Ok([
        ("email", json!(email)),
        ("session", json!(session)),
        ("message", json!(reference)),
        (
            "attachments",
            json!(
                email
                    .attachments
                    .iter()
                    .map(|part| &part.path)
                    .collect::<Vec<_>>()
            ),
        ),
        ("attachment_metadata", json!(email.attachments)),
        ("sender", json!(email.sender)),
        ("from", json!(email.from)),
        ("to", json!(email.to)),
        ("cc", json!(email.cc)),
        ("reply_to", json!(email.reply_to)),
        (
            "subject",
            json!(email.subject.as_deref().unwrap_or_default()),
        ),
        ("text", json!(email.text.as_deref().unwrap_or_default())),
        ("html", json!(email.html.as_deref().unwrap_or_default())),
        ("envelope_from", json!(email.envelope_from)),
        ("recipient", json!(email.recipient)),
        ("automated", json!(email.automated)),
        ("raw", json!(email.raw_path)),
        ("text_path", json!(email.text_path)),
        ("html_path", json!(email.html_path)),
        ("expires_at", json!(email.expires_at)),
        ("text_truncated", json!(email.text_truncated)),
        ("html_truncated", json!(email.html_truncated)),
    ])
}

#[async_trait]
impl NodeLogic for InboundEmailEventNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "events_inbound_email",
            "Inbound Email Event",
            "Run a server workflow when an email arrives at its assigned address",
            "Events",
        );
        node.set_flowscript_name("events", "inboundEmail");
        node.add_icon("/flow/icons/event.svg");
        node.set_start(true);
        node.set_version(2);
        node.set_scores(
            NodeScores::new()
                .set_privacy(4)
                .set_security(5)
                .set_performance(8)
                .set_governance(7)
                .set_reliability(7)
                .set_cost(8)
                .build(),
        );
        node.add_output_pin(
            "exec_out",
            "Output",
            "Run when an email is received",
            VariableType::Execution,
        );
        node.add_output_pin(
            "email",
            "Email",
            "Email content, envelope recipients, headers and temporary file paths that expire at Expires At",
            VariableType::Struct,
        )
        .set_schema::<InboundEmail>();
        node.add_output_pin(
            "session",
            "Mail Session",
            "App and Event reference for sending through this automation address",
            VariableType::Struct,
        )
        .set_schema::<MailSession>();
        node.add_output_pin(
            "message",
            "Mail Message",
            "Original inbound message reference for replies",
            VariableType::Struct,
        )
        .set_schema::<MailMessageRef>();
        node.add_output_pin(
            "attachments",
            "Attachments",
            "Temporary attachment files for file and path nodes. Copy them to app storage to keep them",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_value_type(ValueType::Array);
        node.add_output_pin(
            "attachment_metadata",
            "Attachment Metadata",
            "Attachment filenames, content types, byte sizes and temporary file paths",
            VariableType::Struct,
        )
        .set_schema::<InboundEmailAttachment>()
        .set_value_type(ValueType::Array);
        node.add_output_pin(
            "sender",
            "Sender",
            "Sender header, falling back to the first From address",
            VariableType::Struct,
        )
        .set_schema::<Option<InboundEmailAddress>>();
        for (name, label, description) in [
            ("from", "From", "From header addresses"),
            ("to", "To", "To header addresses"),
            ("cc", "Cc", "Carbon copy header addresses"),
            ("reply_to", "Reply To", "Addresses for replies"),
        ] {
            node.add_output_pin(name, label, description, VariableType::Struct)
                .set_schema::<InboundEmailAddress>()
                .set_value_type(ValueType::Array);
        }
        for (name, label, description) in [
            ("subject", "Subject", "Email subject, empty when absent"),
            ("text", "Text", "Plain text preview, empty when absent"),
            ("html", "HTML", "HTML preview, empty when absent"),
            ("envelope_from", "Envelope From", "SMTP envelope sender"),
            ("recipient", "Recipient", "The receiving automation address"),
        ] {
            node.add_output_pin(name, label, description, VariableType::String);
        }
        node.add_output_pin(
            "automated",
            "Automated",
            "True for bounces, auto-replies, mailing lists and other machine-generated mail. Platform replies to it are refused to prevent mail loops",
            VariableType::Boolean,
        );
        node.add_output_pin(
            "raw",
            "Raw Email",
            "Complete MIME message as a temporary EML file. Copy it to app storage to keep it",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>();
        for (name, label, description) in [
            (
                "text_path",
                "Text File",
                "Complete plain text body as a temporary file, when present",
            ),
            (
                "html_path",
                "HTML File",
                "Complete HTML body as a temporary file, when present",
            ),
        ] {
            node.add_output_pin(name, label, description, VariableType::Struct)
                .set_schema::<Option<FlowPath>>();
        }
        node.add_output_pin(
            "expires_at",
            "Expires At",
            "When the email's stored files are deleted. Copy files to app storage before then to keep them",
            VariableType::Date,
        );
        for (name, label, description) in [
            (
                "text_truncated",
                "Text Truncated",
                "Read Text File for the complete plain text body",
            ),
            (
                "html_truncated",
                "HTML Truncated",
                "Read HTML File for the complete HTML body",
            ),
        ] {
            node.add_output_pin(name, label, description, VariableType::Boolean);
        }
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        if context.execution_environment() != ExecutionEnvironment::Server {
            return Err(anyhow!("Inbound email Events only run on the server"));
        }
        let mut email = email_from_payload(context.get_payload().await?.payload.as_ref())?;
        let app_id = context
            .execution_cache
            .as_ref()
            .map(|cache| cache.app_id.clone())
            .ok_or_else(|| anyhow!("Inbound email requires an app execution"))?;
        let event_id = context
            .event_id()
            .await
            .ok_or_else(|| anyhow!("Inbound email requires an Event execution"))?;
        resolve_mail_references(&mut email, &app_id, &event_id)?;
        for (name, value) in email_outputs(&email)? {
            context.set_pin_value(name, value).await?;
        }
        let exec_out = context.get_pin_by_name("exec_out").await?;
        context.activate_exec_pin_ref(&exec_out).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like::flow::{
        board::cleanup::sync_node_schema::sync_node_with_catalog,
        pin::{Pin, PinType},
    };
    use flow_like_types::dispatch::REQUEST_FILES_STORE_REF;

    fn payload() -> Value {
        let file = |path: &str| json!({"path": path, "store_ref": REQUEST_FILES_STORE_REF});
        json!({"email": {
            "id": "mail-1", "delivery_id": "provider-1",
            "envelope_from": "bounce@example.com", "recipient": "orders@example.com",
            "session": {"app_id": "app-1", "event_id": "event-1"},
            "reference": {"session": {"app_id": "app-1", "event_id": "event-1"}, "delivery_id": "mail-1"},
            "from": [{"name": "Author", "email": "author@example.com"}],
            "to": [{"email": "displayed@example.com"}],
            "subject": "Order", "text": "New order", "text_truncated": true,
            "text_path": file("tmp/mail/body.txt"),
            "attachments": [{"filename": "invoice.pdf", "content_type": "application/pdf", "size": 42,
                "path": file("tmp/mail/attachments/0/invoice.pdf")}],
            "raw_path": file("tmp/mail/raw.eml"),
            "automated": true,
            "expires_at": "2026-09-29T10:00:00+00:00",
            "authentication": {"provider": "ses", "dkim": {"status": "PASS"}}
        }})
    }

    #[test]
    fn exposes_paths_and_metadata_that_match_every_output_schema() {
        let email = email_from_payload(Some(&payload())).unwrap();
        let node = InboundEmailEventNode.get_node();
        let values: std::collections::HashMap<_, _> =
            email_outputs(&email).unwrap().into_iter().collect();
        let attachments: Vec<FlowPath> = json::from_value(values["attachments"].clone()).unwrap();
        assert_eq!(attachments[0].path, "tmp/mail/attachments/0/invoice.pdf");
        assert_eq!(attachments[0].store_ref, REQUEST_FILES_STORE_REF);
        let reader_paths: Vec<flow_like_catalog_data_support::data::path::FlowPath> =
            json::from_value(values["attachments"].clone()).unwrap();
        assert_eq!(reader_paths[0].path, attachments[0].path);
        assert_eq!(reader_paths[0].store_ref, attachments[0].store_ref);
        assert_eq!(values["attachment_metadata"][0]["filename"], "invoice.pdf");
        assert_eq!(values["email"]["authentication"]["dkim"]["status"], "PASS");
        assert_eq!(values["recipient"], "orders@example.com");
        assert_eq!(values["to"][0]["email"], "displayed@example.com");
        assert_eq!(values["text_truncated"], true);
        assert_eq!(values["html_truncated"], false);
        assert_eq!(values["text_path"]["path"], "tmp/mail/body.txt");
        assert_eq!(values["automated"], true);
        assert_eq!(values["expires_at"], "2026-09-29T10:00:00+00:00");
        assert_eq!(values["html"], "");
        assert!(values["html_path"].is_null());
        assert_eq!(values["sender"]["email"], "author@example.com");
        assert_eq!(values["session"], values["email"]["session"]);
        assert_eq!(values["message"], values["email"]["reference"]);
        assert_eq!(values["message"]["delivery_id"], "mail-1");

        for pin in node.pins.values().filter(|pin| pin.name != "exec_out") {
            let value = &values[pin.name.as_str()];
            assert_eq!(pin.pin_type, PinType::Output);
            if let Some(schema) = &pin.schema {
                let schema: Value = json::from_str(schema).unwrap();
                let validator = jsonschema::validator_for(&schema).unwrap();
                if pin.value_type == ValueType::Array {
                    for item in value.as_array().unwrap() {
                        assert!(validator.is_valid(item), "Invalid {} item", pin.name);
                    }
                } else {
                    assert!(validator.is_valid(value), "Invalid {} output", pin.name);
                }
            } else {
                match pin.data_type {
                    VariableType::String | VariableType::Date => assert!(value.is_string()),
                    VariableType::Boolean => assert!(value.is_boolean()),
                    _ => panic!("Untyped {} output", pin.name),
                }
            }
        }
    }

    #[test]
    fn preserves_explicit_sender_and_defaults_missing_content() {
        let mut payload = payload();
        payload["email"]["sender"] = json!({"name": "Agent", "email": "agent@example.com"});
        for field in ["subject", "text", "from", "attachments", "automated"] {
            payload["email"].as_object_mut().unwrap().remove(field);
        }
        let email = email_from_payload(Some(&payload)).unwrap();
        let values: std::collections::HashMap<_, _> =
            email_outputs(&email).unwrap().into_iter().collect();
        assert_eq!(values["sender"]["email"], "agent@example.com");
        assert_eq!(values["email"]["sender"], values["sender"]);
        assert_eq!(values["subject"], "");
        assert_eq!(values["text"], "");
        assert_eq!(values["from"], json!([]));
        assert_eq!(values["attachments"], json!([]));
        assert_eq!(values["automated"], false);
        payload["email"].as_object_mut().unwrap().remove("sender");
        assert!(email_from_payload(Some(&payload)).unwrap().sender.is_none());
    }

    #[test]
    fn catalog_pins_use_native_schema_identity_and_array_types() {
        let node = InboundEmailEventNode.get_node();
        assert_eq!(node.version, Some(2));
        assert!(node.scores.is_some());
        for (name, data_type) in [
            ("automated", VariableType::Boolean),
            ("expires_at", VariableType::Date),
        ] {
            assert_eq!(node.get_pin_by_name(name).unwrap().data_type, data_type);
        }
        assert_eq!(
            Pin::schema_string_for::<FlowPath>(),
            Pin::schema_string_for::<flow_like_catalog_data_support::data::path::FlowPath>()
        );
        assert_eq!(
            node.get_pin_by_name("email").unwrap().schema,
            Pin::schema_string_for::<InboundEmail>()
        );
        assert_eq!(
            node.get_pin_by_name("session").unwrap().schema,
            Pin::schema_string_for::<MailSession>()
        );
        assert_eq!(
            node.get_pin_by_name("message").unwrap().schema,
            Pin::schema_string_for::<MailMessageRef>()
        );
        for name in ["attachments", "raw"] {
            assert_eq!(
                node.get_pin_by_name(name).unwrap().schema,
                Pin::schema_string_for::<FlowPath>()
            );
        }
        for name in ["text_path", "html_path"] {
            assert_eq!(
                node.get_pin_by_name(name).unwrap().schema,
                Pin::schema_string_for::<Option<FlowPath>>()
            );
        }
        assert_eq!(
            node.get_pin_by_name("sender").unwrap().schema,
            Pin::schema_string_for::<Option<InboundEmailAddress>>()
        );
        assert_eq!(
            node.get_pin_by_name("attachment_metadata").unwrap().schema,
            Pin::schema_string_for::<InboundEmailAttachment>()
        );
        for name in ["from", "to", "cc", "reply_to"] {
            assert_eq!(
                node.get_pin_by_name(name).unwrap().schema,
                Pin::schema_string_for::<InboundEmailAddress>()
            );
        }
        for name in [
            "attachments",
            "attachment_metadata",
            "from",
            "to",
            "cc",
            "reply_to",
        ] {
            assert_eq!(
                node.get_pin_by_name(name).unwrap().value_type,
                ValueType::Array
            );
        }
    }

    #[test]
    fn catalog_upgrade_preserves_existing_email_and_execution_wires() {
        let mut placed = Node::new("events_inbound_email", "Inbound Email Event", "", "Events");
        placed
            .add_output_pin("exec_out", "Output", "", VariableType::Execution)
            .connected_to
            .insert("next-exec".into());
        placed
            .add_output_pin("email", "Email", "", VariableType::Struct)
            .set_open_schema()
            .connected_to
            .insert("break-email".into());
        let email_id = placed.get_pin_by_name("email").unwrap().id.clone();
        let exec_id = placed.get_pin_by_name("exec_out").unwrap().id.clone();
        sync_node_with_catalog(&mut placed, &InboundEmailEventNode.get_node());
        assert_eq!(placed.version, Some(2));
        let email = placed.get_pin_by_name("email").unwrap();
        assert_eq!(email.id, email_id);
        assert!(email.connected_to.contains("break-email"));
        assert!(!email.has_open_schema());
        let exec = placed.get_pin_by_name("exec_out").unwrap();
        assert_eq!(exec.id, exec_id);
        assert!(exec.connected_to.contains("next-exec"));
        assert!(placed.get_pin_by_name("attachments").is_some());
    }

    #[test]
    fn missing_or_malformed_email_payload_is_rejected() {
        assert!(email_from_payload(None).is_err());
        for payload in [
            json!({}),
            json!({"email": null}),
            json!({"email": "text"}),
            json!({"email": {}}),
        ] {
            assert!(email_from_payload(Some(&payload)).is_err());
        }
    }

    #[test]
    fn legacy_message_references_use_current_event_and_internal_delivery_id() {
        let mut payload = payload();
        payload["email"].as_object_mut().unwrap().remove("session");
        payload["email"]
            .as_object_mut()
            .unwrap()
            .remove("reference");
        let mut email = email_from_payload(Some(&payload)).unwrap();
        assert!(email_outputs(&email).is_err());
        resolve_mail_references(&mut email, "app-1", "event-1").unwrap();
        let reference = email.reference.as_ref().unwrap();
        assert_eq!(reference.delivery_id, email.id);
        assert_ne!(reference.delivery_id, email.delivery_id);
        assert_eq!(reference.session.app_id, "app-1");
        assert_eq!(reference.session.event_id, "event-1");
        assert!(email_outputs(&email).is_ok());
    }

    #[test]
    fn rejects_references_for_a_different_app_event_or_delivery() {
        for (app_id, event_id) in [("other", "event-1"), ("app-1", "other"), ("", "event-1")] {
            let mut email = email_from_payload(Some(&payload())).unwrap();
            assert!(resolve_mail_references(&mut email, app_id, event_id).is_err());
        }
        let mut email = email_from_payload(Some(&payload())).unwrap();
        email.reference.as_mut().unwrap().delivery_id = email.delivery_id.clone();
        assert!(resolve_mail_references(&mut email, "app-1", "event-1").is_err());
        let mut email = email_from_payload(Some(&payload())).unwrap();
        email.reference.as_mut().unwrap().session.event_id = "other".into();
        assert!(resolve_mail_references(&mut email, "app-1", "event-1").is_err());
    }
}

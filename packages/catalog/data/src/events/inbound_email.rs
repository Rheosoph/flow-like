use flow_like::flow::{
    execution::{ExecutionEnvironment, context::ExecutionContext},
    node::{Node, NodeLogic, NodeScores},
    pin::ValueType,
    variable::VariableType,
};
use flow_like_catalog_core::{
    FlowPath, InboundEmail, InboundEmailAddress, InboundEmailAttachment,
    InboundEmailAuthentication, InboundEmailHeader, MailMessageRef, MailSession,
};
use flow_like_types::{
    Value, anyhow, async_trait,
    json::{self, json},
};
use schemars::JsonSchema;
use serde::Serialize;

#[crate::register_node]
#[derive(Default)]
pub struct InboundEmailEventNode;

impl InboundEmailEventNode {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Serialize, JsonSchema)]
struct InboundEmailAddresses {
    /// Sender header, falling back to the first From address
    sender: Option<InboundEmailAddress>,
    from: Vec<InboundEmailAddress>,
    to: Vec<InboundEmailAddress>,
    cc: Vec<InboundEmailAddress>,
    reply_to: Vec<InboundEmailAddress>,
    /// SMTP envelope sender
    envelope_from: String,
    /// The automation address that received the email
    recipient: String,
}

#[derive(Serialize, JsonSchema)]
struct InboundEmailContent {
    subject: String,
    text: String,
    html: String,
    /// Text is a preview. Read text_path for the complete plain text body
    text_truncated: bool,
    /// HTML is a preview. Read html_path for the complete HTML body
    html_truncated: bool,
    text_path: Option<FlowPath>,
    html_path: Option<FlowPath>,
}

#[derive(Serialize, JsonSchema)]
struct InboundEmailDelivery {
    /// Bounce, auto-reply, mailing list or other machine-generated mail. Platform replies to it are refused to prevent mail loops
    automated: bool,
    /// When the raw, body and attachment files are deleted
    #[schemars(extend("format" = "date-time"))]
    expires_at: Option<String>,
    /// Complete MIME message as a temporary EML file
    raw_path: FlowPath,
    /// Attachments beyond the limit of 100 that are only contained in the raw EML file
    omitted_attachments: u32,
    #[schemars(extend("format" = "date-time"))]
    received_at: Option<String>,
    message_id: Option<String>,
    /// The mail provider's delivery ID
    provider_delivery_id: String,
    headers: Vec<InboundEmailHeader>,
    authentication: Option<InboundEmailAuthentication>,
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

fn email_outputs(email: InboundEmail) -> flow_like_types::Result<[(&'static str, Value); 6]> {
    let InboundEmail {
        id: _,
        delivery_id,
        envelope_from,
        recipient,
        session,
        reference,
        sender,
        from,
        to,
        cc,
        reply_to,
        subject,
        message_id,
        text,
        html,
        text_truncated,
        html_truncated,
        text_path,
        html_path,
        headers,
        attachments,
        omitted_attachments,
        raw_path,
        received_at,
        expires_at,
        authentication,
        automated,
    } = email;
    let session = session.ok_or_else(|| anyhow!("Missing mail session"))?;
    let reference = reference.ok_or_else(|| anyhow!("Missing mail message reference"))?;
    Ok([
        ("message", json!(reference)),
        ("session", json!(session)),
        (
            "addresses",
            json!(InboundEmailAddresses {
                sender,
                from,
                to,
                cc,
                reply_to,
                envelope_from,
                recipient,
            }),
        ),
        (
            "content",
            json!(InboundEmailContent {
                subject: subject.unwrap_or_default(),
                text: text.unwrap_or_default(),
                html: html.unwrap_or_default(),
                text_truncated,
                html_truncated,
                text_path,
                html_path,
            }),
        ),
        ("attachments", json!(attachments)),
        (
            "delivery",
            json!(InboundEmailDelivery {
                automated,
                expires_at,
                raw_path,
                omitted_attachments,
                received_at,
                message_id,
                provider_delivery_id: delivery_id,
                headers,
                authentication,
            }),
        ),
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
        node.set_version(4);
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
            "message",
            "Message",
            "Reference to this email for Reply Platform Email",
            VariableType::Struct,
        )
        .set_schema::<MailMessageRef>();
        node.add_output_pin(
            "session",
            "Mail Session",
            "App and Event reference for sending new mail from this automation address",
            VariableType::Struct,
        )
        .set_schema::<MailSession>();
        node.add_output_pin(
            "addresses",
            "Addresses",
            "Sender, header recipients and SMTP envelope addresses",
            VariableType::Struct,
        )
        .set_schema::<InboundEmailAddresses>();
        node.add_output_pin(
            "content",
            "Content",
            "Subject and body previews, plus temporary files with the complete bodies",
            VariableType::Struct,
        )
        .set_schema::<InboundEmailContent>();
        node.add_output_pin(
            "attachments",
            "Attachments",
            "Attachment filenames, content types, byte sizes and temporary files. Embedded is true for parts the HTML body shows through cid:<content_id>, such as signature logos, which are rarely real attachments. Copy files to app storage to keep them",
            VariableType::Struct,
        )
        .set_schema::<InboundEmailAttachment>()
        .set_value_type(ValueType::Array);
        node.add_output_pin(
            "delivery",
            "Delivery",
            "Machine-generated flag, file expiry, raw EML file, headers and authentication verdicts",
            VariableType::Struct,
        )
        .set_schema::<InboundEmailDelivery>();
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
        for (name, value) in email_outputs(email)? {
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
    use flow_like::flow::{board::cleanup::sync_node_schema::sync_node_with_catalog, pin::Pin};
    use flow_like_types::dispatch::REQUEST_FILES_STORE_REF;
    use std::collections::HashMap;

    const DATA_PINS: [&str; 6] = [
        "message",
        "session",
        "addresses",
        "content",
        "attachments",
        "delivery",
    ];

    fn payload() -> Value {
        let file = |path: &str| json!({"path": path, "store_ref": REQUEST_FILES_STORE_REF});
        json!({"email": {
            "id": "mail-1", "delivery_id": "provider-1",
            "envelope_from": "bounce@example.com", "recipient": "orders@example.com",
            "session": {"app_id": "app-1", "event_id": "event-1"},
            "reference": {"session": {"app_id": "app-1", "event_id": "event-1"}, "delivery_id": "mail-1"},
            "from": [{"name": "Author", "email": "author@example.com"}],
            "to": [{"email": "displayed@example.com"}],
            "subject": "Order", "message_id": "<order@example.com>",
            "text": "New order", "text_truncated": true,
            "text_path": file("tmp/mail/body.txt"),
            "headers": [{"name": "X-Priority", "value": "1"}],
            "attachments": [{"filename": "invoice.pdf", "content_type": "application/pdf", "size": 42,
                "path": file("tmp/mail/attachments/0/invoice.pdf"),
                "content_id": "logo@example.com", "disposition": "inline", "embedded": true}],
            "omitted_attachments": 2,
            "raw_path": file("tmp/mail/raw.eml"),
            "automated": true,
            "received_at": "2026-09-28T10:00:00+00:00",
            "expires_at": "2026-09-29T10:00:00+00:00",
            "authentication": {"provider": "ses", "dkim": {"status": "PASS"}}
        }})
    }

    fn outputs(payload: &Value) -> HashMap<&'static str, Value> {
        let email = email_from_payload(Some(payload)).unwrap();
        email_outputs(email).unwrap().into_iter().collect()
    }

    #[test]
    fn routes_every_payload_field_to_exactly_one_schema_valid_pin() {
        let values = outputs(&payload());
        let attachments: Vec<InboundEmailAttachment> =
            json::from_value(values["attachments"].clone()).unwrap();
        assert_eq!(attachments[0].filename.as_deref(), Some("invoice.pdf"));
        assert_eq!(
            attachments[0].path.path,
            "tmp/mail/attachments/0/invoice.pdf"
        );
        assert_eq!(attachments[0].path.store_ref, REQUEST_FILES_STORE_REF);
        assert_eq!(
            attachments[0].content_id.as_deref(),
            Some("logo@example.com")
        );
        assert_eq!(attachments[0].disposition.as_deref(), Some("inline"));
        assert!(attachments[0].embedded && attachments[0].charset.is_none());
        let reader_path: flow_like_catalog_data_support::data::path::FlowPath =
            json::from_value(values["attachments"][0]["path"].clone()).unwrap();
        assert_eq!(reader_path.path, attachments[0].path.path);

        assert_eq!(values["message"]["delivery_id"], "mail-1");
        assert_eq!(values["message"]["session"], values["session"]);
        assert_eq!(values["session"]["event_id"], "event-1");

        let addresses = &values["addresses"];
        assert_eq!(addresses["sender"]["email"], "author@example.com");
        assert_eq!(addresses["from"][0]["email"], "author@example.com");
        assert_eq!(addresses["to"][0]["email"], "displayed@example.com");
        assert_eq!(addresses["recipient"], "orders@example.com");
        assert_eq!(addresses["envelope_from"], "bounce@example.com");

        let content = &values["content"];
        assert_eq!(content["subject"], "Order");
        assert_eq!(content["text"], "New order");
        assert_eq!(content["html"], "");
        assert_eq!(content["text_truncated"], true);
        assert_eq!(content["html_truncated"], false);
        assert_eq!(content["text_path"]["path"], "tmp/mail/body.txt");
        assert!(content["html_path"].is_null());

        let delivery = &values["delivery"];
        assert_eq!(delivery["automated"], true);
        assert_eq!(delivery["expires_at"], "2026-09-29T10:00:00+00:00");
        assert_eq!(delivery["received_at"], "2026-09-28T10:00:00+00:00");
        assert_eq!(delivery["raw_path"]["path"], "tmp/mail/raw.eml");
        assert_eq!(delivery["message_id"], "<order@example.com>");
        assert_eq!(delivery["provider_delivery_id"], "provider-1");
        assert_eq!(delivery["headers"][0]["name"], "X-Priority");
        assert_eq!(delivery["authentication"]["dkim"]["status"], "PASS");
        assert_eq!(delivery["omitted_attachments"], 2);

        let node = InboundEmailEventNode.get_node();
        for name in DATA_PINS {
            let pin = node.get_pin_by_name(name).unwrap();
            let schema: Value = json::from_str(pin.schema.as_deref().unwrap()).unwrap();
            let validator = jsonschema::validator_for(&schema).unwrap();
            if pin.value_type == ValueType::Array {
                for item in values[name].as_array().unwrap() {
                    assert!(validator.is_valid(item), "Invalid {name} item");
                }
            } else {
                assert!(validator.is_valid(&values[name]), "Invalid {name} output");
            }
        }
    }

    #[test]
    fn preserves_explicit_sender_and_defaults_missing_content() {
        let mut payload = payload();
        payload["email"]["sender"] = json!({"name": "Agent", "email": "agent@example.com"});
        for field in [
            "subject",
            "text",
            "from",
            "attachments",
            "automated",
            "omitted_attachments",
        ] {
            payload["email"].as_object_mut().unwrap().remove(field);
        }
        let values = outputs(&payload);
        assert_eq!(values["addresses"]["sender"]["email"], "agent@example.com");
        assert_eq!(values["addresses"]["from"], json!([]));
        assert_eq!(values["content"]["subject"], "");
        assert_eq!(values["content"]["text"], "");
        assert_eq!(values["attachments"], json!([]));
        assert_eq!(values["delivery"]["automated"], false);
        assert_eq!(values["delivery"]["omitted_attachments"], 0);
        payload["email"].as_object_mut().unwrap().remove("sender");
        assert!(email_from_payload(Some(&payload)).unwrap().sender.is_none());
    }

    #[test]
    fn catalog_exposes_one_typed_struct_per_concern() {
        let node = InboundEmailEventNode.get_node();
        assert_eq!(node.version, Some(4));
        assert!(node.scores.is_some());
        let mut names: Vec<_> = node.pins.values().map(|pin| pin.name.as_str()).collect();
        names.sort_unstable();
        let mut expected = DATA_PINS.to_vec();
        expected.push("exec_out");
        expected.sort_unstable();
        assert_eq!(names, expected);
        for name in DATA_PINS {
            let pin = node.get_pin_by_name(name).unwrap();
            assert_eq!(pin.data_type, VariableType::Struct);
            assert_eq!(
                pin.value_type == ValueType::Array,
                name == "attachments",
                "{name} value type"
            );
        }
        for (name, schema) in [
            ("message", Pin::schema_string_for::<MailMessageRef>()),
            ("session", Pin::schema_string_for::<MailSession>()),
            (
                "addresses",
                Pin::schema_string_for::<InboundEmailAddresses>(),
            ),
            ("content", Pin::schema_string_for::<InboundEmailContent>()),
            (
                "attachments",
                Pin::schema_string_for::<InboundEmailAttachment>(),
            ),
            ("delivery", Pin::schema_string_for::<InboundEmailDelivery>()),
        ] {
            assert_eq!(node.get_pin_by_name(name).unwrap().schema, schema, "{name}");
        }
        assert_eq!(
            Pin::schema_string_for::<FlowPath>(),
            Pin::schema_string_for::<flow_like_catalog_data_support::data::path::FlowPath>()
        );
    }

    #[test]
    fn catalog_upgrade_keeps_reference_wires_and_drops_unwired_flat_duplicates() {
        let mut placed = Node::new("events_inbound_email", "Inbound Email Event", "", "Events");
        placed.set_version(2);
        placed
            .add_output_pin("exec_out", "Output", "", VariableType::Execution)
            .connected_to
            .insert("next-exec".into());
        placed
            .add_output_pin("message", "Mail Message", "", VariableType::Struct)
            .set_schema::<MailMessageRef>()
            .connected_to
            .insert("reply-message".into());
        for name in ["email", "sender"] {
            placed
                .add_output_pin(name, name, "", VariableType::Struct)
                .connected_to
                .insert(format!("{name}-consumer"));
        }
        placed.add_output_pin("subject", "Subject", "", VariableType::String);
        let message_id = placed.get_pin_by_name("message").unwrap().id.clone();
        sync_node_with_catalog(&mut placed, &InboundEmailEventNode.get_node());
        assert_eq!(placed.version, Some(4));
        let message = placed.get_pin_by_name("message").unwrap();
        assert_eq!(message.id, message_id);
        assert!(message.connected_to.contains("reply-message"));
        assert!(
            placed
                .get_pin_by_name("exec_out")
                .unwrap()
                .connected_to
                .contains("next-exec")
        );
        assert!(placed.get_pin_by_name("subject").is_none());
        for name in ["email", "sender"] {
            let retired = placed.get_pin_by_name(name).unwrap();
            assert!(retired.connected_to.contains(&format!("{name}-consumer")));
        }
        for name in DATA_PINS {
            assert!(placed.get_pin_by_name(name).is_some(), "{name} missing");
        }
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
        assert!(email_outputs(email.clone()).is_err());
        resolve_mail_references(&mut email, "app-1", "event-1").unwrap();
        let reference = email.reference.as_ref().unwrap();
        assert_eq!(reference.delivery_id, email.id);
        assert_ne!(reference.delivery_id, email.delivery_id);
        assert_eq!(reference.session.app_id, "app-1");
        assert_eq!(reference.session.event_id, "event-1");
        assert!(email_outputs(email).is_ok());
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

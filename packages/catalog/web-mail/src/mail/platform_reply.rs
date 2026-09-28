use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::{MailMessageRef, MailSession};
use flow_like_types::{anyhow, async_trait, json::json};

use super::platform_send::{add_result_pins, finish, mail_scores, optional_text, submit};

#[crate::register_node]
#[derive(Default)]
pub struct PlatformReplyMailNode;

impl PlatformReplyMailNode {
    pub fn new() -> Self {
        Self
    }
}

fn validate_message(
    session: &MailSession,
    message: &MailMessageRef,
) -> flow_like_types::Result<()> {
    if message.delivery_id.is_empty() || message.session != *session {
        return Err(anyhow!(
            "The received message must belong to the supplied mail session"
        ));
    }
    Ok(())
}

#[async_trait]
impl NodeLogic for PlatformReplyMailNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "email_platform_reply",
            "Reply Platform Email",
            "Reply to a received email and preserve its thread",
            "Email",
        );
        node.set_version(1);
        node.set_flowscript_name("email", "replyPlatform");
        node.set_receiver("session");
        node.add_icon("/flow/icons/mail.svg");
        node.set_scores(mail_scores());
        node.add_input_pin(
            "exec_in",
            "In",
            "Reply to the email",
            VariableType::Execution,
        );
        node.add_input_pin(
            "session",
            "Session",
            "Mail session from an inbound email event",
            VariableType::Struct,
        )
        .set_schema::<MailSession>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "message",
            "Message",
            "Received message reference from an inbound email event",
            VariableType::Struct,
        )
        .set_schema::<MailMessageRef>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        for (name, label, description) in [
            ("text", "Text", "Plain text reply body"),
            ("html", "HTML", "HTML reply body"),
        ] {
            node.add_input_pin(name, label, description, VariableType::String)
                .set_default_value(Some(json!("")));
        }
        add_result_pins(&mut node);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let session = context.evaluate_pin::<MailSession>("session").await?;
        let message = context.evaluate_pin::<MailMessageRef>("message").await?;
        validate_message(&session, &message)?;
        let text = optional_text(context.evaluate_pin::<String>("text").await?);
        let html = optional_text(context.evaluate_pin::<String>("html").await?);
        let result = submit(
            context,
            &session,
            "reply",
            json!({"session":session,"message":message,"text":text,"html":html}),
        )
        .await?;
        finish(context, result).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reply_uses_typed_references_and_a_server_owned_envelope() {
        let node = PlatformReplyMailNode::new().get_node();
        for (name, title) in [("session", "MailSession"), ("message", "MailMessageRef")] {
            let pin = node.get_pin_by_name(name).unwrap();
            assert_eq!(pin.data_type, VariableType::Struct);
            assert_eq!(pin.options.as_ref().unwrap().enforce_schema, Some(true));
            assert!(pin.default_value.is_none());
            let schema: flow_like_types::Value =
                flow_like_types::json::from_str(pin.schema.as_deref().unwrap()).unwrap();
            assert_eq!(schema["title"], title);
        }
        for name in [
            "to",
            "cc",
            "bcc",
            "from",
            "reply_to",
            "subject",
            "headers",
            "in_reply_to",
            "references",
        ] {
            assert!(
                node.get_pin_by_name(name).is_none(),
                "Reply envelope pin {name} must be server controlled"
            );
        }
        assert!(node.get_pin_by_name("session_out").is_some());
        assert!(node.scores.is_some());
    }

    #[test]
    fn rejects_a_message_from_a_different_session() {
        let session = MailSession {
            app_id: "app-1".into(),
            event_id: "event-1".into(),
        };
        let mut message = MailMessageRef {
            session: session.clone(),
            delivery_id: "delivery-1".into(),
        };
        assert!(validate_message(&session, &message).is_ok());
        message.session.event_id = "event-2".into();
        assert!(validate_message(&session, &message).is_err());
        message.session = session.clone();
        message.delivery_id.clear();
        assert!(validate_message(&session, &message).is_err());
    }
}

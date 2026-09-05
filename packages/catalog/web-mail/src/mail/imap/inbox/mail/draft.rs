use crate::mail::imap::ImapConnection;
#[cfg(feature = "execute")]
use crate::mail::smtp::send_mail::{build_rfc5322_message_send, generate_message_id};
#[cfg(feature = "execute")]
use flow_like::flow::execution::LogLevel;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::FlowPath;
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct ImapCreateDraftNode;

impl ImapCreateDraftNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for ImapCreateDraftNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "email_imap_create_draft",
            "Create Draft",
            "Appends a new draft message to a mailbox (defaults to 'Drafts')",
            "Email/IMAP",
        );
        node.set_flowscript_name("imap", "createDraft");
        node.set_receiver("connection");
        node.add_icon("/flow/icons/mail.svg");

        node.add_input_pin("exec_in", "In", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "connection",
            "Connection",
            "IMAP connection details",
            VariableType::Struct,
        )
        .set_schema::<ImapConnection>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "mailbox",
            "Drafts Mailbox",
            "Where to store the draft",
            VariableType::String,
        )
        .set_default_value(Some(json!("Drafts")));

        node.add_input_pin(
            "create_if_missing",
            "Create If Missing",
            "Create the destination mailbox if it doesn't exist",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin("from", "From", "From header", VariableType::String);
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

        node.add_input_pin(
            "mark_seen",
            "Mark as Seen",
            "Save draft with \\Seen flag in addition to \\Draft",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

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

        let connection: ImapConnection = context.evaluate_pin("connection").await?;
        let mailbox: String = context.evaluate_pin("mailbox").await?;
        let create_if_missing: bool = context.evaluate_pin("create_if_missing").await?;

        let from: String = context.evaluate_pin("from").await?;
        let to: String = context.evaluate_pin("to").await?;
        let cc: String = context.evaluate_pin("cc").await?;
        let bcc: String = context.evaluate_pin("bcc").await?;
        let subject: String = context.evaluate_pin("subject").await?;
        let body_text: String = context.evaluate_pin("body_text").await?;
        let body_html: String = context.evaluate_pin("body_html").await?;
        let mark_seen: bool = context.evaluate_pin("mark_seen").await?;

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

        let mut cached_session = connection.to_session_cache(context).await?;

        if create_if_missing && !cached_session.mailbox_exists(&mailbox).await? {
            context.log_message(
                &format!("Drafts mailbox '{}' missing → creating", mailbox),
                LogLevel::Debug,
            );
            cached_session.create_mailbox(&mailbox).await.map_err(|e| {
                flow_like_types::anyhow!("Failed to create mailbox '{}': {}", mailbox, e)
            })?;
        }

        let message_id = generate_message_id(&from);
        let message = build_rfc5322_message_send(
            &from,
            &to,
            &cc,
            &bcc,
            &subject,
            &body_text,
            &body_html,
            &message_id,
            &attachments,
        );

        let flags_str = if mark_seen {
            "\\Draft \\Seen"
        } else {
            "\\Draft"
        };

        let mut session = cached_session.session.lock().await;
        if let Err(e) = session
            .append(&mailbox, Some(flags_str), None, message.as_bytes())
            .await
        {
            context.log_message(
                &format!(
                    "APPEND with flags failed on '{}' ({}). Retrying without flags…",
                    mailbox, e
                ),
                LogLevel::Warn,
            );
            session
                .append(&mailbox, None, None, message.as_bytes())
                .await
                .map_err(|e2| {
                    flow_like_types::anyhow!("APPEND to '{}' failed (no flags): {}", mailbox, e2)
                })?;
        }

        context.log_message(
            &format!(
                "Draft created in '{}' (subject: '{}') with {} attachment(s)",
                mailbox,
                subject,
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

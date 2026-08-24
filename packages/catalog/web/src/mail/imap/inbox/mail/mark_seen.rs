#[cfg(feature = "execute")]
use flow_like::flow::execution::LogLevel;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};
#[cfg(feature = "execute")]
use futures::TryStreamExt;

use crate::mail::imap::inbox::list::EmailRef;

#[crate::register_node]
#[derive(Default)]
pub struct ImapMarkSeenNode;

impl ImapMarkSeenNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for ImapMarkSeenNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "email_imap_mark_seen",
            "Mark Mail as Seen",
            "Marks a mail (by UID) as seen/read in IMAP mailbox",
            "Email/IMAP",
        );
        node.set_flowscript_name("imap", "markSeen");
        node.set_receiver("email");
        node.add_icon("/flow/icons/mail.svg");

        node.add_input_pin("exec_in", "In", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "email",
            "Email",
            "EmailRef containing connection, inbox, uid",
            VariableType::Struct,
        )
        .set_schema::<EmailRef>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "mark_as_seen",
            "Mark as Seen",
            "True to mark as seen, false to mark as unseen",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_output_pin(
            "email_ref",
            "Email Ref",
            "Reference to the marked message",
            VariableType::Struct,
        )
        .set_schema::<EmailRef>();
        node.add_output_pin(
            "exec_out",
            "Out",
            "Execution output",
            VariableType::Execution,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let email: EmailRef = context.evaluate_pin("email").await?;
        let mark_as_seen: bool = context.evaluate_pin("mark_as_seen").await?;

        let cached_session = email.connection.to_session_cache(context).await?;

        cached_session
            .session
            .lock()
            .await
            .select(&email.inbox.name)
            .await
            .map_err(|e| {
                flow_like_types::anyhow!("Failed to select mailbox '{}': {}", email.inbox.name, e)
            })?;

        let uid = email.uid.clone().to_string();
        let flag_operation = if mark_as_seen {
            "+FLAGS.SILENT \\Seen"
        } else {
            "-FLAGS.SILENT \\Seen"
        };

        cached_session
            .session
            .lock()
            .await
            .uid_store(&uid, flag_operation)
            .await
            .map_err(|e| flow_like_types::anyhow!("UID STORE {} failed: {}", flag_operation, e))?
            .try_collect::<Vec<_>>()
            .await?;

        let action = if mark_as_seen { "seen" } else { "unseen" };
        context.log_message(
            &format!(
                "Marked UID {} as {} in '{}'",
                email.uid, action, email.inbox.name
            ),
            LogLevel::Debug,
        );

        context.set_pin_value("email_ref", json!(email)).await?;
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

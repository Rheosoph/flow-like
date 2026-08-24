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
pub struct ImapDeleteMailNode;

impl ImapDeleteMailNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for ImapDeleteMailNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "email_imap_delete_message",
            "Delete Mail",
            "Deletes a mail (by UID) from its current mailbox",
            "Email/IMAP",
        );
        node.set_flowscript_name("imap", "deleteMessage");
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
            "expunge_mode",
            "Expunge Mode",
            "How to remove after marking \\Deleted",
            VariableType::String,
        )
        .set_default_value(Some(json!("uid_expunge")))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "uid_expunge".to_string(),
                    "expunge".to_string(),
                    "none".to_string(),
                ])
                .build(),
        );

        node.add_output_pin("exec_out", "", "", VariableType::Execution);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let email: EmailRef = context.evaluate_pin("email").await?;
        let expunge_mode: String = context.evaluate_pin("expunge_mode").await?;

        let cached_session = email.connection.to_session_cache(context).await?;

        // Ensure the source mailbox is selected
        let source_mailbox = email.inbox.name.clone();
        cached_session
            .session
            .lock()
            .await
            .select(&source_mailbox)
            .await
            .map_err(|e| {
                flow_like_types::anyhow!("Failed to select mailbox '{}': {}", source_mailbox, e)
            })?;

        let uid = email.uid.to_string();
        let mut session = cached_session.session.lock().await;

        // Mark as \Deleted
        session
            .uid_store(&uid, "+FLAGS.SILENT \\\\Deleted")
            .await
            .map_err(|e| flow_like_types::anyhow!("UID STORE +FLAGS \\Deleted failed: {}", e))?
            .try_collect::<Vec<_>>()
            .await?;

        // Expunge behavior
        match expunge_mode.as_str() {
            "uid_expunge" => {
                let uid_expunge_result = session.uid_expunge(&uid).await;
                if let Err(e) = &uid_expunge_result {
                    context.log_message(
                        &format!(
                            "UID EXPUNGE not available or failed ({}), attempting plain EXPUNGE",
                            e
                        ),
                        LogLevel::Debug,
                    );
                    drop(uid_expunge_result);
                    session
                        .expunge()
                        .await
                        .map_err(|e2| flow_like_types::anyhow!("EXPUNGE failed: {}", e2))?
                        .try_collect::<Vec<_>>()
                        .await?;
                } else {
                    context.log_message(
                        &format!("UID EXPUNGE successful for UID {}", uid),
                        LogLevel::Debug,
                    );
                }
            }
            "expunge" => {
                session
                    .expunge()
                    .await
                    .map_err(|e| flow_like_types::anyhow!("EXPUNGE failed: {}", e))?
                    .try_collect::<Vec<_>>()
                    .await?;
            }
            "none" => { /* leave as \Deleted */ }
            other => {
                return Err(flow_like_types::anyhow!(
                    "Unknown expunge_mode '{}'. Expected uid_expunge | expunge | none",
                    other
                ));
            }
        }

        context.log_message(
            &format!("Deleted UID {} from '{}'", email.uid, source_mailbox),
            LogLevel::Debug,
        );

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

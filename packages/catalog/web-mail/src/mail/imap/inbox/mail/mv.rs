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
pub struct ImapMoveMailNode;

impl ImapMoveMailNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for ImapMoveMailNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "email_imap_move_message",
            "Move Mail to Mailbox",
            "Moves a mail (by UID) to another IMAP mailbox",
            "Email/IMAP",
        );
        node.set_flowscript_name("imap", "moveMessage");
        node.set_receiver("email");
        node.add_icon("/flow/icons/mail.svg");

        // Impure node → requires execution pins
        node.add_input_pin("exec_in", "In", "Trigger", VariableType::Execution);

        // EmailRef input (schema enforced)
        node.add_input_pin(
            "email",
            "Email",
            "EmailRef containing connection, inbox, uid",
            VariableType::Struct,
        )
        .set_schema::<EmailRef>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "destination",
            "Destination Mailbox",
            "Target mailbox (e.g. Archive)",
            VariableType::String,
        )
        .set_default_value(Some(json!("Archive")));

        node.add_input_pin(
            "create_if_missing",
            "Create If Missing",
            "Create the destination mailbox if it doesn't exist",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_input_pin(
            "expunge_mode",
            "Expunge Mode",
            "How to remove from source when MOVE is unavailable",
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

        node.add_output_pin(
            "new_message_ref",
            "New Message Ref",
            "Reference to the new message",
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

        let mut email: EmailRef = context.evaluate_pin("email").await?;
        let destination: String = context.evaluate_pin("destination").await?;
        let create_if_missing: bool = context.evaluate_pin("create_if_missing").await?;
        let expunge_mode: String = context.evaluate_pin("expunge_mode").await?;

        let mut cached_session = email.connection.to_session_cache(context).await?;

        let source_mailbox = email.inbox.name.clone();
        cached_session
            .session
            .lock()
            .await
            .select(&source_mailbox)
            .await
            .map_err(|e| {
                flow_like_types::anyhow!(
                    "Failed to select source mailbox '{}': {}",
                    source_mailbox,
                    e
                )
            })?;

        if create_if_missing && !cached_session.mailbox_exists(&destination).await? {
            context.log_message(
                &format!("Destination '{}' missing → creating", destination),
                LogLevel::Debug,
            );
            cached_session
                .create_mailbox(&destination)
                .await
                .map_err(|e| {
                    flow_like_types::anyhow!("Failed to create mailbox '{}': {}", destination, e)
                })?;
        }

        let uid = email.uid.clone().to_string();

        let mut session = cached_session.session.lock().await;
        let move_attempt = session.uid_mv(&uid, &destination).await;

        let outcome = match move_attempt {
            Ok(_) => MoveOutcome::Moved,
            Err(move_err) => {
                context.log_message(
                    &format!(
                        "UID MOVE failed ({}). Falling back to UID COPY + delete.",
                        move_err
                    ),
                    LogLevel::Debug,
                );

                session.uid_copy(&uid, &destination).await.map_err(|e| {
                    flow_like_types::anyhow!("UID COPY to '{}' failed: {}", destination, e)
                })?;

                session
                    .uid_store(&uid, "+FLAGS.SILENT \\Deleted")
                    .await
                    .map_err(|e| {
                        flow_like_types::anyhow!("UID STORE +FLAGS \\Deleted failed: {}", e)
                    })?
                    .try_collect::<Vec<_>>()
                    .await?;

                match expunge_mode.as_str() {
                    "uid_expunge" => {
                        let uid_expunge_result = session.uid_expunge(&uid).await;
                        if let Err(e) = &uid_expunge_result {
                            context.log_message(
                                &format!("UID EXPUNGE not available or failed ({}), attempting plain EXPUNGE", e),
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

                MoveOutcome::CopiedThenDeleted
            }
        };

        let msg = match outcome {
            MoveOutcome::Moved => format!(
                "Moved UID {} from '{}' to '{}' via UID MOVE",
                email.uid, source_mailbox, destination
            ),
            MoveOutcome::CopiedThenDeleted => format!(
                "Copied UID {} to '{}' and removed from '{}'",
                email.uid, destination, source_mailbox
            ),
        };

        context.log_message(&msg, LogLevel::Debug);

        email.inbox.name = destination.clone();
        context
            .set_pin_value("new_message_ref", json!(email))
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
#[derive(Debug, Clone, Copy)]
enum MoveOutcome {
    Moved,
    CopiedThenDeleted,
}

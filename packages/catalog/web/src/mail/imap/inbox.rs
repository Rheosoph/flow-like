use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
#[cfg(feature = "execute")]
use flow_like_types::{async_trait, bail, json::json};
#[cfg(not(feature = "execute"))]
use flow_like_types::{async_trait, json::json};
#[cfg(feature = "execute")]
use futures::TryStreamExt;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub mod list;
pub mod mail;

use crate::mail::imap::ImapConnection;

#[derive(Serialize, Deserialize, JsonSchema, Clone)]
pub struct ImapInbox {
    pub connection: ImapConnection,
    pub name: String,
}

impl ImapInbox {
    pub fn new(connection: ImapConnection, name: String) -> Self {
        ImapInbox { connection, name }
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct ImapInboxNode;

impl ImapInboxNode {
    pub fn new() -> Self {
        ImapInboxNode
    }
}

#[async_trait]
impl NodeLogic for ImapInboxNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "mail_imap_inbox",
            "IMAP Inbox",
            "Wraps an IMAP mailbox for paginated fetching",
            "Email/IMAP",
        );
        node.set_flowscript_name("imap", "inbox");
        node.set_receiver("connection");
        node.add_icon("/flow/icons/mail.svg");

        node.add_input_pin("exec_in", "In", "Execution input", VariableType::Execution);
        node.add_output_pin(
            "exec_out",
            "Out",
            "Execution output",
            VariableType::Execution,
        );

        node.add_input_pin(
            "connection",
            "Connection",
            "Reference to an existing IMAP connection",
            VariableType::Struct,
        )
        .set_schema::<ImapConnection>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "inbox",
            "Inbox",
            "Mailbox name to wrap",
            VariableType::String,
        )
        .set_default_value(Some(json!("INBOX")));

        node.add_output_pin(
            "inbox_struct",
            "Inbox Struct",
            "Wrapped IMAP inbox for pagination",
            VariableType::Struct,
        )
        .set_schema::<ImapInbox>();

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let connection: ImapConnection = context.evaluate_pin("connection").await?;
        let inbox_name: String = context.evaluate_pin("inbox").await?;

        let inbox = ImapInbox::new(connection.clone(), inbox_name);

        context.set_pin_value("inbox_struct", json!(inbox)).await?;

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

#[crate::register_node]
#[derive(Default)]
pub struct ImapListInboxesNode;

impl ImapListInboxesNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for ImapListInboxesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "mail_imap_list_inboxes",
            "IMAP List Inboxes",
            "Lists all available IMAP mailboxes",
            "Email/IMAP",
        );
        node.set_flowscript_name("imap", "listInboxes");
        node.set_receiver("connection");
        node.add_icon("/flow/icons/mail.svg");

        node.add_input_pin("exec_in", "In", "Execution input", VariableType::Execution);
        node.add_output_pin(
            "exec_out",
            "Out",
            "Execution output",
            VariableType::Execution,
        );

        node.add_input_pin(
            "connection",
            "Connection",
            "Reference to an existing IMAP connection",
            VariableType::Struct,
        )
        .set_schema::<ImapConnection>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "names",
            "Mailbox Names",
            "All mailbox names returned by the server",
            VariableType::String,
        )
        .set_value_type(ValueType::Array);

        node.add_output_pin(
            "inboxes",
            "Inbox Structs",
            "All mailboxes wrapped as ImapInbox",
            VariableType::Struct,
        )
        .set_schema::<ImapInbox>()
        .set_value_type(ValueType::Array);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let connection: ImapConnection = context.evaluate_pin("connection").await?;
        let session_arc = connection.to_session(context).await?;
        let mut session = session_arc.lock().await;

        let name_stream = session
            .list(None, Some("*"))
            .await
            .map_err(|e| flow_like_types::anyhow!("IMAP LIST failed: {}", e))?;

        let names: Vec<String> = name_stream
            .map_ok(|n| n.name().to_string())
            .try_collect()
            .await
            .map_err(|e| flow_like_types::anyhow!("IMAP LIST stream failed: {}", e))?;

        let inboxes: Vec<ImapInbox> = names
            .iter()
            .cloned()
            .map(|n| ImapInbox::new(connection.clone(), n))
            .collect();

        context.set_pin_value("names", json!(names)).await?;
        context.set_pin_value("inboxes", json!(inboxes)).await?;

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

#[crate::register_node]
#[derive(Default)]
pub struct ImapCreateMailboxNode;

impl ImapCreateMailboxNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for ImapCreateMailboxNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "mail_imap_create_mailbox",
            "IMAP Create Mailbox (If Missing)",
            "Creates a mailbox if it doesn't exist; no-op if it already exists",
            "Email/IMAP",
        );
        node.set_flowscript_name("imap", "createMailbox");
        node.set_receiver("connection");
        node.add_icon("/flow/icons/mail.svg");

        node.add_input_pin("exec_in", "In", "Execution input", VariableType::Execution);
        node.add_output_pin(
            "exec_out",
            "Success",
            "Execution output (success)",
            VariableType::Execution,
        );

        node.add_input_pin(
            "connection",
            "Connection",
            "Reference to an existing IMAP connection",
            VariableType::Struct,
        )
        .set_schema::<ImapConnection>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "name",
            "Mailbox Name",
            "Mailbox to create if missing",
            VariableType::String,
        )
        .set_default_value(Some(json!("NewMailbox")));

        node.add_output_pin(
            "created",
            "Created",
            "True if created, false if it already existed",
            VariableType::Boolean,
        );

        node.add_output_pin(
            "inbox_struct",
            "Inbox",
            "The resulting mailbox wrapped as ImapInbox",
            VariableType::Struct,
        )
        .set_schema::<ImapInbox>();

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        // Deactivate both exec outs up front
        context.deactivate_exec_pin("exec_out").await?;

        let name: String = context.evaluate_pin("name").await?;

        let connection: ImapConnection = context.evaluate_pin("connection").await?;
        let mut session = connection.to_session_cache(context).await?;

        let created = match session.mailbox_exists(&name).await {
            Ok(true) => false,
            Ok(false) => match session.create_mailbox(&name).await {
                Ok(()) => true,
                Err(e) => {
                    let msg = format!("{e}");
                    let looks_like_exists = msg.to_ascii_lowercase().contains("exists");
                    if looks_like_exists {
                        false
                    } else {
                        bail!(format!("Failed to create mailbox: {e}"))
                    }
                }
            },
            Err(e) => {
                bail!(format!("Failed to check mailbox existence: {e}"))
            }
        };

        let inbox = ImapInbox::new(connection.clone(), name);

        context.set_pin_value("created", json!(created)).await?;
        context.set_pin_value("inbox_struct", json!(inbox)).await?;

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

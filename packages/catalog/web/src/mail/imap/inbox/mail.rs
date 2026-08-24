use crate::mail::imap::inbox::list::{Attachment, Email, EmailRef, MailAddress};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

pub mod copy;
pub mod delete;
pub mod draft;
pub mod mark_seen;
pub mod mv;

#[crate::register_node]
#[derive(Default)]
pub struct FetchMailNode;

impl FetchMailNode {
    pub fn new() -> Self {
        FetchMailNode
    }
}

#[async_trait]
impl NodeLogic for FetchMailNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "email_imap_inbox_fetch_mail",
            "Fetch Mail",
            "Fetches the full email content",
            "Email/IMAP",
        );
        node.set_flowscript_name("imap", "fetchMail");
        node.set_receiver("email_ref");
        node.add_icon("/flow/icons/mail.svg");
        node.set_version(1);

        node.add_input_pin("exec_in", "In", "Execution input", VariableType::Execution);
        node.add_output_pin(
            "exec_out",
            "Out",
            "Execution output",
            VariableType::Execution,
        );

        node.add_input_pin(
            "email_ref",
            "Email Reference",
            "Reference to the email (connection+uid+inbox)",
            VariableType::Struct,
        )
        .set_schema::<EmailRef>();

        node.add_output_pin(
            "email",
            "Email",
            "Parsed email metadata",
            VariableType::Struct,
        )
        .set_schema::<Email>();

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let email: EmailRef = context.evaluate_pin("email_ref").await?;
        let parsed = email.fetch(context).await?;

        context.set_pin_value("email", json!(parsed)).await?;
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
pub struct EmailHeadersNode;

impl EmailHeadersNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for EmailHeadersNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "email_get_headers",
            "Email → Headers",
            "Access address header fields of an Email",
            "Email/Access",
        );
        node.set_flowscript_name("email", "getHeaders");
        node.set_receiver("email");
        node.add_icon("/flow/icons/mail.svg");
        node.set_version(2);

        node.add_input_pin("email", "Email", "Email struct", VariableType::Struct)
            .set_schema::<Email>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin("from", "From", "Primary from address", VariableType::Struct)
            .set_schema::<Option<MailAddress>>();
        node.add_output_pin("sender", "Sender", "Sender addresses", VariableType::Struct)
            .set_schema::<MailAddress>()
            .set_value_type(ValueType::Array);
        node.add_output_pin("to", "To", "To addresses", VariableType::Struct)
            .set_schema::<MailAddress>()
            .set_value_type(ValueType::Array);
        node.add_output_pin("cc", "Cc", "Carbon copy addresses", VariableType::Struct)
            .set_schema::<MailAddress>()
            .set_value_type(ValueType::Array);
        node.add_output_pin(
            "bcc",
            "Bcc",
            "Blind carbon copy addresses",
            VariableType::Struct,
        )
        .set_schema::<MailAddress>()
        .set_value_type(ValueType::Array);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let email: Email = context.evaluate_pin("email").await?;

        context.set_pin_value("from", json!(email.from)).await?;
        context.set_pin_value("sender", json!(email.sender)).await?;
        context.set_pin_value("to", json!(email.to)).await?;
        context.set_pin_value("cc", json!(email.cc)).await?;
        context.set_pin_value("bcc", json!(email.bcc)).await?;

        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct EmailContentNode;

impl EmailContentNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for EmailContentNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "email_get_content",
            "Email → Content",
            "Access subject, date, plain and HTML bodies",
            "Email/Access",
        );
        node.set_flowscript_name("email", "getContent");
        node.set_receiver("email");
        node.add_icon("/flow/icons/mail.svg");
        node.set_version(1);

        node.add_input_pin("email", "Email", "Email struct", VariableType::Struct)
            .set_schema::<Email>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin("subject", "Subject", "Email subject", VariableType::String);
        node.add_output_pin("date", "Date", "Email date", VariableType::String);
        node.add_output_pin("plain", "Plain", "Plaintext body", VariableType::String);
        node.add_output_pin("html", "HTML", "HTML body", VariableType::String);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let email: Email = context.evaluate_pin("email").await?;

        context
            .set_pin_value("subject", json!(email.subject))
            .await?;
        context.set_pin_value("date", json!(email.date)).await?;
        context.set_pin_value("plain", json!(email.plain)).await?;
        context.set_pin_value("html", json!(email.html)).await?;

        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct EmailAttachmentsNode;

impl EmailAttachmentsNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for EmailAttachmentsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "email_get_attachments",
            "Email → Attachments",
            "Access attachments array",
            "Email/Access",
        );
        node.set_flowscript_name("email", "getAttachments");
        node.set_receiver("email");
        node.add_icon("/flow/icons/mail.svg");
        node.set_version(2);

        node.add_input_pin("email", "Email", "Email struct", VariableType::Struct)
            .set_schema::<Email>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "attachments",
            "Attachments",
            "Array of attachments",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<Attachment>();

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let email: Email = context.evaluate_pin("email").await?;
        context
            .set_pin_value("attachments", json!(email.attachments))
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct MailAddressFieldsNode;

impl MailAddressFieldsNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for MailAddressFieldsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "mail_address_fields",
            "MailAddress → Fields",
            "Access name and email on a MailAddress",
            "Email/Access",
        );
        node.set_flowscript_name("email", "addressToFields");
        node.add_icon("/flow/icons/mail.svg");

        node.add_input_pin(
            "address",
            "Address",
            "MailAddress struct",
            VariableType::Struct,
        )
        .set_schema::<MailAddress>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "name",
            "Name",
            "Display name (optional)",
            VariableType::String,
        );
        node.add_output_pin("email", "Email", "Email address", VariableType::String);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let addr: MailAddress = context.evaluate_pin("address").await?;
        context.set_pin_value("name", json!(addr.name)).await?;
        context.set_pin_value("email", json!(addr.email)).await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct AttachmentFieldsNode;

impl AttachmentFieldsNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for AttachmentFieldsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "attachment_fields",
            "Attachment → Fields",
            "Access filename, content_type and data",
            "Email/Access",
        );
        node.set_flowscript_name("email", "attachmentToFields");
        node.add_icon("/flow/icons/mail.svg");

        node.add_input_pin(
            "attachment",
            "Attachment",
            "Attachment struct",
            VariableType::Struct,
        )
        .set_schema::<Attachment>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "filename",
            "Filename",
            "Attachment filename",
            VariableType::String,
        );
        node.add_output_pin(
            "content_type",
            "Content Type",
            "MIME content type",
            VariableType::String,
        );
        // Data is a Vec<u8>. We expose it as a Struct with its schema.
        node.add_output_pin("data", "Data", "Raw bytes (Vec<u8>)", VariableType::Byte)
            .set_value_type(ValueType::Array);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let a: Attachment = context.evaluate_pin("attachment").await?;
        context.set_pin_value("filename", json!(a.filename)).await?;
        context
            .set_pin_value("content_type", json!(a.content_type))
            .await?;
        context.set_pin_value("data", json!(a.data)).await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct ToMailReferenceNode;

impl ToMailReferenceNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for ToMailReferenceNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "mail_imap_inbox_mail_to_reference",
            "Mail → Reference",
            "Transforms a Mail struct into a reference",
            "Email/Access",
        );
        node.set_flowscript_name("email", "toReference");
        node.set_receiver("mail");
        node.add_icon("/flow/icons/mail.svg");
        node.set_version(1);

        node.add_input_pin("mail", "Mail", "Mail struct", VariableType::Struct)
            .set_schema::<Email>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "reference",
            "Reference",
            "Mail reference",
            VariableType::Struct,
        )
        .set_schema::<EmailRef>();

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let mail: Email = context.evaluate_pin("mail").await?;
        let email_ref = EmailRef::new(mail.connection.clone(), mail.inbox.clone(), mail.uid);
        context.set_pin_value("reference", json!(email_ref)).await?;
        Ok(())
    }
}

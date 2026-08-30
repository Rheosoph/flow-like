use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};

use super::Attachment;

#[crate::register_node]
#[derive(Default)]
pub struct AttachmentToUrlNode {}

impl AttachmentToUrlNode {
    pub fn new() -> Self {
        AttachmentToUrlNode {}
    }
}

#[async_trait]
impl NodeLogic for AttachmentToUrlNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "events_chat_attachment_to_signed_url",
            "To Signed URL",
            "Get the URL from an attachment",
            "Events/Chat/Attachments",
        );
        node.set_flowscript_name("chat", "toSignedUrl");
        node.set_receiver("attachment");
        node.add_icon("/flow/icons/paperclip.svg");

        node.add_input_pin(
            "attachment",
            "Attachment",
            "Attachment to the Chat",
            VariableType::Struct,
        )
        .set_schema::<Attachment>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin("signed_url", "Signed URL", "", VariableType::String);

        node.add_output_pin("success", "Success", "", VariableType::Boolean);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let attachment: Attachment = context.evaluate_pin("attachment").await?;

        match attachment {
            Attachment::Url(url) => {
                context
                    .set_pin_value("signed_url", Value::String(url))
                    .await?;
                context.set_pin_value("success", json!(true)).await?;
            }
            Attachment::Complex(complex) => {
                context
                    .set_pin_value("signed_url", Value::String(complex.url))
                    .await?;
                context.set_pin_value("success", json!(true)).await?;
            }
        }

        return Ok(());
    }
}

#[cfg(feature = "execute")]
use lopdf::Document;

use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::FlowPath;
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct PdfEncryptNode;

impl PdfEncryptNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for PdfEncryptNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "pdf_encrypt",
            "Encrypt PDF",
            "Encrypt a PDF with a user password for restricted access.",
            "Document/PDF",
        );
        node.add_icon("/flow/icons/lock.svg");
        node.set_scores(
            NodeScores::new()
                .set_privacy(10)
                .set_security(10)
                .set_performance(6)
                .set_governance(9)
                .set_reliability(7)
                .set_cost(10)
                .build(),
        );

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin("template", "Template", "PDF file", VariableType::Struct)
            .set_schema::<FlowPath>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "user_password",
            "User Password",
            "Password required to open",
            VariableType::String,
        );
        node.add_input_pin(
            "owner_password",
            "Owner Password",
            "Password for full access (optional, defaults to user password)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin("output", "Output Path", "Save path", VariableType::Struct)
            .set_schema::<FlowPath>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin("exec_out", "Done", "Continues", VariableType::Execution);
        node.add_output_pin("result", "Result", "Output file path", VariableType::Struct)
            .set_schema::<FlowPath>();

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let template: FlowPath = context.evaluate_pin("template").await?;
        let user_password: String = context.evaluate_pin("user_password").await?;
        let owner_password: String = context.evaluate_pin("owner_password").await?;
        let output: FlowPath = context.evaluate_pin("output").await?;

        let bytes = template.get(context, false).await?;
        let mut doc = Document::load_mem(&bytes)?;

        let owner_pw = if owner_password.is_empty() {
            user_password.clone()
        } else {
            owner_password
        };

        let encryption_version = lopdf::encryption::EncryptionVersion::V1 {
            document: &doc,
            owner_password: &owner_pw,
            user_password: &user_password,
            permissions: lopdf::encryption::Permissions::default(),
        };
        let state = lopdf::encryption::EncryptionState::try_from(encryption_version)?;
        doc.encrypt(&state)?;

        let mut buf = Vec::new();
        doc.save_to(&mut buf)?;
        output.put(context, buf, false).await?;
        context.set_pin_value("result", json!(output)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!("Requires the 'execute' feature"))
    }
}

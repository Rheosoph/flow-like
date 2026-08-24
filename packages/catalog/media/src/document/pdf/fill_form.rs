#[cfg(feature = "execute")]
use lopdf::{Document, Object, StringFormat};

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
pub struct PdfFillFormNode;

impl PdfFillFormNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for PdfFillFormNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "pdf_fill_form",
            "Fill PDF Form Field",
            "Sets the value of a named AcroForm field in a PDF document.",
            "Document/PDF",
        );
        node.set_flowscript_name("pdf", "fillForm");
        node.add_icon("/flow/icons/text.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(8)
                .set_security(7)
                .set_performance(8)
                .set_governance(8)
                .set_reliability(7)
                .set_cost(10)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger execution",
            VariableType::Execution,
        );

        node.add_input_pin(
            "template",
            "Template",
            "PDF file containing form fields",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "field_name",
            "Field Name",
            "Name of the AcroForm field to fill",
            VariableType::String,
        );

        node.add_input_pin(
            "field_value",
            "Field Value",
            "Value to set on the form field",
            VariableType::String,
        );

        node.add_input_pin(
            "output",
            "Output Path",
            "Path to save the filled PDF",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exec_out",
            "Output",
            "Execution continues after form fill",
            VariableType::Execution,
        );

        node.add_output_pin("result", "Result", "Output file path", VariableType::Struct)
            .set_schema::<FlowPath>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let template: FlowPath = context.evaluate_pin("template").await?;
        let field_name: String = context.evaluate_pin("field_name").await?;
        let field_value: String = context.evaluate_pin("field_value").await?;
        let output: FlowPath = context.evaluate_pin("output").await?;

        let bytes = template.get(context, false).await?;
        let mut doc = Document::load_mem(&bytes)?;

        set_acroform_field(&mut doc, &field_name, &field_value)?;

        let mut buf = Vec::new();
        doc.save_to(&mut buf)?;

        output.put(context, buf, false).await?;

        context.set_pin_value("result", json!(output)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "PDF processing requires the 'execute' feature"
        ))
    }
}

#[cfg(feature = "execute")]
fn set_acroform_field(
    doc: &mut Document,
    field_name: &str,
    field_value: &str,
) -> flow_like_types::Result<()> {
    let acroform_id = doc
        .catalog()?
        .get(b"AcroForm")
        .ok()
        .and_then(|obj| obj.as_reference().ok())
        .ok_or_else(|| flow_like_types::anyhow!("PDF has no AcroForm"))?;

    let fields_refs: Vec<lopdf::ObjectId> = {
        let acroform = doc.get_object(acroform_id)?;
        let dict = acroform
            .as_dict()
            .map_err(|_| flow_like_types::anyhow!("AcroForm is not a dictionary"))?;
        let fields = dict
            .get(b"Fields")
            .map_err(|_| flow_like_types::anyhow!("AcroForm has no Fields array"))?;
        let arr = fields
            .as_array()
            .map_err(|_| flow_like_types::anyhow!("Fields is not an array"))?;
        arr.iter().filter_map(|o| o.as_reference().ok()).collect()
    };

    for field_id in fields_refs {
        if try_set_field(doc, field_id, field_name, field_value)? {
            return Ok(());
        }
    }

    Err(flow_like_types::anyhow!(
        "Form field '{}' not found",
        field_name
    ))
}

#[cfg(feature = "execute")]
fn try_set_field(
    doc: &mut Document,
    obj_id: lopdf::ObjectId,
    target_name: &str,
    value: &str,
) -> flow_like_types::Result<bool> {
    let name_match = {
        let obj = doc.get_object(obj_id)?;
        let dict = obj
            .as_dict()
            .map_err(|_| flow_like_types::anyhow!("Field object is not a dictionary"))?;
        match dict.get(b"T") {
            Ok(t_obj) => match t_obj {
                Object::String(bytes, _) => String::from_utf8_lossy(bytes).as_ref() == target_name,
                _ => false,
            },
            Err(_) => false,
        }
    };

    if name_match {
        let obj = doc.get_object_mut(obj_id)?;
        let dict = obj
            .as_dict_mut()
            .map_err(|_| flow_like_types::anyhow!("Field object is not a dictionary"))?;
        dict.set(
            "V",
            Object::String(value.as_bytes().to_vec(), StringFormat::Literal),
        );
        return Ok(true);
    }

    let kid_refs: Vec<lopdf::ObjectId> = {
        let obj = doc.get_object(obj_id)?;
        let dict = obj
            .as_dict()
            .map_err(|_| flow_like_types::anyhow!("Field object is not a dictionary"))?;
        match dict.get(b"Kids") {
            Ok(kids) => match kids.as_array() {
                Ok(arr) => arr.iter().filter_map(|o| o.as_reference().ok()).collect(),
                Err(_) => vec![],
            },
            Err(_) => vec![],
        }
    };

    for kid_id in kid_refs {
        if try_set_field(doc, kid_id, target_name, value)? {
            return Ok(true);
        }
    }

    Ok(false)
}

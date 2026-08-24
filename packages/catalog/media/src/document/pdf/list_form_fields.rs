#[cfg(feature = "execute")]
use lopdf::{Document, Object};

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
pub struct PdfListFormFieldsNode;

impl PdfListFormFieldsNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for PdfListFormFieldsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "pdf_list_form_fields",
            "List PDF Form Fields",
            "Reads a PDF and returns all AcroForm field names so you know which fields are available to fill.",
            "Document/PDF",
        );
        node.set_flowscript_name("pdf", "listFormFields");
        node.add_icon("/flow/icons/text.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(8)
                .set_security(8)
                .set_performance(9)
                .set_governance(8)
                .set_reliability(8)
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
            "PDF File",
            "PDF file containing form fields",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exec_out",
            "Output",
            "Execution continues after reading fields",
            VariableType::Execution,
        );

        node.add_output_pin(
            "field_names",
            "Field Names",
            "Array of all form field names in the PDF",
            VariableType::String,
        )
        .set_value_type(flow_like::flow::pin::ValueType::Array);

        node.add_output_pin(
            "field_count",
            "Field Count",
            "Total number of form fields",
            VariableType::Integer,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let template: FlowPath = context.evaluate_pin("template").await?;
        let bytes = template.get(context, false).await?;
        let doc = Document::load_mem(&bytes)?;

        let field_names = collect_form_field_names(&doc)?;
        let count = field_names.len() as i64;

        context
            .set_pin_value("field_names", json!(field_names))
            .await?;
        context.set_pin_value("field_count", json!(count)).await?;
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
fn collect_form_field_names(doc: &Document) -> flow_like_types::Result<Vec<String>> {
    let catalog = doc.catalog()?;

    let acroform_ref = match catalog.get(b"AcroForm") {
        Ok(obj) => match obj.as_reference() {
            Ok(id) => id,
            Err(_) => return Ok(vec![]),
        },
        Err(_) => return Ok(vec![]),
    };

    let acroform = doc.get_object(acroform_ref)?;
    let dict = match acroform.as_dict() {
        Ok(d) => d,
        Err(_) => return Ok(vec![]),
    };

    let fields = match dict.get(b"Fields") {
        Ok(f) => match f.as_array() {
            Ok(arr) => arr.clone(),
            Err(_) => return Ok(vec![]),
        },
        Err(_) => return Ok(vec![]),
    };

    let field_refs: Vec<lopdf::ObjectId> = fields
        .iter()
        .filter_map(|o| o.as_reference().ok())
        .collect();

    let mut names = Vec::new();
    for field_id in field_refs {
        collect_field_names_recursive(doc, field_id, &mut names, "");
    }

    Ok(names)
}

#[cfg(feature = "execute")]
fn collect_field_names_recursive(
    doc: &Document,
    obj_id: lopdf::ObjectId,
    names: &mut Vec<String>,
    parent_path: &str,
) {
    let obj = match doc.get_object(obj_id) {
        Ok(o) => o,
        Err(_) => return,
    };

    let dict = match obj.as_dict() {
        Ok(d) => d,
        Err(_) => return,
    };

    let field_name = dict
        .get(b"T")
        .ok()
        .and_then(|t| match t {
            Object::String(bytes, _) => Some(String::from_utf8_lossy(bytes).to_string()),
            _ => None,
        })
        .unwrap_or_default();

    let full_path = if parent_path.is_empty() {
        field_name.clone()
    } else if field_name.is_empty() {
        parent_path.to_string()
    } else {
        format!("{}.{}", parent_path, field_name)
    };

    let kid_refs: Vec<lopdf::ObjectId> = dict
        .get(b"Kids")
        .ok()
        .and_then(|k| k.as_array().ok())
        .map(|arr| arr.iter().filter_map(|o| o.as_reference().ok()).collect())
        .unwrap_or_default();

    if kid_refs.is_empty() {
        if !full_path.is_empty() {
            names.push(full_path);
        }
    } else {
        for kid_id in kid_refs {
            collect_field_names_recursive(doc, kid_id, names, &full_path);
        }
    }
}

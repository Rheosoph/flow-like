use crate::types::handles::AutomationSession;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct AssertTemplateExistsNode {}

impl AssertTemplateExistsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for AssertTemplateExistsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "rpa_assert_template_exists",
            "Assert Template Exists",
            "Asserts that a template image exists on screen",
            "Automation/RPA",
        );
        node.set_version(1);
        node.set_flowscript_name("rpa", "assertTemplateExists");
        node.add_icon("/flow/icons/rpa.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(2)
                .set_security(4)
                .set_performance(6)
                .set_governance(5)
                .set_reliability(8)
                .set_cost(8)
                .build(),
        );
        node.set_only_offline(true);

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "RPA session handle",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_input_pin(
            "template_path",
            "Template Path",
            "Path to the template image",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "template",
            "Template",
            "Template image from any FlowPath store; preferred over a local path",
            VariableType::Struct,
        )
        .set_schema::<flow_like_catalog_core::FlowPath>();

        node.add_input_pin(
            "confidence",
            "Confidence",
            "Minimum match confidence",
            VariableType::Float,
        )
        .set_default_value(Some(json!(0.8)));

        node.add_output_pin(
            "exec_pass",
            "Pass",
            "Assertion passed",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_fail",
            "Fail",
            "Assertion failed",
            VariableType::Execution,
        );

        node.add_output_pin(
            "passed",
            "Passed",
            "Whether assertion passed",
            VariableType::Boolean,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_pass").await?;
        context.deactivate_exec_pin("exec_fail").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        session.ensure_active(context).await?;
        let template_bytes = crate::types::screen_match::load_template(context).await?;
        let confidence: f64 = context.evaluate_pin("confidence").await?;

        let matches =
            crate::types::screen_match::match_desktop_async(template_bytes.clone(), confidence, -2)
                .await?;
        let passed = !matches.is_empty();

        context.set_pin_value("passed", json!(passed)).await?;

        if passed {
            context.activate_exec_pin("exec_pass").await?;
        } else {
            context.activate_exec_pin("exec_fail").await?;
        }

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "RPA automation requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct AssertColorAtPositionNode {}

impl AssertColorAtPositionNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for AssertColorAtPositionNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "rpa_assert_color",
            "Assert Color At Position",
            "Asserts that a specific color exists at a position",
            "Automation/RPA",
        );
        node.set_version(1);
        node.set_flowscript_name("rpa", "assertColor");
        node.add_icon("/flow/icons/rpa.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(3)
                .set_security(5)
                .set_performance(8)
                .set_governance(5)
                .set_reliability(7)
                .set_cost(9)
                .build(),
        );
        node.set_only_offline(true);

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "RPA session handle",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_input_pin("x", "X", "X position", VariableType::Integer)
            .set_default_value(Some(json!(0)));

        node.add_input_pin("y", "Y", "Y position", VariableType::Integer)
            .set_default_value(Some(json!(0)));

        node.add_input_pin("red", "Red", "Expected red (0-255)", VariableType::Integer)
            .set_default_value(Some(json!(255)));

        node.add_input_pin(
            "green",
            "Green",
            "Expected green (0-255)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(255)));

        node.add_input_pin(
            "blue",
            "Blue",
            "Expected blue (0-255)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(255)));

        node.add_input_pin(
            "tolerance",
            "Tolerance",
            "Color tolerance (0-255)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(10)));

        node.add_output_pin(
            "exec_pass",
            "Pass",
            "Assertion passed",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_fail",
            "Fail",
            "Assertion failed",
            VariableType::Execution,
        );

        node.add_output_pin(
            "passed",
            "Passed",
            "Whether assertion passed",
            VariableType::Boolean,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_pass").await?;
        context.deactivate_exec_pin("exec_fail").await?;

        let _session: AutomationSession = context.evaluate_pin("session").await?;
        _session.ensure_active(context).await?;
        let x: i64 = context.evaluate_pin("x").await?;
        let y: i64 = context.evaluate_pin("y").await?;
        let target_r: i64 = context.evaluate_pin("red").await?;
        let target_g: i64 = context.evaluate_pin("green").await?;
        let target_b: i64 = context.evaluate_pin("blue").await?;
        let tolerance: i64 = context.evaluate_pin("tolerance").await?;
        if [target_r, target_g, target_b, tolerance]
            .iter()
            .any(|v| !(0..=255).contains(v))
        {
            return Err(flow_like_types::anyhow!(
                "Color channels and tolerance must be between 0 and 255"
            ));
        }

        let [r, g, b] = crate::types::screen_match::capture_pixel(x, y)?;
        let passed = (r as i64 - target_r).abs() <= tolerance
            && (g as i64 - target_g).abs() <= tolerance
            && (b as i64 - target_b).abs() <= tolerance;

        context.set_pin_value("passed", json!(passed)).await?;

        if passed {
            context.activate_exec_pin("exec_pass").await?;
        } else {
            context.activate_exec_pin("exec_fail").await?;
        }

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "RPA automation requires the 'execute' feature"
        ))
    }
}

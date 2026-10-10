#![cfg_attr(not(feature = "execute"), allow(dead_code))]

use crate::computer::ocr::InputPoint;
use crate::types::handles::AutomationSession;
use crate::types::screen_frame::ScreenFrame;
use flow_like::{
    bit::Bit,
    flow::{
        execution::context::ExecutionContext,
        node::{Node, NodeLogic, NodeScores},
        pin::{PinOptions, ValueType},
        variable::VariableType,
    },
};
use flow_like_catalog_core::NodeImage;
use flow_like_types::{Value, async_trait, json::json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub(crate) const PERCEPTION_SCREENSHOT: &str = "screenshot";
pub(crate) const PERCEPTION_MARKS: &str = "screenshot_marks";
pub(crate) const PERCEPTION_MARKS_OCR: &str = "screenshot_marks_ocr";

pub(crate) const DEFAULT_FORBIDDEN_TEXT: [&str; 8] = [
    r"rm\s+-[a-zA-Z]*[rR][a-zA-Z]*\s+(/|~)",
    r"(curl|wget)\b[^|\n]*\|\s*(sudo\s+)?(ba|z|da)?sh\b",
    r"\bmkfs(\.\w+)?\b",
    r":\(\)\s*\{",
    r"\bdd\b[^\n]*\bof=/dev/",
    r"(?i)\bformat(\.com)?\s+[a-z]:",
    r"(?i)\b(rd|rmdir)\s+/s\b",
    r"(?i)\bRemove-Item\b[^\n]*-Recurse",
];

/// One observe → act turn of the Computer Use Agent.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct AgentStep {
    /// 1-based turn number.
    pub index: u32,
    /// Summary of the observed screen before the actions.
    pub observation: String,
    /// Text the model wrote next to its tool calls.
    pub assistant: String,
    pub actions: Vec<AgentAction>,
    /// Optional action selection or reason for falling back to the vision model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<super::decision::AgentDecision>,
    pub duration_ms: u64,
}

/// One tool call of a turn.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct AgentAction {
    pub tool: String,
    /// Arguments passed to the action; coordinates are pixels of the observed screenshot.
    pub args: Value,
    /// Desktop input coordinates the action used.
    #[serde(default)]
    pub input_points: Vec<InputPoint>,
    /// Not run: an earlier call of the turn failed, the turn limit was reached or the task ended.
    #[serde(default)]
    pub skipped: bool,
    pub ok: bool,
    /// What happened, or why it failed.
    pub message: String,
}

#[crate::register_node]
#[derive(Default)]
pub struct ComputerUseAgentNode;

impl ComputerUseAgentNode {
    pub fn new() -> Self {
        Self
    }
}

#[cfg(feature = "execute")]
fn bounded(label: &str, value: i64, min: i64, max: i64) -> flow_like_types::Result<i64> {
    if (min..=max).contains(&value) {
        Ok(value)
    } else {
        Err(flow_like_types::anyhow!(
            "{} must be between {} and {}, got {}",
            label,
            min,
            max,
            value
        ))
    }
}

#[cfg(feature = "execute")]
struct Inputs {
    session: AutomationSession,
    model: Bit,
    goal: String,
    display_index: i64,
    target_window: String,
    extra_instructions: String,
    settings: super::episode::Settings,
}

#[cfg(feature = "execute")]
async fn read_inputs(context: &mut ExecutionContext) -> flow_like_types::Result<Inputs> {
    use std::time::Duration;

    let session: AutomationSession = context.evaluate_pin("session").await?;
    session.ensure_active(context).await?;
    let goal: String = context.evaluate_pin("goal").await?;
    if goal.trim().is_empty() {
        return Err(flow_like_types::anyhow!(
            "The goal is empty; describe the task for the agent"
        ));
    }
    let perception = context.evaluate_pin::<String>("perception").await?;
    let max_steps = context.evaluate_pin("max_steps").await?;
    let max_duration = context.evaluate_pin("max_duration_s").await?;
    let max_actions = context.evaluate_pin("max_actions_per_step").await?;
    let settle_ms = context.evaluate_pin("settle_ms").await?;
    let forbidden = context
        .evaluate_pin::<Vec<String>>("forbidden_text")
        .await?;
    let decision_bit = crate::llm::optional_pin::<Bit>(context, "decision_model").await?;
    let decision_model = match decision_bit {
        Some(bit) => {
            context.check_cancelled()?;
            let app_state = context.app_state.clone();
            let token = context.token.clone();
            let usage_context = context.model_usage_context();
            let built = context
                .run_cancellable(async move {
                    app_state
                        .model_factory
                        .build_systemone(&bit, app_state.clone(), token, usage_context)
                        .await
                })
                .await?;
            context.check_cancelled()?;
            match built {
                Ok(model) => Some(model),
                Err(error) => {
                    context.log_message(
                        &format!(
                            "Could not load the computer use Decision Model; continuing with the vision model: {error}"
                        ),
                        flow_like::flow::execution::LogLevel::Warn,
                    );
                    None
                }
            }
        }
        None => None,
    };
    let settings = super::episode::Settings {
        decision_model,
        perception: super::observe::Perception::parse(&perception)?,
        max_steps: bounded("Max Steps", max_steps, 1, 500)? as u32,
        max_duration: Duration::from_secs(bounded("Max Duration", max_duration, 1, 86_400)? as u64),
        max_actions: bounded("Max Actions per Step", max_actions, 1, 20)? as usize,
        settle: Duration::from_millis(bounded("Settle Time", settle_ms, 0, 10_000)? as u64),
        forbidden: super::tools::ForbiddenText::new(&forbidden)?,
    };
    Ok(Inputs {
        model: context.evaluate_pin("model").await?,
        display_index: context.evaluate_pin("display_index").await?,
        target_window: context.evaluate_pin("target_window").await?,
        extra_instructions: context.evaluate_pin("extra_instructions").await?,
        session,
        goal,
        settings,
    })
}

#[cfg(feature = "execute")]
async fn write_outputs(
    context: &mut ExecutionContext,
    session: &AutomationSession,
    outcome: super::episode::Outcome,
) -> flow_like_types::Result<()> {
    context.log_message(
        &format!(
            "Computer use agent finished with status {} after {} steps",
            outcome.status.as_str(),
            outcome.steps.len()
        ),
        flow_like::flow::execution::LogLevel::Info,
    );
    let final_image = NodeImage::new(
        context,
        image::DynamicImage::ImageRgba8(outcome.final_shot.image),
    )
    .await;
    context.set_pin_value("session_out", json!(session)).await?;
    context
        .set_pin_value("status", json!(outcome.status.as_str()))
        .await?;
    context
        .set_pin_value("answer", json!(outcome.answer))
        .await?;
    context.set_pin_value("steps", json!(outcome.steps)).await?;
    context
        .set_pin_value("final_image", json!(final_image))
        .await?;
    context
        .set_pin_value("final_frame", json!(outcome.final_shot.frame))
        .await?;
    context.activate_exec_pin(outcome.status.exec_pin()).await?;
    Ok(())
}

#[async_trait]
impl NodeLogic for ComputerUseAgentNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "computer_use_agent",
            "Computer Use Agent",
            "Operate the desktop toward a goal using a vision model with tool calling. An optional Decision Model selects routine actions from observed elements; the vision model handles complex interactions and confirms completion. Stops when done, when input is needed, or when stuck or out of steps or time",
            "Automation/Computer/Agent",
        );
        node.set_version(4);
        node.set_flowscript_name("computer", "useAgent");
        node.add_icon("/flow/icons/bot-invoke.svg");
        node.set_scores(
            NodeScores::new()
                .set_privacy(2)
                .set_security(3)
                .set_performance(3)
                .set_governance(4)
                .set_reliability(5)
                .set_cost(3)
                .build(),
        );
        node.set_only_offline(true);
        node.set_long_running(true);

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "session",
            "Session",
            "Computer session handle",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();
        node.add_input_pin(
            "model",
            "Model",
            "Vision model with tool calling that operates the desktop",
            VariableType::Struct,
        )
        .set_schema::<Bit>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "goal",
            "Goal",
            "The task in plain language, e.g. 'Rename report.txt on the Desktop to final.txt'",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "display_index",
            "Display Index",
            "Display the agent sees and acts on; -1 is the primary display",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(-1)));
        node.add_input_pin(
            "target_window",
            "Target Window",
            "Optional window title (or part of it): the agent sees the display showing this window and its elements are the numbered ones. Empty uses the focused window outside Flow-Like",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "perception",
            "Perception",
            "screenshot: the plain screenshot; screenshot_marks: numbered boxes on accessibility elements plus an element list; screenshot_marks_ocr: also number recognized text",
            VariableType::String,
        )
        .set_default_value(Some(json!(PERCEPTION_MARKS)))
        .set_options(
            PinOptions::new()
                .set_valid_values(
                    [PERCEPTION_SCREENSHOT, PERCEPTION_MARKS, PERCEPTION_MARKS_OCR]
                        .iter()
                        .map(|s| s.to_string())
                        .collect(),
                )
                .build(),
        );
        for (name, friendly, description, default, max) in [
            (
                "max_steps",
                "Max Steps",
                "Most model turns before the agent stops with max_steps (1–500)",
                30,
                500,
            ),
            (
                "max_duration_s",
                "Max Duration (s)",
                "Wall-clock budget in seconds before the agent stops with timeout (1–86400)",
                600,
                86_400,
            ),
            (
                "max_actions_per_step",
                "Max Actions per Step",
                "Most tool calls executed per model turn (1–20)",
                5,
                20,
            ),
        ] {
            node.add_input_pin(name, friendly, description, VariableType::Integer)
                .set_default_value(Some(json!(default)))
                .set_options(
                    PinOptions::new()
                        .set_range((1.0, max as f64))
                        .set_step(1.0)
                        .build(),
                );
        }
        node.add_input_pin(
            "settle_ms",
            "Settle Time (ms)",
            "Longest wait after actions for the screen to stop changing before the next screenshot (0–10000)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(600)))
        .set_options(
            PinOptions::new()
                .set_range((0.0, 10_000.0))
                .set_step(50.0)
                .build(),
        );
        node.add_input_pin(
            "forbidden_text",
            "Forbidden Text",
            "Regular expressions the agent may never type; a matching type call is refused",
            VariableType::String,
        )
        .set_value_type(ValueType::Array)
        .set_default_value(Some(json!(DEFAULT_FORBIDDEN_TEXT)));
        node.add_input_pin(
            "extra_instructions",
            "Extra Instructions",
            "Optional guidance added to the task, e.g. which app to use or what to avoid",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "decision_model",
            "Decision Model",
            "Optional SystemOne model from Find Decision Model or Load Bit. Chooses observed elements and routine actions from accessibility/OCR text. The vision model handles text entry, planning, verification and fallback. Leave empty for the vision model alone",
            VariableType::Struct,
        )
        .set_schema::<Bit>()
        .set_default_value(Some(json!(null)))
        .set_options(PinOptions::new().set_enforce_schema(true).set_optional(true).build());

        node.add_output_pin(
            "exec_out",
            "Done",
            "The model reported the goal as achieved",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_failed",
            "Failed",
            "The model reported failure, the agent was stuck, ran out of steps or time, or hit an error",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_needs_input",
            "Needs Input",
            "The model asked the user a question; it is in Answer",
            VariableType::Execution,
        );
        node.add_output_pin(
            "session_out",
            "Session",
            "Computer session handle (pass-through)",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();
        node.add_output_pin(
            "status",
            "Status",
            "done, failed, stuck, max_steps, timeout or needs_input",
            VariableType::String,
        );
        node.add_output_pin(
            "answer",
            "Answer",
            "The model's result or summary, the question for the user, or why the agent stopped",
            VariableType::String,
        );
        node.add_output_pin(
            "steps",
            "Steps",
            "Trajectory: per turn the screenshot seen, model text, action results, optional decision selection or fallback reason, and duration",
            VariableType::Struct,
        )
        .set_schema::<AgentStep>()
        .set_value_type(ValueType::Array);
        node.add_output_pin(
            "final_image",
            "Final Screenshot",
            "The last screenshot, without marks",
            VariableType::Struct,
        )
        .set_schema::<NodeImage>();
        node.add_output_pin(
            "final_frame",
            "Final Frame",
            "Desktop rectangle and pixel size of the final screenshot",
            VariableType::Struct,
        )
        .set_schema::<ScreenFrame>();
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use super::episode::{self, ModelAgent};
        use super::observe::{Target, capture, observe};
        use super::prompt::{Briefing, preamble, task_text};
        use super::tools::definitions;

        for pin in ["exec_out", "exec_failed", "exec_needs_input"] {
            context.deactivate_exec_pin(pin).await?;
        }
        let inputs = read_inputs(context).await?;
        drop(inputs.session.create_enigo(context).await?);
        let target = Target::new(inputs.display_index, &inputs.target_window);
        let located = target.locate().await?;
        if let Some(warning) = located.warning {
            return Err(flow_like_types::anyhow!(warning));
        }

        let briefing = Briefing {
            goal: &inputs.goal,
            extra_instructions: &inputs.extra_instructions,
            target_window: located.window.as_ref().map(|_| inputs.target_window.trim()),
            marks: inputs.settings.perception.marks(),
            max_actions: inputs.settings.max_actions,
        };
        let agent: ModelAgent = inputs
            .model
            .agent(context, &None)
            .await?
            .preamble(&preamble(&briefing))
            .tools(definitions(briefing.marks))
            .tool_choice(rig::message::ToolChoice::Required)
            .build();
        let now = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
        let task = task_text(&briefing, &now);

        let first = capture(located.display_index).await?;
        let observation = observe(first, &located, inputs.settings.perception).await?;
        let outcome = episode::run(
            context,
            &inputs.session,
            &agent,
            &target,
            &inputs.settings,
            task,
            observation,
        )
        .await?;
        write_outputs(context, &inputs.session, outcome).await
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Computer automation requires the 'execute' feature"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like::flow::board::cleanup::sync_node_schema::sync_node_with_catalog;

    #[test]
    fn decision_model_upgrade_preserves_existing_inputs_and_connections() {
        let catalog = ComputerUseAgentNode.get_node();
        let mut placed = catalog.clone();
        placed.version = Some(2);
        placed.pins.retain(|_, pin| pin.name != "decision_model");
        placed
            .get_pin_mut_by_name("model")
            .unwrap()
            .depends_on
            .insert("vision-model".into());
        placed
            .get_pin_mut_by_name("goal")
            .unwrap()
            .set_default_value(Some(json!("Read the current document")));
        placed
            .get_pin_mut_by_name("exec_out")
            .unwrap()
            .connected_to
            .insert("next-step".into());
        let original = placed.clone();
        sync_node_with_catalog(&mut placed, &catalog);
        for before in original.pins.values() {
            let after = placed.get_pin_by_name(&before.name).unwrap();
            assert_eq!(after.id, before.id);
            assert_eq!(after.index, before.index);
            assert_eq!(after.default_value, before.default_value);
            assert_eq!(after.depends_on, before.depends_on);
            assert_eq!(after.connected_to, before.connected_to);
        }
        let decision = placed.get_pin_by_name("decision_model").unwrap();
        assert!(
            decision.index
                > original
                    .get_pin_by_name("extra_instructions")
                    .unwrap()
                    .index
        );
        assert!(
            flow_like_types::json::from_slice::<Value>(decision.default_value.as_ref().unwrap())
                .unwrap()
                .is_null()
        );
        assert_eq!(placed.version, Some(4));
    }

    #[test]
    fn action_selection_upgrade_preserves_a_connected_decision_model() {
        let catalog = ComputerUseAgentNode.get_node();
        let mut placed = catalog.clone();
        placed.version = Some(3);
        let decision = placed.get_pin_mut_by_name("decision_model").unwrap();
        decision.depends_on.insert("decision-model".into());
        placed
            .get_pin_mut_by_name("perception")
            .unwrap()
            .set_default_value(Some(json!(PERCEPTION_SCREENSHOT)));
        let original = placed.clone();

        sync_node_with_catalog(&mut placed, &catalog);

        for before in original.pins.values() {
            let after = placed.get_pin_by_name(&before.name).unwrap();
            assert_eq!(after.id, before.id);
            assert_eq!(after.index, before.index);
            assert_eq!(after.default_value, before.default_value);
            assert_eq!(after.depends_on, before.depends_on);
            assert_eq!(after.connected_to, before.connected_to);
        }
        assert_eq!(placed.version, Some(4));
    }

    #[test]
    fn old_step_traces_deserialize_without_a_decision() {
        let value = json!({"index":1,"observation":"original screen","assistant":"","actions":[],"duration_ms":100});
        let step: AgentStep = flow_like_types::json::from_value(value.clone()).unwrap();
        assert!(step.decision.is_none());
        assert_eq!(json!(step), value);
    }
}

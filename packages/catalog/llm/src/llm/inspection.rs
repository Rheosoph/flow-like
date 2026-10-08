use flow_like::{
    bit::Bit,
    flow::{
        execution::context::ExecutionContext,
        node::{Node, NodeLogic},
        pin::PinOptions,
        variable::VariableType,
    },
};
use flow_like_ml_core::{Annotation, InspectionSpec, LabelProvenance, Sample};
use flow_like_model_provider::{history::History, response::LLMUsageStats};
use flow_like_types::{Result, async_trait, json::json};

fn base(id: &str, name: &str, description: &str, function: &str) -> Node {
    let mut node = Node::new(id, name, description, "AI/ML/Inspection");
    node.set_flowscript_name("inspection", function);
    node.add_icon("/flow/icons/bot-invoke.svg");
    node.set_long_running(true);
    node.add_input_pin(
        "exec_in",
        "Input",
        "Start evaluation",
        VariableType::Execution,
    );
    node.add_input_pin(
        "model",
        "Teacher Model",
        "A model supporting structured tool output",
        VariableType::Struct,
    )
    .set_schema::<Bit>()
    .set_options(PinOptions::new().set_enforce_schema(true).build());
    node.add_input_pin(
        "history",
        "History",
        "Inspection description and relevant images or sensor evidence",
        VariableType::Struct,
    )
    .set_schema::<History>()
    .set_options(PinOptions::new().set_enforce_schema(true).build());
    node.add_output_pin(
        "exec_out",
        "Done",
        "Validated output is available",
        VariableType::Execution,
    );
    node.add_output_pin(
        "stats",
        "Usage",
        "Teacher model usage",
        VariableType::Struct,
    )
    .set_schema::<LLMUsageStats>();
    node
}

#[crate::register_node]
#[derive(Default)]
pub struct PlanInspectionTrainingNode;

#[async_trait]
impl NodeLogic for PlanInspectionTrainingNode {
    fn get_node(&self) -> Node {
        let mut node = base(
            "ml_plan_inspection_training",
            "Plan Inspection Training",
            "Select a supported Burn architecture and bounded training configuration from an inspection specification",
            "planTraining",
        );
        node.add_input_pin(
            "spec",
            "Inspection",
            "Task, input dimensions and label order",
            VariableType::Struct,
        )
        .set_schema::<InspectionSpec>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_output_pin(
            "config",
            "Training Configuration",
            "Validated architecture and optimizer configuration",
            VariableType::Struct,
        )
        .set_schema::<flow_like_ml_burn::TrainingConfig>();
        node
    }
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        #[cfg(feature = "execute")]
        {
            context.deactivate_exec_pin("exec_out").await?;
            let model: Bit = context.evaluate_pin("model").await?;
            let history: History = context.evaluate_pin("history").await?;
            let spec: InspectionSpec = context.evaluate_pin("spec").await?;
            spec.validate()?;
            let schema =
                json!(schemars::schema_for!(flow_like_ml_burn::TrainingConfig)).to_string();
            let hint = format!(
                "Choose a supported trainable architecture for this inspection: {}. The input shape excludes batch. Sensor sequences are [time,channels]; images are [channels,height,width]. Preserve class order and set output count to label count for classification. Prefer a small LSTM or TCN for temporal dependencies, MLP for engineered sensor features, ResNet18 for images, YOLOX for boxes, U-Net for semantic masks, and reconstruction models for anomaly tasks. Use the auto backend unless the user requests a specific backend. Auto probes the GPU backends available in the installed build and falls back to CPU. Start with at most 100 epochs, batch size at most 128, hidden size at most 128 and a fixed seed. Do not claim predicted accuracy or invent future outcomes.",
                json!(spec)
            );
            let (value, stats) = super::llm_extractor_history::extract_structured_history(
                context, model, history, schema, hint,
            )
            .await?;
            let config: flow_like_ml_burn::TrainingConfig =
                flow_like_types::json::from_value(value)?;
            config.validate_planning_budget()?;
            flow_like_ml_burn::validate_inspection_config(&spec, &config)?;
            context.set_pin_value("config", json!(config)).await?;
            context.set_pin_value("stats", json!(stats)).await?;
            context.activate_exec_pin("exec_out").await?;
            Ok(())
        }
        #[cfg(not(feature = "execute"))]
        {
            let _ = context;
            Err(flow_like_types::anyhow!(
                "Training planning requires LLM execution"
            ))
        }
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct BuildInspectionSpecNode;

#[async_trait]
impl NodeLogic for BuildInspectionSpecNode {
    fn get_node(&self) -> Node {
        let mut node = base(
            "ml_build_inspection_spec",
            "Build Inspection Spec",
            "Turn an inspection description into a validated task, label schema and input contract",
            "buildSpec",
        );
        node.add_output_pin(
            "spec",
            "Inspection",
            "Validated task specification",
            VariableType::Struct,
        )
        .set_schema::<InspectionSpec>();
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        #[cfg(feature = "execute")]
        {
            context.deactivate_exec_pin("exec_out").await?;
            let model: Bit = context.evaluate_pin("model").await?;
            let history: History = context.evaluate_pin("history").await?;
            let schema = json!(schemars::schema_for!(InspectionSpec)).to_string();
            let hint = "Describe the inspection as a task specification. Preserve the user's class order and sensor/image dimensions. Use only information provided in the history. Do not invent measured performance. A future event requires an explicit prediction horizon. Give the specification a stable descriptive identifier.".to_string();
            let (value, stats) = super::llm_extractor_history::extract_structured_history(
                context, model, history, schema, hint,
            )
            .await?;
            let spec: InspectionSpec = flow_like_types::json::from_value(value)?;
            spec.validate()?;
            context.set_pin_value("spec", json!(spec)).await?;
            context.set_pin_value("stats", json!(stats)).await?;
            context.activate_exec_pin("exec_out").await?;
            Ok(())
        }
        #[cfg(not(feature = "execute"))]
        {
            let _ = context;
            Err(flow_like_types::anyhow!(
                "Inspection planning requires LLM execution"
            ))
        }
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct TeacherLabelNode;

#[async_trait]
impl NodeLogic for TeacherLabelNode {
    fn get_node(&self) -> Node {
        let mut node = base(
            "ml_teacher_label",
            "Teacher Label",
            "Label supplied inspection evidence and preserve the teacher provenance",
            "teacherLabel",
        );
        node.add_input_pin(
            "spec",
            "Inspection",
            "Task and class order",
            VariableType::Struct,
        )
        .set_schema::<InspectionSpec>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "sample",
            "Sample",
            "Sample whose evidence is included in History",
            VariableType::Struct,
        )
        .set_schema::<Sample>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_output_pin(
            "labelled_sample",
            "Labelled Sample",
            "Teacher label with model and prompt provenance",
            VariableType::Struct,
        )
        .set_schema::<Sample>();
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        #[cfg(feature = "execute")]
        {
            context.deactivate_exec_pin("exec_out").await?;
            let model: Bit = context.evaluate_pin("model").await?;
            let history: History = context.evaluate_pin("history").await?;
            let spec: InspectionSpec = context.evaluate_pin("spec").await?;
            let mut sample: Sample = context.evaluate_pin("sample").await?;
            spec.validate()?;
            sample.input.validate(16_777_216)?;
            if spec.prediction_horizon_ms.is_some_and(|h| h > 0) {
                return Err(flow_like_types::anyhow!(
                    "Future-event labels require observed outcomes. Use Attach Observed Outcome after the prediction horizon."
                ));
            }
            let schema = json!(schemars::schema_for!(Annotation)).to_string();
            let hint = format!(
                "Label only the evidence for sample {} according to this inspection: {}. Class IDs are zero-based indices into labels. Box coordinates are normalized to [0,1]. Return unlabeled when the evidence cannot establish a label. Do not infer events that have not happened.",
                sample.id,
                json!(spec)
            );
            let prompt_digest = flow_like_ml_core::content_digest(&flow_like_types::json::to_vec(
                &(hint.clone(), &history),
            )?);
            let model_id = model.id.clone();
            let (value, stats) = super::llm_extractor_history::extract_structured_history(
                context, model, history, schema, hint,
            )
            .await?;
            sample.annotation = flow_like_types::json::from_value(value)?;
            sample.provenance = LabelProvenance::Teacher {
                model: model_id,
                prompt_digest,
                confidence: None,
            };
            if matches!(sample.annotation, Annotation::Unlabeled) {
                sample.validate(spec.labels.len(), 16_777_216)?;
                if sample.input.shape != spec.input_shape {
                    return Err(flow_like_types::anyhow!(
                        "Sample shape differs from inspection input"
                    ));
                }
            } else {
                spec.validate_sample(&sample)?;
            }
            context
                .set_pin_value("labelled_sample", json!(sample))
                .await?;
            context.set_pin_value("stats", json!(stats)).await?;
            context.activate_exec_pin("exec_out").await?;
            Ok(())
        }
        #[cfg(not(feature = "execute"))]
        {
            let _ = context;
            Err(flow_like_types::anyhow!(
                "Teacher labels require LLM execution"
            ))
        }
    }
}

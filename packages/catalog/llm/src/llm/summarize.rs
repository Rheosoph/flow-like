use flow_like::{
    bit::Bit,
    flow::{
        execution::context::ExecutionContext,
        node::{Node, NodeLogic, NodeScores},
        pin::PinOptions,
        variable::VariableType,
    },
};
use flow_like_types::{async_trait, json::json};

#[cfg(feature = "execute")]
use flow_like::flow::execution::{LogLevel, log::LogMessage};
#[cfg(feature = "execute")]
use flow_like_model_provider::summarization::{
    ChunkingMethod, DensificationStrategy, SummarizationConfig, SummarizationStrategy,
};

#[crate::register_node]
#[derive(Default)]
pub struct SummarizeNode {}

impl SummarizeNode {
    pub fn new() -> Self {
        SummarizeNode {}
    }
}

#[async_trait]
impl NodeLogic for SummarizeNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_llm_summarize",
            "Summarize",
            "Summarizes long text using an LLM with configurable strategies. Supports Map-Reduce (parallel, fast), Refine (sequential, coherent), Hierarchical (structure-aware), Hybrid (parallel + coherent), and Sliding Window (memory-efficient). Optional Chain of Density post-processing for optimal information density.",
            "AI/Generative",
        );
        node.set_flowscript_name("ai", "summarize");
        node.add_icon("/flow/icons/bot-invoke.svg");
        node.set_version(4);

        node.set_scores(
            NodeScores::new()
                .set_privacy(5)
                .set_security(5)
                .set_performance(6)
                .set_governance(5)
                .set_reliability(7)
                .set_cost(5)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger",
            VariableType::Execution,
        );

        node.add_input_pin(
            "model",
            "Model",
            "Bit describing the provider/model to use for summarization",
            VariableType::Struct,
        )
        .set_schema::<Bit>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "text",
            "Text",
            "The long text to summarize (markdown supported)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "strategy",
            "Strategy",
            "Summarization strategy:\n\
             • Refine — sequential, best coherence, no parallelism\n\
             • MapReduce — parallel chunking, fast, may lose cross-chunk context\n\
             • Hierarchical — structure-aware tree, best for headed documents\n\
             • Hybrid — MapReduce speed + Refine coherence polish\n\
             • SlidingWindow — fixed memory buffer, best for very long documents",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "Refine".to_string(),
                    "MapReduce".to_string(),
                    "Hierarchical".to_string(),
                    "Hybrid".to_string(),
                    "SlidingWindow".to_string(),
                ])
                .build(),
        )
        .set_default_value(Some(json!("Refine")));

        node.add_input_pin(
            "densification",
            "Densification",
            "Post-processing to increase information density:\n\
             • None — use the strategy output as-is\n\
             • ChainOfDensity — iteratively compress to optimal density (~0.15 entities/token)",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["None".to_string(), "ChainOfDensity".to_string()])
                .build(),
        )
        .set_default_value(Some(json!("None")));

        node.add_input_pin(
            "instructions",
            "Instructions",
            "Optional focus instructions (e.g. 'focus on action items', 'use bullet points')",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "prior_summary",
            "Prior Summary",
            "Optional existing summary to build upon (used as initial context for Refine/Hybrid/SlidingWindow strategies)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "chunk_size",
            "Chunk Size",
            "Maximum characters per chunk. Reduce for models with smaller context windows (default: 8000)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(8000)));

        node.add_input_pin(
            "chunk_overlap",
            "Chunk Overlap %",
            "Overlap between adjacent chunks as percentage (0-50). Prevents information loss at boundaries (default: 10)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(10)));

        node.add_input_pin(
            "track_entities",
            "Track Entities",
            "Extract and track named entities across chunks to prevent information loss. Adds 2-3 extra LLM calls but improves factual preservation.",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_input_pin(
            "concurrency",
            "Concurrency",
            "Parallel requests for MapReduce/Hybrid strategies. 0 = unlimited, 1 = sequential (default: 4)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(4)));

        node.add_input_pin(
            "max_iterations",
            "Max Iterations",
            "Safety limit on summarization passes. Each pass reduces total length (default: 5)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(5)));

        node.add_input_pin(
            "density_steps",
            "Density Steps",
            "Number of Chain of Density refinement steps when densification is enabled (1-5, default: 3). Research shows step 3 is the human-preferred sweet spot.",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(3)));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Fires once summarization is complete",
            VariableType::Execution,
        );

        node.add_output_pin(
            "summary",
            "Summary",
            "The final summarized text",
            VariableType::String,
        );

        node.add_output_pin(
            "entities",
            "Entities",
            "Tracked entities found in the document (only populated when Track Entities is enabled)",
            VariableType::String,
        )
        .set_value_type(flow_like::flow::pin::ValueType::Array);

        node.add_output_pin(
            "llm_calls",
            "LLM Calls",
            "Total number of LLM invocations used during summarization",
            VariableType::Integer,
        );

        node.set_long_running(true);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let model_bit = context.evaluate_pin::<Bit>("model").await?;
        let text = context.evaluate_pin::<String>("text").await?;
        let strategy_str = context.evaluate_pin::<String>("strategy").await?;
        let densification_str = context.evaluate_pin::<String>("densification").await?;
        let instructions = context.evaluate_pin::<String>("instructions").await?;
        let prior_summary = context.evaluate_pin::<String>("prior_summary").await?;
        let chunk_size = context.evaluate_pin::<i64>("chunk_size").await?;
        let chunk_overlap = context.evaluate_pin::<i64>("chunk_overlap").await?;
        let track_entities = context.evaluate_pin::<bool>("track_entities").await?;
        let concurrency = context.evaluate_pin::<i64>("concurrency").await?;
        let max_iterations = context.evaluate_pin::<i64>("max_iterations").await?;
        let density_steps = context.evaluate_pin::<i64>("density_steps").await?;

        let strategy = SummarizationStrategy::try_from(strategy_str.as_str()).unwrap_or_default();
        let densification =
            DensificationStrategy::try_from(densification_str.as_str()).unwrap_or_default();

        let mut model_name = model_bit.id.clone();
        if let Some(meta) = model_bit.meta.get("en") {
            model_name = meta.name.clone();
        }

        let model_factory = context.app_state.model_factory.clone();
        let model = model_factory
            .lock()
            .await
            .build(
                &model_bit,
                context.app_state.clone(),
                context.token.clone(),
                context.model_usage_context(),
            )
            .await?;

        let config = SummarizationConfig {
            strategy,
            densification,
            chunking: ChunkingMethod::Markdown,
            chunk_size: chunk_size as usize,
            chunk_overlap_percent: (chunk_overlap as u8).min(50),
            max_iterations: max_iterations as u32,
            track_entities,
            instructions,
            prior_summary,
            concurrency: concurrency as usize,
            density_steps: density_steps as u32,
            memory_budget_ratio: 0.4,
        };

        let mut log = LogMessage::new(
            &format!(
                "Summarizing {} chars with {} strategy, model {}",
                text.len(),
                config.strategy.as_str(),
                model_name
            ),
            LogLevel::Info,
            None,
        );

        let result = flow_like_model_provider::summarization::summarize(
            &text,
            &config,
            model.as_ref(),
            &model_name,
        )
        .await?;

        log.end();
        context.log(log);

        context
            .set_pin_value("summary", json!(result.summary))
            .await?;
        context
            .set_pin_value("entities", json!(result.entities))
            .await?;
        context
            .set_pin_value("llm_calls", json!(result.stats.llm_calls as i64))
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "This node requires the 'execute' feature"
        ))
    }
}

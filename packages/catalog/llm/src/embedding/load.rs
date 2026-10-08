use flow_like::{
    bit::{Bit, BitTypes},
    flow::{
        execution::context::ExecutionContext,
        node::{Node, NodeLogic},
        pin::PinOptions,
        variable::VariableType,
    },
};
use flow_like_types::{async_trait, bail, json::json};
use std::sync::Arc;

use super::{CachedEmbeddingModel, CachedEmbeddingModelObject};

#[crate::register_node]
#[derive(Default)]
pub struct LoadModelNode {}

impl LoadModelNode {
    pub fn new() -> Self {
        LoadModelNode {}
    }
}

#[async_trait]
impl NodeLogic for LoadModelNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "load_model",
            "Load Embedding Model",
            "Loads a model from a Bit",
            "AI/Embedding",
        );
        node.set_flowscript_name("ai.embedding", "loadModel");

        node.add_icon("/flow/icons/bot-invoke.svg");
        node.set_version(4);
        node.set_long_running(true);

        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
        );

        node.add_input_pin(
            "bit",
            "Model Bit",
            "The Bit that contains the Model",
            VariableType::Struct,
        )
        .set_schema::<Bit>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exec_out",
            "Output",
            "Done with the Execution",
            VariableType::Execution,
        );

        node.add_output_pin("model", "Model", "Model Out", VariableType::Struct)
            .set_schema::<CachedEmbeddingModel>();

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let bit: Bit = context.evaluate_pin("bit").await?;

        if bit.bit_type != BitTypes::Embedding && bit.bit_type != BitTypes::ImageEmbedding {
            bail!("Not an Embedding Model");
        }

        let versioned =
            flow_like::models::embedding_factory::versioned_embedding_spec(&bit)?.is_some();
        let app_state = context.app_state.clone();
        let model_factory = context.app_state.embedding_factory.clone();

        if !versioned
            && bit.bit_type == BitTypes::ImageEmbedding
            && !flow_like::models::embedding_factory::prefers_local_execution(
                &bit,
                &context.app_state,
            )
            .await
        {
            bail!(
                "Image embedding execution requires local ML and a filesystem-backed Bit store; remote image embedding is not supported"
            );
        }

        let unified = model_factory
            .build(
                &bit,
                app_state.clone(),
                context.token.clone(),
                context.model_usage_context(),
            )
            .await?;
        let cache_key = format!(
            "embedding:{}:{}",
            bit.id,
            unified.descriptor().pipeline_fingerprint
        );
        if let Some(cached) = context.get_cache(&cache_key).await
            && cached.as_any().is::<CachedEmbeddingModelObject>()
        {
            let model = CachedEmbeddingModel {
                cache_key,
                model_type: bit.bit_type.clone(),
            };
            context.set_pin_value("model", json!(model)).await?;
            context.activate_exec_pin("exec_out").await?;
            return Ok(());
        }

        let model = if versioned {
            let text = model_factory
                .build_text_routed(
                    &bit,
                    app_state.clone(),
                    context.token.clone(),
                    context.model_usage_context(),
                )
                .await?;
            let image = if !unified.supports_encoded_media()
                && unified.descriptor().modalities.contains(
                    &flow_like_model_provider::embedding::interface::EmbeddingModality::Image,
                ) {
                Some(model_factory.build_image(&bit, app_state).await?)
            } else {
                None
            };
            CachedEmbeddingModelObject {
                text_model: Some(text),
                image_model: image,
                model: Some(unified),
            }
        } else {
            match bit.bit_type {
                BitTypes::Embedding => {
                    let model = model_factory
                        .build_text_routed(
                            &bit,
                            app_state,
                            context.token.clone(),
                            context.model_usage_context(),
                        )
                        .await?;

                    CachedEmbeddingModelObject {
                        text_model: Some(model),
                        image_model: None,
                        model: Some(unified),
                    }
                }
                BitTypes::ImageEmbedding => {
                    let model = model_factory.build_image(&bit, app_state).await?;

                    CachedEmbeddingModelObject {
                        text_model: None,
                        image_model: Some(model),
                        model: Some(unified),
                    }
                }
                _ => {
                    bail!("Unsupported Bit Type");
                }
            }
        };

        context.set_cache(&cache_key, Arc::new(model)).await;
        let model = CachedEmbeddingModel {
            cache_key,
            model_type: bit.bit_type.clone(),
        };

        context.set_pin_value("model", json!(model)).await?;
        context.activate_exec_pin("exec_out").await?;

        return Ok(());
    }
}

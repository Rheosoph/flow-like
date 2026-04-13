use std::collections::HashMap;
use std::sync::Arc;

use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::FlowPath;
use flow_like_catalog_data::events::chat_event::Attachment;
use flow_like_model_provider::history::{Content, History, MessageContent};
use flow_like_storage::Path;
use flow_like_storage::files::store::FlowLikeStore;
use flow_like_types::Cacheable;
use flow_like_types::{async_trait, json::json};

fn extract_image_urls(history: &History) -> Vec<String> {
    history
        .messages
        .last()
        .and_then(|m| match &m.content {
            MessageContent::Contents(contents) => Some(contents),
            _ => None,
        })
        .map(|contents| {
            contents
                .iter()
                .filter_map(|c| match c {
                    Content::Image { image_url, .. } => Some(image_url.url.clone()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

fn sanitize_for_path(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '-'
            }
        })
        .collect()
}

fn deduplicate_name(name: &str, used_names: &mut HashMap<String, u32>) -> String {
    let count = used_names.entry(name.to_string()).or_insert(0);
    *count += 1;
    if *count == 1 {
        return name.to_string();
    }
    let idx = *count - 1;
    let path = std::path::Path::new(name);
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or(name);
    let ext = path.extension().and_then(|s| s.to_str());
    match ext {
        Some(e) => format!("{}-{}.{}", stem, idx, e),
        None => format!("{}-{}", stem, idx),
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct ExtractAttachments {}

impl ExtractAttachments {
    pub fn new() -> Self {
        ExtractAttachments {}
    }
}

#[async_trait]
impl NodeLogic for ExtractAttachments {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_gen_llm_history_extract_attachments",
            "Extract Attachments",
            "Pulls down image attachments referenced in the latest chat message",
            "Events/Chat",
        );
        node.add_icon("/flow/icons/paperclip.svg");
        node.set_version(1);
        node.set_scores(
            NodeScores::new()
                .set_privacy(7)
                .set_security(7)
                .set_performance(7)
                .set_reliability(8)
                .set_governance(7)
                .set_cost(9)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Begin execution to sync attachments",
            VariableType::Execution,
        );

        node.add_input_pin(
            "history",
            "History",
            "Chat history whose final message may contain image parts",
            VariableType::Struct,
        )
        .set_schema::<History>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "attachments",
            "Attachments",
            "Existing attachments to merge with new downloads",
            VariableType::Struct,
        )
        .set_schema::<Attachment>()
        .set_default_value(Some(flow_like_types::json::json!([])))
        .set_value_type(flow_like::flow::pin::ValueType::Array)
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exec_out",
            "Output",
            "Signals completion once attachments are cached",
            VariableType::Execution,
        );

        node.add_output_pin(
            "paths",
            "Paths",
            "Virtual file paths pointing to cached attachments",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_value_type(flow_like::flow::pin::ValueType::Array)
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let mut attachments = context
            .evaluate_pin::<Vec<Attachment>>("attachments")
            .await?;

        if let Ok(history) = context.evaluate_pin::<History>("history").await {
            let urls = extract_image_urls(&history);
            attachments.extend(urls.into_iter().map(Attachment::Url));
        }

        let id = context.id.clone();
        let cache_path = format!("virtual_dir_{}", id);

        if !context.has_cache(&cache_path).await {
            let store = FlowLikeStore::Memory(Arc::new(
                flow_like_storage::object_store::memory::InMemory::new(),
            ));
            let store: Arc<dyn Cacheable> = Arc::new(store);
            context.set_cache(&cache_path, store).await;
        }

        let mut paths = Vec::with_capacity(attachments.len());
        let virtual_path = FlowPath {
            path: "".to_string(),
            store_ref: cache_path.clone(),
            cache_store_ref: None,
        };

        let runtime = virtual_path.to_runtime(context).await?;
        let store = runtime.store;
        let generic_store = store.as_generic();
        let mut used_names: HashMap<String, u32> = HashMap::new();

        for attachment in &attachments {
            let (downloaded_path, preferred_name) = match attachment {
                Attachment::Url(url) => {
                    let (path, _len) = store.put_from_url(url).await?;
                    (path, None)
                }
                Attachment::Complex(complex) => {
                    let (path, _len) = store.put_from_url(&complex.url).await?;
                    (path, complex.name.clone().filter(|n| !n.is_empty()))
                }
            };

            let final_path = if let Some(name) = preferred_name {
                let sanitized = sanitize_for_path(&name);
                let unique = deduplicate_name(&sanitized, &mut used_names);
                let target = Path::from(unique);
                if target != downloaded_path {
                    generic_store.rename(&downloaded_path, &target).await?;
                }
                target
            } else {
                let name_str = downloaded_path.to_string();
                deduplicate_name(&name_str, &mut used_names);
                downloaded_path
            };

            paths.push(FlowPath {
                path: final_path.to_string(),
                store_ref: cache_path.clone(),
                cache_store_ref: None,
            });
        }

        context.set_pin_value("paths", json!(&paths)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}

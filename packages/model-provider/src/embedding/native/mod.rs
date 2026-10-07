//! Native ONNX embedding adapters with stable preprocessing for existing model Bits.
//!
//! The legacy preprocessing contract follows FastEmbed 5.17.2 and its contributors,
//! licensed under Apache-2.0: https://github.com/Anush008/fastembed-rs.
//! Flow-Like's adapted implementation uses direct ORT sessions and streaming batches.

mod external;
mod image;
mod output;
mod text;
mod tokenizer;

pub use external::{ExternalizedModel, externalize_oversized_model};
pub use image::{ImagePreprocessor, NativeImageEmbedding};
pub use output::{Pooling, normalize};
pub use text::{NativeTextEmbedding, NativeTextPreprocessor, TokenizedBatch};
pub use tokenizer::{TokenizerFiles, load_tokenizer};

use anyhow::{Context, Result};
use ort::session::{Session, builder::GraphOptimizationLevel};
use std::path::Path;

#[derive(Debug, Clone, Default)]
pub struct SessionOptions {
    pub intra_threads: Option<usize>,
}

impl SessionOptions {
    fn builder(&self) -> Result<ort::session::builder::SessionBuilder> {
        let threads = self
            .intra_threads
            .map(Ok)
            .unwrap_or_else(|| std::thread::available_parallelism().map(usize::from))?;
        anyhow::ensure!(threads > 0, "intra_threads must be greater than zero");
        Ok(crate::ml::ort_runtime::configured_session_builder()?
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(|error| anyhow::anyhow!(error.to_string()))?
            .with_intra_threads(threads)
            .map_err(|error| anyhow::anyhow!(error.to_string()))?)
    }

    pub fn load_file(&self, path: impl AsRef<Path>) -> Result<Session> {
        if path.as_ref().metadata()?.len() > i32::MAX as u64 {
            let bundle = externalize_oversized_model(path.as_ref())?;
            // Session commit eagerly resolves external tensors. ORT owns tensor buffers
            // and any live file mappings after the temporary directory is removed.
            return self
                .builder()?
                .commit_from_file(bundle.model_path())
                .with_context(|| {
                    format!(
                        "Could not load externalized ONNX graph {}",
                        path.as_ref().display()
                    )
                });
        }
        // ORT resolves external tensor files relative to the graph's directory.
        self.builder()?
            .commit_from_file(path.as_ref())
            .with_context(|| format!("Could not load ONNX graph {}", path.as_ref().display()))
    }

    pub fn load_memory(&self, graph: &[u8]) -> Result<Session> {
        Ok(self.builder()?.commit_from_memory(graph)?)
    }

    pub fn load_memory_with_external(
        &self,
        graph: &[u8],
        external: impl IntoIterator<Item = (String, Vec<u8>)>,
    ) -> Result<Session> {
        let mut builder = self.builder()?;
        for (name, bytes) in external {
            builder = builder
                .with_external_initializer_file_in_memory(name, bytes.into())
                .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        }
        Ok(builder.commit_from_memory(graph)?)
    }
}

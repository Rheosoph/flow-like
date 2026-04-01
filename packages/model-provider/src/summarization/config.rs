use flow_like_types::json::{Deserialize, Serialize};
use schemars::JsonSchema;

/// Primary summarization strategy controlling how chunks are processed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
pub enum SummarizationStrategy {
    /// Chunks processed in parallel, then merged in a reduce pass.
    /// Best for: speed, large documents, uniform importance.
    /// Trade-off: loses cross-chunk context during map phase.
    MapReduce,
    /// Chunks processed sequentially; each step extends a rolling summary.
    /// Best for: narrative coherence, chronological documents.
    /// Trade-off: strictly sequential (no parallelism), later chunks may be under-represented.
    #[default]
    Refine,
    /// Exploits document structure (headings, sections) to build a summary tree.
    /// Best for: well-structured reports, technical documents.
    /// Trade-off: requires detectable structure; deep hierarchies multiply LLM calls.
    Hierarchical,
    /// Map-Reduce for initial summaries + Refine for coherent final output.
    /// Best for: large documents where both speed and coherence matter.
    /// Trade-off: more LLM calls than either strategy alone.
    Hybrid,
    /// Fixed memory buffer updated after each chunk, preventing context pressure.
    /// Best for: very long documents, streaming/real-time summarization.
    /// Trade-off: aggressive compression may lose details from early chunks.
    SlidingWindow,
}

impl SummarizationStrategy {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::MapReduce => "MapReduce",
            Self::Refine => "Refine",
            Self::Hierarchical => "Hierarchical",
            Self::Hybrid => "Hybrid",
            Self::SlidingWindow => "SlidingWindow",
        }
    }

    pub fn all_values() -> Vec<String> {
        vec![
            "MapReduce".to_string(),
            "Refine".to_string(),
            "Hierarchical".to_string(),
            "Hybrid".to_string(),
            "SlidingWindow".to_string(),
        ]
    }
}

impl TryFrom<&str> for SummarizationStrategy {
    type Error = flow_like_types::Error;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        match s {
            "MapReduce" => Ok(Self::MapReduce),
            "Refine" => Ok(Self::Refine),
            "Hierarchical" => Ok(Self::Hierarchical),
            "Hybrid" => Ok(Self::Hybrid),
            "SlidingWindow" => Ok(Self::SlidingWindow),
            _ => Err(flow_like_types::anyhow!("Unknown strategy: {}", s)),
        }
    }
}

/// Post-processing densification applied to the final summary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
pub enum DensificationStrategy {
    #[default]
    None,
    /// Chain of Density: iteratively revises summary to increase entity density
    /// while keeping length constant. Produces human-preferred information density.
    ChainOfDensity,
}

impl DensificationStrategy {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::None => "None",
            Self::ChainOfDensity => "ChainOfDensity",
        }
    }

    pub fn all_values() -> Vec<String> {
        vec!["None".to_string(), "ChainOfDensity".to_string()]
    }
}

impl TryFrom<&str> for DensificationStrategy {
    type Error = flow_like_types::Error;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        match s {
            "None" => Ok(Self::None),
            "ChainOfDensity" => Ok(Self::ChainOfDensity),
            _ => Err(flow_like_types::anyhow!(
                "Unknown densification strategy: {}",
                s
            )),
        }
    }
}

/// Chunking method for splitting input text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
pub enum ChunkingMethod {
    /// Split at fixed character intervals. Simple but may break mid-sentence.
    FixedSize,
    /// Split at markdown structural boundaries (headings, paragraphs, lists).
    /// Preserves semantic units. Recommended for most use cases.
    #[default]
    Markdown,
}

/// A chunk of text with positional metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextChunk {
    pub content: String,
    pub index: usize,
    pub metadata: Option<String>,
}

impl TextChunk {
    pub fn new(content: String, index: usize) -> Self {
        Self {
            content,
            index,
            metadata: None,
        }
    }

    pub fn with_metadata(mut self, metadata: String) -> Self {
        self.metadata = Some(metadata);
        self
    }
}

/// A section-level summary produced as a byproduct of hierarchical/multi-resolution output.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SectionSummary {
    pub title: String,
    pub summary: String,
    pub chunk_indices: Vec<usize>,
}

/// Statistics about the summarization process.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct SummarizationStats {
    pub total_chunks: usize,
    pub llm_calls: usize,
    pub strategy_used: String,
    pub densification_applied: bool,
    pub input_chars: usize,
    pub output_chars: usize,
}

/// The complete result of a summarization pipeline.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SummarizationResult {
    pub summary: String,
    pub entities: Vec<String>,
    pub section_summaries: Vec<SectionSummary>,
    pub stats: SummarizationStats,
}

/// Full configuration for a summarization pipeline run.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SummarizationConfig {
    pub strategy: SummarizationStrategy,
    pub densification: DensificationStrategy,
    pub chunking: ChunkingMethod,
    /// Maximum characters per chunk (default: 8000).
    pub chunk_size: usize,
    /// Overlap between adjacent chunks as a percentage 0-50 (default: 10).
    pub chunk_overlap_percent: u8,
    /// Safety limit on outer summarization iterations (default: 5).
    pub max_iterations: u32,
    /// Extract and track named entities across chunks for information preservation.
    pub track_entities: bool,
    /// Optional user instructions (e.g. "focus on action items", "use bullet points").
    pub instructions: String,
    /// Optional prior summary to build upon.
    pub prior_summary: String,
    /// Concurrency for parallel strategies like MapReduce (0 = all at once).
    pub concurrency: usize,
    /// Number of Chain of Density refinement steps (default: 3, max 5).
    pub density_steps: u32,
    /// Sliding window memory budget as fraction of chunk_size (default: 0.4 = 40%).
    pub memory_budget_ratio: f32,
}

impl Default for SummarizationConfig {
    fn default() -> Self {
        Self {
            strategy: SummarizationStrategy::default(),
            densification: DensificationStrategy::default(),
            chunking: ChunkingMethod::default(),
            chunk_size: 8000,
            chunk_overlap_percent: 10,
            max_iterations: 5,
            track_entities: false,
            instructions: String::new(),
            prior_summary: String::new(),
            concurrency: 4,
            density_steps: 3,
            memory_budget_ratio: 0.4,
        }
    }
}

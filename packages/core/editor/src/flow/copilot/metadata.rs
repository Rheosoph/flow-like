//! Catalog metadata and commands shared with FlowScript editing.
//!
//! The copilot service uses these same modules. Metadata-only callers do not
//! need its bot memory, tools, or database runtime.

#[path = "declarations.rs"]
mod declarations;
#[path = "provider.rs"]
mod provider;
#[path = "search.rs"]
mod search;
#[path = "types.rs"]
mod types;

pub(crate) use declarations::opened_alias_reaches_other;
pub use provider::{CatalogProvider, node_to_metadata, pin_to_metadata};
pub use search::{
    SearchQueryAnalysis, analyze_search_query, enrich_node_metadata, render_catalog_search_results,
    score_catalog_metadata, search_result_hint_lines,
};
pub use types::*;

//! Unified Copilot - AI-powered agent for both flow graph and UI generation
//!
//! This module provides a unified `UnifiedCopilot` struct that can operate in three modes:
//! - Board: Only flow graph modifications (nodes, connections, variables)
//! - Frontend: Only UI component generation (A2UI)
//! - Both: Can modify both flow graphs and UI components
//!
//! The unified copilot delegates to the existing specialized copilot implementations
//! (`Copilot` for board and `A2UICopilot` for frontend) to ensure consistent behavior
//! and avoid code duplication.

pub mod prompts;
mod types;
mod unified;

pub use types::*;
pub use unified::UnifiedCopilot;

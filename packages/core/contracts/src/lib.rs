//! Dependency-light contracts for integrating with Flow-Like Core.
//!
//! Keep this crate limited to wire and boundary types. Runtime implementations,
//! storage clients, model providers, and board mutation internals belong in the
//! parent `flow-like` crate.

pub mod copilot;

pub use copilot::{
    AgentType, ChatImage, ChatMessage, ChatRole, FlowIrCommitToken, PlanStep, PlanStepStatus,
    RunContext, StreamEvent, TemplateInfo,
};

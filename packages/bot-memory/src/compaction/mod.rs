//! Compaction: per-turn tool-result masking, triggers and cut planning, without model calls.

mod config;
mod planner;

pub use config::{CompactionConfig, history_window, usable_window};
pub use planner::{
    CompactionPlan, MaskedTool, Masking, PlannerInput, Trigger, TurnPhase, TurnPlan,
    mask_tool_results, placeholder, plan_compaction, plan_turn, trigger_for,
};
pub(crate) use planner::{close_pairs, tool_pairs};

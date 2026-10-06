//! Desktop FlowPilot entry points. Provider execution, workflow policy, and persistence live in
//! focused child modules; command reexports keep the Tauri registration paths stable.
#![allow(clippy::too_many_arguments)]

/// Avoid emitting verbose FlowPilot lifecycle traces in production. User-visible stream frames,
/// tool results, warnings, and errors use separate paths and remain available in every build.
macro_rules! flowpilot_debug_log {
    ($($arg:tt)*) => {
        #[cfg(debug_assertions)]
        {
            println!($($arg)*);
        }
    };
}
macro_rules! flowpilot_debug_trace {
    ($($arg:tt)*) => {
        #[cfg(debug_assertions)]
        {
            tracing::debug!($($arg)*);
        }
    };
}

mod agent_surface;
mod attachments;
mod backend_commands;
mod backend_types;
mod backends;
mod board_commits;
mod board_jobs;
mod catalog;
mod chat;
mod cli_auth;
mod cli_resolution;
mod client_pool;
mod delegated_completion;
mod external_chat;
mod external_continuation;
mod external_invocation;
mod external_phase;
mod external_process;
mod external_stream;
mod external_usage;
mod global_chat;
mod mcp;
mod mcp_progress;
mod model_catalog;
mod platform_bridge;
mod provider_errors;
mod runtime;
mod sdk_chat;
mod stream_events;
mod telemetry;
mod tool_policy;
pub(super) mod workflow_benchmark;
#[cfg(all(debug_assertions, desktop))]
mod workflow_benchmark_cli;
#[cfg(all(debug_assertions, desktop))]
pub(crate) use workflow_benchmark_cli::run_workflow_benchmark_cli;
mod workflow_declarations;
mod workflow_diagnostics;
mod workflow_observation;
mod workflow_preflight;
mod workflow_reporting;
mod workflow_results;
mod workflow_sdk;
mod workflow_state;

pub use workflow_benchmark::{
    __cmd__flowpilot_run_workflow_benchmark, __cmd__flowpilot_workflow_benchmark_cases,
    __cmd__flowpilot_workflow_benchmark_scorecards,
    __tauri_command_name_flowpilot_run_workflow_benchmark,
    __tauri_command_name_flowpilot_workflow_benchmark_cases,
    __tauri_command_name_flowpilot_workflow_benchmark_scorecards, flowpilot_run_workflow_benchmark,
    flowpilot_workflow_benchmark_cases, flowpilot_workflow_benchmark_scorecards,
};

pub use backend_commands::{
    __cmd__copilot_sdk_create_agent_session, __cmd__copilot_sdk_get_auth_status,
    __cmd__copilot_sdk_is_running, __cmd__copilot_sdk_list_models, __cmd__copilot_sdk_start,
    __cmd__copilot_sdk_stop, __cmd__flowpilot_agent_backend_get_auth_status,
    __cmd__flowpilot_agent_backend_is_running, __cmd__flowpilot_agent_backend_list,
    __cmd__flowpilot_agent_backend_list_models, __cmd__flowpilot_agent_backend_start,
    __cmd__flowpilot_agent_backend_status, __cmd__flowpilot_agent_backend_stop,
    __tauri_command_name_copilot_sdk_create_agent_session,
    __tauri_command_name_copilot_sdk_get_auth_status, __tauri_command_name_copilot_sdk_is_running,
    __tauri_command_name_copilot_sdk_list_models, __tauri_command_name_copilot_sdk_start,
    __tauri_command_name_copilot_sdk_stop,
    __tauri_command_name_flowpilot_agent_backend_get_auth_status,
    __tauri_command_name_flowpilot_agent_backend_is_running,
    __tauri_command_name_flowpilot_agent_backend_list,
    __tauri_command_name_flowpilot_agent_backend_list_models,
    __tauri_command_name_flowpilot_agent_backend_start,
    __tauri_command_name_flowpilot_agent_backend_status,
    __tauri_command_name_flowpilot_agent_backend_stop, SpecializedAgentType,
    copilot_sdk_create_agent_session, copilot_sdk_get_auth_status, copilot_sdk_is_running,
    copilot_sdk_list_models, copilot_sdk_start, copilot_sdk_stop,
    flowpilot_agent_backend_get_auth_status, flowpilot_agent_backend_is_running,
    flowpilot_agent_backend_list, flowpilot_agent_backend_list_models,
    flowpilot_agent_backend_start, flowpilot_agent_backend_status, flowpilot_agent_backend_stop,
};
pub use backend_types::{
    CopilotAuthStatus, CopilotModelInfo, FlowPilotAgentBackendKind, FlowPilotAgentCapabilitySet,
    FlowPilotAgentTransportKind, FlowPilotBackendStatus, ReasoningEffortOption,
};
pub use board_commits::{
    __cmd__flowpilot_apply_flow_ir_commit, __cmd__flowpilot_flow_ir_commit_disposition,
    __cmd__flowpilot_read_flow_ir_commit_board,
    __tauri_command_name_flowpilot_apply_flow_ir_commit,
    __tauri_command_name_flowpilot_flow_ir_commit_disposition,
    __tauri_command_name_flowpilot_read_flow_ir_commit_board, ApplyFlowIrCommitResult,
    FlowIrCommitDisposition, FlowIrCommitDispositionResult, flowpilot_apply_flow_ir_commit,
    flowpilot_flow_ir_commit_disposition, flowpilot_read_flow_ir_commit_board,
};
pub(crate) use board_jobs::ensure_board_mutation_not_reserved_by_flowpilot;
pub use board_jobs::{
    __cmd__flowpilot_ack_board_edit_job_delivery, __cmd__flowpilot_claim_board_edit_job_delivery,
    __cmd__flowpilot_create_board_edit_job, __cmd__flowpilot_get_board_edit_job,
    __cmd__flowpilot_list_board_edit_jobs, __cmd__flowpilot_resolve_board_edit_job,
    __tauri_command_name_flowpilot_ack_board_edit_job_delivery,
    __tauri_command_name_flowpilot_claim_board_edit_job_delivery,
    __tauri_command_name_flowpilot_create_board_edit_job,
    __tauri_command_name_flowpilot_get_board_edit_job,
    __tauri_command_name_flowpilot_list_board_edit_jobs,
    __tauri_command_name_flowpilot_resolve_board_edit_job, BoardEditJob, BoardEditJobDeliveryClaim,
    BoardEditJobPhase, BoardEditJobResolution, BoardEditJobReview,
    flowpilot_ack_board_edit_job_delivery, flowpilot_claim_board_edit_job_delivery,
    flowpilot_create_board_edit_job, flowpilot_get_board_edit_job, flowpilot_list_board_edit_jobs,
    flowpilot_resolve_board_edit_job,
};
pub use chat::{__cmd__copilot_chat, __tauri_command_name_copilot_chat, copilot_chat};
pub use global_chat::{
    __cmd__global_chat, __cmd__global_chat_clear_memory, __cmd__global_chat_delete_memory,
    __cmd__global_chat_list_memories, __cmd__global_chat_memory_status, __cmd__global_chat_resume,
    __cmd__global_chat_steer, __cmd__global_chat_take_unconsumed_steering,
    __tauri_command_name_global_chat, __tauri_command_name_global_chat_clear_memory,
    __tauri_command_name_global_chat_delete_memory, __tauri_command_name_global_chat_list_memories,
    __tauri_command_name_global_chat_memory_status, __tauri_command_name_global_chat_resume,
    __tauri_command_name_global_chat_steer,
    __tauri_command_name_global_chat_take_unconsumed_steering, GLOBAL_CHAT_CHANNEL_EVENT,
    GlobalChatResumeResult, global_chat, global_chat_clear_memory, global_chat_delete_memory,
    global_chat_list_memories, global_chat_memory_status, global_chat_resume, global_chat_steer,
    global_chat_take_unconsumed_steering,
};
pub use runtime::{
    __cmd__cancel_copilot_chat, __tauri_command_name_cancel_copilot_chat, cancel_copilot_chat,
};

#[cfg(test)]
mod tests;

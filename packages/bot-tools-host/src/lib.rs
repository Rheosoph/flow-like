//! The server side of the bot tools: the tool job wire format and [`HostEnv`], a `ToolEnv` over
//! per-app stores that the API (in process) and the executor (tool jobs) both use. Contract:
//! `todo/bots-capabilities/lanes/HOST-SERVER-TOOLS.md`.

mod env;
mod models;
mod wire;

pub use env::{AppAccess, HostEnv};
pub use models::{ModelSource, StateModels};
pub use wire::{
    IMAGES_FIELD, JobApp, JobModels, JobScope, ToolJob, ToolJobEnvelope, ToolReply, job_hash,
    outcome_output,
};

#[cfg(test)]
mod tests;

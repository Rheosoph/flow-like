//! The server side of the bot tools: the tool job wire format and [`HostEnv`], a `ToolEnv` over
//! per-app stores that the API (in process) and the executor (tool jobs) both use. Contract:
//! `todo/bots-capabilities/lanes/HOST-SERVER-TOOLS.md`.

mod env;
mod files;
mod models;
mod wire;

pub use env::{AppAccess, HostEnv, ImageSource, SecretSource};
pub use files::{ConversationFiles, LinkedFiles, read_object};
pub use models::{ModelSource, StateModels, VisionRouted};
pub use wire::{
    IMAGES_FIELD, JobApp, JobConnector, JobFile, JobImage, JobModels, JobScope, JobSecrets,
    SkillCallJob, ToolJob, ToolJobEnvelope, ToolReply, connector_source, job_hash, outcome_output,
};

#[cfg(test)]
mod tests;

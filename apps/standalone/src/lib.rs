#![recursion_limit = "256"]

pub mod acme;
pub mod archives;
pub mod broker;
pub mod certificate_inventory;
pub mod certificate_issuers;
pub mod certificate_requests;
pub mod certificates;
pub mod config;
pub mod crypto;
pub mod enrollment;
pub mod fleet;
pub mod host;
#[cfg(unix)]
pub mod ipc;
pub mod isolation;
pub mod management;
pub(crate) mod operational;
pub mod outbox;
pub mod placement_data;
pub mod project_artifacts;
pub mod release;
pub mod rollout;
pub mod secrets;
pub mod service;
pub mod state;
pub mod supervisor;
pub mod telemetry;
pub mod telemetry_groups;
pub mod transport;
pub mod usage;
pub mod vault;

#[cfg(feature = "runtime")]
pub mod dependencies;
#[cfg(feature = "runtime")]
pub mod deployment;
#[cfg(feature = "runtime")]
pub mod environment;
#[cfg(feature = "runtime")]
pub mod hosting;
#[cfg(feature = "runtime")]
pub mod online;
#[cfg(feature = "runtime")]
pub mod runtime;

pub mod replicas;

#[cfg(all(feature = "runtime", unix))]
pub mod service_tls;

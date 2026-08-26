#![recursion_limit = "256"]

pub mod a2ui;
#[cfg(feature = "flow")]
pub mod copilot;
pub mod credentials;
#[cfg(feature = "flow")]
pub mod flow;
pub mod protobuf;
pub mod state;
pub mod utils;

#[cfg(feature = "app")]
pub mod app;
#[cfg(feature = "bit")]
pub mod bit;
#[cfg(feature = "hub")]
pub mod hub;
#[cfg(feature = "model")]
pub mod models;
#[cfg(feature = "hub")]
pub mod profile;

#[cfg(feature = "schema-gen")]
pub mod schema_gen;

pub use flow_like_core_contracts;
pub use flow_like_model_provider;
pub use flow_like_storage;
pub use flow_like_types;
pub use num_cpus;

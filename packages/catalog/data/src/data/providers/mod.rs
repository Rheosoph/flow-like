//! Cloud-provider credential nodes.
//!
//! Each cloud (AWS, Azure, GCP, Cloudflare) has one provider node that produces
//! a schema-typed credential struct. Consumer nodes accept that struct as input
//! and rely on the provider's helper methods to build SDK clients — so
//! credential-construction logic lives in one place per cloud.
//!
//! Auth-mode dropdown + diff-only `on_update` pin reconciliation is used across
//! all providers so users only see the pins relevant to the currently-selected
//! auth mode.

pub mod aws;
pub mod azure;
pub mod cloudflare;
pub mod gcp;
pub mod util;

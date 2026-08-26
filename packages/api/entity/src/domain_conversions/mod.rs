//! Optional conversions between generated database entities and Flow-Like domain models.
//!
//! Keeping these implementations in the entity crate satisfies Rust's orphan rules while the
//! feature gate lets schema-only consumers avoid the full domain dependency graph.

mod app;
mod bit;
mod profile;

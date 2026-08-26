//! SeaORM entities generated for the API database schema.
//!
//! The generated module remains in its codegen-managed location. This stable wrapper keeps
//! hand-written extensions out of that directory so regeneration cannot discard them.

#[path = "../../src/entity/mod.rs"]
mod generated;

pub use generated::*;

#[cfg(feature = "domain-conversions")]
mod domain_conversions;

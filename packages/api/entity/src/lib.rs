//! SeaORM entities generated for the API database schema.
//!
//! The generated module remains in its codegen-managed location. This stable wrapper keeps
//! hand-written extensions out of that directory so regeneration cannot discard them.

/// `#[sea_orm::model]` derives `find_by_*`/`filter_by_*`/`delete_by_*` helpers named after the
/// database's unique indices, which Prisma writes in camelCase.
#[allow(non_snake_case)]
/// `DeriveActiveEnum` expands `Ok(Self::Error)` inside its `impl TryFrom<&str>`, which collides
/// with that impl's own `type Error` for any enum carrying an `Error` variant (`ExecutionStatus`).
#[allow(ambiguous_associated_items)]
#[path = "../../src/entity/mod.rs"]
mod generated;

pub use generated::*;

mod relations;

#[cfg(feature = "domain-conversions")]
mod domain_conversions;

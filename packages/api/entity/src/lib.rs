//! SeaORM entities generated for the API database schema.
//!
//! `src/generated/` is owned by `sea-orm-cli` (`packages/api/docker-compose.yml`). This stable
//! wrapper keeps hand-written extensions out of that directory so regeneration cannot discard
//! them.

/// `#[sea_orm::model]` derives `find_by_*`/`filter_by_*`/`delete_by_*` helpers named after the
/// database's unique indices, which Prisma writes in camelCase.
#[allow(non_snake_case)]
/// `DeriveActiveEnum` expands `Ok(Self::Error)` inside its `impl TryFrom<&str>`, which collides
/// with that impl's own `type Error` for any enum carrying an `Error` variant (`ExecutionStatus`).
#[allow(ambiguous_associated_items)]
mod generated;

pub use generated::*;

pub mod caller_apps;
pub mod json_types;
mod relations;
pub mod sea_orm_active_enums;

#[cfg(feature = "domain-conversions")]
mod domain_conversions;

//! Azure data-plane access shared by every Azure-hosted Flow-Like process.
//!
//! Both clients are Entra-only: Cosmos DB is reached through its stable REST
//! contract with AAD tokens, PostgreSQL through a managed-identity token used
//! as the connection password. Neither accepts keys, connection strings, or
//! SAS material.

pub mod cosmos;
pub mod postgres;

//! AWS data-plane access shared by every AWS-hosted Flow-Like process.
//!
//! Aurora DSQL is reached with IAM authentication only: a SigV4-presigned
//! token becomes the PostgreSQL password of each connection, and the process
//! mints a fresh one on demand before the previous expires. No static
//! database credential is accepted anywhere.

pub mod dsql;
pub mod lambda;

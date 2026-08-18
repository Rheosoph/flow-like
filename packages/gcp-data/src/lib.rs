//! GCP data-plane access shared by every GCP-hosted Flow-Like process.
//!
//! Both clients are keyless: Firestore is reached through its stable REST v1
//! contract with a metadata-server access token, PostgreSQL through a Cloud SQL
//! IAM token used as the connection password. Neither accepts a service-account
//! key file, a connection string, or an emulator endpoint — under this port no
//! long-lived database or storage credential exists to be stolen, and the
//! credential that does exist is minted per scope and expires within the hour.

pub mod firestore;
pub mod metadata;
pub mod postgres;

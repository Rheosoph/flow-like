//! The query service runs every data read away from the API: the API authorizes, mints
//! least-privilege credentials and sends one signed request; the service opens the sources,
//! enforces its limits and streams rows back as Arrow.

pub mod client;
pub mod frames;
pub mod protocol;
#[cfg(feature = "server")]
pub mod server;

pub use client::{QueryClient, QueryError, QueryTransport, RequestSigner};
pub use frames::QueryResponse;
pub use protocol::*;

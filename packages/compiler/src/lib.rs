//! Flow-Like WASM Compiler
//!
//! Compilation worker that processes WASM package compilation jobs.
//! Receives jobs from a queue (SQS, Redis, etc.), compiles raw `.wasm`
//! to pre-compiled `.cwasm` for target platforms, and reports back via JWT-signed callbacks.
//!
//! Follows the same pattern as `flow-like-executor`:
//! - Shared library crate with all logic
//! - Thin deployment wrappers in `apps/backend/`
//!
//! ## Usage
//!
//! ```rust,ignore
//! use flow_like_compiler::{compiler_router, CompilerState};
//!
//! let state = CompilerState::from_env();
//! let app = compiler_router(state);
//! ```

pub mod compile;
pub mod config;
pub mod error;
pub mod jwt;
pub mod router;

pub use compile::compile;
pub use config::CompilerConfig;
pub use error::CompilerError;
pub use flow_like_types::dispatch::{CompilationJob, CompilationResult, CompilationStatus};
pub use router::{compiler_router, process_job, CompilerState};

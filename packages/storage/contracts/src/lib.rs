//! Backend-neutral storage contracts.
//!
//! This crate intentionally excludes Lance, DataFusion, object-store clients, and
//! database implementations so consumers that only exchange graph/vector types do
//! not pay for the storage runtime dependency graph.

#[cfg(feature = "graph")]
pub mod graph;
#[cfg(feature = "vector")]
pub mod vector;

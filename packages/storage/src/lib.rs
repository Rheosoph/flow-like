#[cfg(feature = "database-runtime")]
pub mod android_store;
#[cfg(feature = "database-runtime")]
pub mod arrow_utils;
#[cfg(any(feature = "query-parser", feature = "database-runtime"))]
pub mod databases;
#[cfg(feature = "files")]
pub use flow_like_storage_files as files;
#[cfg(feature = "database-runtime")]
pub mod lancedb_write_options;

#[cfg(feature = "database-runtime")]
pub use arrow;
#[cfg(feature = "database-runtime")]
pub use arrow_array;
#[cfg(feature = "database-runtime")]
pub use arrow_schema;
#[cfg(feature = "database-runtime")]
pub use datafusion;
#[cfg(feature = "files")]
pub use flow_like_storage_files::Path;
#[cfg(feature = "files")]
pub use flow_like_storage_files::blake3;
#[cfg(feature = "files")]
pub use flow_like_storage_files::object_store;
#[cfg(feature = "database-runtime")]
pub use lance;
#[cfg(feature = "database-runtime")]
pub use lance_file;
#[cfg(feature = "database-runtime")]
pub use lance_io;
#[cfg(feature = "database-runtime")]
pub use lancedb;
#[cfg(feature = "database-runtime")]
pub use num_cpus;
#[cfg(feature = "database-runtime")]
pub use serde_arrow;

// Re-export data lake formats
#[cfg(feature = "delta")]
pub use deltalake;

#[cfg(feature = "iceberg")]
pub use iceberg;

#[cfg(feature = "iceberg")]
pub use iceberg_datafusion;

// Federation support for query push-down to remote databases
#[cfg(feature = "federation")]
pub use datafusion_federation;

// Graph query engine
#[cfg(feature = "graph")]
pub use lance_graph;

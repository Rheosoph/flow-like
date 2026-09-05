//! Database-engine portability layer.
//!
//! The API and its workers run on PostgreSQL, CockroachDB and Amazon Aurora
//! DSQL through one sea-orm connection. The three differ in exactly the places
//! this crate owns: how a serialization conflict is reported and retried,
//! which isolation levels exist, and how many rows one transaction may touch.
//! Everything else stays engine-agnostic by going through here.

pub mod batch;
pub mod conflict;
pub mod dialect;
pub mod pool;
pub mod retry;

pub use batch::{
    BatchOutcome, DEFAULT_WRITE_CHUNK, delete_in_batches, delete_in_batches_by_tuple,
    insert_chunked_in_txn, insert_in_chunks, update_in_batches,
};
pub use conflict::{AsDbConflict, DbConflict, classify_commit_err, classify_db_err};
pub use dialect::{DIALECT_ENV, DSQL_MAX_ROWS_PER_TRANSACTION, DbDialect};
pub use retry::{RetryPolicy, TransactionBody, retry_transaction};
